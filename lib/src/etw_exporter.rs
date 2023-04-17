use crate::constants::*;
use crate::error::*;
use crate::exporter_traits::*;
use crate::json;
use chrono::{Datelike, Timelike};
use opentelemetry::Array;
use opentelemetry::{
    trace::{Event, Link, SpanContext, SpanId, SpanKind, Status, TraceError},
    Key, Value,
};
use opentelemetry_sdk::export::trace::{ExportResult, SpanData};
use std::borrow::Borrow;
use std::borrow::Cow;
use std::cell::RefCell;
use std::io::{Cursor, Write};
use std::mem::MaybeUninit;
use std::sync::Arc;
use std::{pin::Pin, time::SystemTime};
use tracelogging_dynamic::*;

thread_local! {static EBW: RefCell<EtwEventBuilderWrapper> = RefCell::new(EtwEventBuilderWrapper::new());}

#[derive(Clone)]
pub(crate) struct EtwExporterConfig {
    pub(crate) span_keywords: u64,
    pub(crate) event_keywords: u64,
    pub(crate) links_keywords: u64,
    pub(crate) json: bool,
    pub(crate) common_schema: bool,
    pub(crate) etw_activities: bool,
}

impl ExporterConfig for EtwExporterConfig {
    fn get_span_keywords(&self) -> u64 {
        self.span_keywords
    }

    fn get_event_keywords(&self) -> u64 {
        self.event_keywords
    }

    fn get_links_keywords(&self) -> u64 {
        self.links_keywords
    }

    fn get_export_as_json(&self) -> bool {
        self.json
    }

    fn get_export_common_schema_event(&self) -> bool {
        self.common_schema
    }

    fn get_export_span_events(&self) -> bool {
        self.etw_activities
    }
}

struct Win32SystemTime {
    st: [u16; 8],
}

impl From<std::time::SystemTime> for Win32SystemTime {
    fn from(value: std::time::SystemTime) -> Self {
        let dt = chrono::DateTime::from(value);

        Win32SystemTime {
            st: [
                dt.year() as u16,
                dt.month() as u16,
                0,
                dt.day() as u16,
                dt.hour() as u16,
                dt.minute() as u16,
                dt.second() as u16,
                (dt.nanosecond() / 1000000) as u16,
            ],
        }
    }
}

struct EtwEventBuilderWrapper {
    eb: EventBuilder,
}

impl EtwEventBuilderWrapper {
    pub fn new() -> EtwEventBuilderWrapper {
        EtwEventBuilderWrapper {
            eb: EventBuilder::new(),
        }
    }

    fn add_win32_systemtime(
        &mut self,
        field_name: &str,
        win32_systemtime: &Win32SystemTime,
        field_tag: u32,
    ) -> &mut Self {
        self.eb.add_systemtime(
            field_name,
            &win32_systemtime.st,
            OutType::DateTimeUtc,
            field_tag,
        );
        self
    }

    fn add_string(
        &mut self,
        field_name: &str,
        field_value: impl AsRef<[u8]>,
        field_tag: u32,
    ) -> &mut Self {
        self.eb
            .add_str8(field_name, field_value, OutType::Utf8, field_tag);
        self
    }

    fn add_attributes_to_event(
        &mut self,
        attribs: &mut dyn Iterator<Item = (&Key, &Value)>,
        use_byte_for_bools: bool,
    ) {
        for attrib in attribs {
            let field_name = &attrib.0.to_string();
            match attrib.1 {
                Value::Bool(b) => {
                    if use_byte_for_bools {
                        self.add_u8(field_name, *b as u8, OutType::Boolean, 0);
                    } else {
                        self.add_bool32(field_name, *b as i32, OutType::Boolean, 0);
                    }
                }
                Value::I64(i) => {
                    self.add_i64(field_name, *i, OutType::Signed, 0);
                }
                Value::F64(f) => {
                    self.add_f64(field_name, *f, OutType::Signed, 0);
                }
                Value::String(s) => {
                    self.add_str8(field_name, &s.to_string(), OutType::Utf8, 0);
                }
                Value::Array(array) => match array {
                    Array::Bool(v) => {
                        if use_byte_for_bools {
                            self.add_u8_sequence(
                                field_name,
                                v.iter().map(|b| if *b { &1u8 } else { &0u8 }),
                                OutType::Boolean,
                                0,
                            );
                        } else {
                            self.add_bool32_sequence(
                                field_name,
                                v.iter().map(|b| if *b { &1i32 } else { &0i32 }),
                                OutType::Boolean,
                                0,
                            );
                        }
                    }
                    Array::I64(v) => {
                        self.add_i64_sequence(field_name, v.iter(), OutType::Signed, 0);
                    }
                    Array::F64(v) => {
                        self.add_f64_sequence(field_name, v.iter(), OutType::Signed, 0);
                    }
                    Array::String(v) => {
                        self.add_str8_sequence(
                            field_name,
                            v.iter().map(|s| s.to_string()),
                            OutType::Utf8,
                            0,
                        );
                    }
                },
            }
        }
    }

    fn write_span_links(
        &mut self,
        tlg_provider: &Pin<&Provider>,
        level: Level,
        keywords: u64,
        activities: &Activities,
        event_name: &str,
        span_timestamp: &SystemTime,
        links: &mut dyn Iterator<Item = &Link>,
        use_byte_for_bools: bool,
    ) -> ExportResult {
        if tlg_provider.enabled(level, keywords) {
            for link in links {
                self.reset(
                    event_name,
                    Level::Verbose,
                    keywords,
                    EVENT_TAG_IGNORE_EVENT_TIME,
                );
                self.opcode(Opcode::Info);

                self.add_filetime(
                    "otel_event_time",
                    win_filetime_from_systemtime!(span_timestamp),
                    OutType::DateTimeUtc,
                    FIELD_TAG_IS_REAL_EVENT_TIME,
                );
                self.add_win32_systemtime("time", &(*span_timestamp).into(), 0);

                self.add_str8(
                    "Link",
                    std::fmt::format(format_args!("{:16x}", link.span_context.span_id())),
                    OutType::Utf8,
                    0,
                );

                self.add_attributes_to_event(
                    &mut link.attributes.iter().map(|kv| (&kv.key, &kv.value)),
                    use_byte_for_bools,
                );

                let win32err = self.write(
                    tlg_provider,
                    Some(Guid::from_bytes_be(&activities.activity_id)).as_ref(),
                    activities
                        .parent_activity_id
                        .as_ref()
                        .and_then(|g| Some(Guid::from_bytes_be(g)))
                        .as_ref(),
                );

                if win32err != 0 {
                    return Err(TraceError::ExportFailed(Box::new(Win32Error { win32err })));
                }
            }
        }

        Ok(())
    }

    fn write_span_events(
        &mut self,
        tlg_provider: &Pin<&Provider>,
        level: Level,
        keywords: u64,
        activities: &Activities,
        events: &mut dyn Iterator<Item = &Event>,
        use_byte_for_bools: bool,
        export_payload_as_json: bool,
    ) -> ExportResult {
        if tlg_provider.enabled(level, keywords) {
            for event in events {
                self.reset(
                    &event.name,
                    Level::Verbose,
                    keywords,
                    EVENT_TAG_IGNORE_EVENT_TIME,
                );
                self.opcode(Opcode::Info);

                self.add_filetime(
                    "otel_event_time",
                    win_filetime_from_systemtime!(event.timestamp),
                    OutType::DateTimeUtc,
                    FIELD_TAG_IS_REAL_EVENT_TIME,
                );
                self.add_win32_systemtime("time", &event.timestamp.into(), 0);

                self.add_str8("SpanId", &activities.span_id, OutType::Utf8, 0);

                if !activities.parent_span_id.is_empty() {
                    self.add_str8("ParentId", &activities.parent_span_id, OutType::Utf8, 0);
                }

                self.add_str8("TraceId", &activities.trace_id_name, OutType::Utf8, 0);

                let mut added = false;

                #[cfg(feature = "json")]
                if export_payload_as_json {
                    let json_string = json::get_attributes_as_json(
                        &mut event.attributes.iter().map(|kv| (&kv.key, &kv.value)),
                    );
                    self.add_str8("Payload", &json_string, OutType::Json, 0);
                    added = true;
                }

                if !added {
                    self.add_attributes_to_event(
                        &mut event.attributes.iter().map(|kv| (&kv.key, &kv.value)),
                        use_byte_for_bools,
                    );
                }

                let win32err = self.write(
                    tlg_provider,
                    Some(Guid::from_bytes_be(&activities.activity_id)).as_ref(),
                    activities
                        .parent_activity_id
                        .as_ref()
                        .and_then(|g| Some(Guid::from_bytes_be(g)))
                        .as_ref(),
                );

                if win32err != 0 {
                    return Err(TraceError::ExportFailed(Box::new(Win32Error { win32err })));
                }
            }
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn write_span_event(
        &mut self,
        tlg_provider: &Pin<&tracelogging_dynamic::Provider>,
        name: &str,
        level: Level,
        keywords: u64,
        activities: &Activities,
        event_time: &SystemTime,
        span_kind: Option<&SpanKind>,
        status: &Status,
        attributes: &mut dyn Iterator<Item = (&Key, &Value)>,
        is_start: bool,
        add_tags: bool,
        use_byte_for_bools: bool,
        export_payload_as_json: bool,
    ) -> ExportResult {
        let (event_tags, field_tags) = if add_tags {
            (EVENT_TAG_IGNORE_EVENT_TIME, FIELD_TAG_IS_REAL_EVENT_TIME)
        } else {
            (0, 0)
        };
        let (opcode, time_field_name) = if is_start {
            (Opcode::Start, "StartTime")
        } else {
            (Opcode::Stop, "EndTime")
        };

        self.reset(name, level, keywords, event_tags);
        self.opcode(opcode);

        self.add_filetime(
            "otel_event_time",
            win_filetime_from_systemtime!(event_time),
            OutType::DateTimeUtc,
            field_tags,
        );
        self.add_win32_systemtime(time_field_name, &(*event_time).into(), 0);

        if let Some(sk) = span_kind {
            self.add_string(
                "Kind",
                match sk {
                    SpanKind::Client => "Client",
                    SpanKind::Server => "Server",
                    SpanKind::Producer => "Producer",
                    SpanKind::Consumer => "Consumer",
                    SpanKind::Internal => "Internal",
                },
                0,
            );
        }

        if let Status::Error { description } = &status {
            self.add_string("StatusMessage", description.to_string(), 0);
        };

        self.add_str8("SpanId", &activities.span_id, OutType::Utf8, 0);

        if !activities.parent_span_id.is_empty() {
            self.add_str8("ParentId", &activities.parent_span_id, OutType::Utf8, 0);
        }

        self.add_str8("TraceId", &activities.trace_id_name, OutType::Utf8, 0);

        let mut added = false;

        #[cfg(feature = "json")]
        if export_payload_as_json {
            let json_string = json::get_attributes_as_json(attributes);
            self.add_str8("Payload", &json_string, OutType::Json, 0);
            added = true;
        }

        if !added {
            self.add_attributes_to_event(attributes, use_byte_for_bools);
        }

        let win32err = self.write(
            tlg_provider,
            Some(Guid::from_bytes_be(&activities.activity_id)).as_ref(),
            activities
                .parent_activity_id
                .as_ref()
                .and_then(|g| Some(Guid::from_bytes_be(g)))
                .as_ref(),
        );

        if win32err != 0 {
            return Err(TraceError::ExportFailed(Box::new(Win32Error { win32err })));
        }

        Ok(())
    }

    fn write_common_schema_span<'a, C>(
        &mut self,
        tlg_provider: &Pin<&Provider>,
        name: &str,
        level: Level,
        keywords: u64,
        span_data: &SpanData,
        span_context: &SpanContext,
        export_payload_as_json: bool,
        _attributes: C,
    ) -> ExportResult
    where
        C: IntoIterator<Item = (&'a Key, &'a Value)>,
    {
        // Avoid allocations for these fixed-length strings

        let trace_id = unsafe {
            let mut trace_id = MaybeUninit::<[u8; 32]>::uninit();
            let mut cur = Cursor::new((&mut *trace_id.as_mut_ptr()).as_mut_slice());
            write!(&mut cur, "{:32x}", span_context.trace_id()).expect("!write");
            trace_id.assume_init()
        };

        let span_id = unsafe {
            let mut span_id = MaybeUninit::<[u8; 16]>::uninit();
            let mut cur = Cursor::new((&mut *span_id.as_mut_ptr()).as_mut_slice());
            write!(&mut cur, "{:16x}", span_context.span_id()).expect("!write");
            span_id.assume_init()
        };

        let event_tags: u32 = 0; // TODO
        self.reset(name, level, keywords, event_tags);
        self.opcode(Opcode::Info);

        // Promoting values from PartC to PartA extensions is apparently just a draft spec
        // and not necessary / supported by consumers.
        // let exts = json::extract_common_schema_parta_exts(attributes);

        self.add_u16("__csver__", 0x0401, OutType::Signed, 0);
        self.add_struct("PartA", 2 /* + exts.len() as u8*/, 0);
        {
            let time: String = chrono::DateTime::to_rfc3339(
                &chrono::DateTime::<chrono::Utc>::from(span_data.end_time),
            );
            self.add_str8("time", time, OutType::Utf8, 0);

            self.add_struct("ext_dt", 2, 0);
            {
                self.add_str8("traceId", &trace_id, OutType::Utf8, 0);
                self.add_str8("spanId", &span_id, OutType::Utf8, 0);
            }

            // for ext in exts {
            //     self.add_struct(ext.0, ext.1.len() as u8, 0);

            //     for field in ext.1 {
            //         self.add_str8(field.0, field.1.as_ref(), OutType::Utf8, 0);
            //     }
            // }
        }

        // if !span_data.links.is_empty() {
        //     self.add_struct("PartB", 5, 0);
        //     {
        //         self.add_str8("_typeName", "SpanLink", OutType::Utf8, 0);
        //         self.add_str8("fromTraceId", &traceId, OutType::Utf8, 0);
        //         self.add_str8("fromSpanId", &spanId, OutType::Utf8, 0);
        //         self.add_str8("toTraceId", "SpanLink", OutType::Utf8, 0);
        //         self.add_str8("toSpanId", "SpanLink", OutType::Utf8, 0);
        //     }
        // }

        let mut status_message: Cow<str> = Cow::default();
        let mut partb_field_count = 5u8;
        if span_data.parent_span_id != SpanId::INVALID {
            partb_field_count += 1;
        }
        if let Status::Error { description } = &span_data.status {
            partb_field_count += 1;
            status_message = Cow::Borrowed(description);
        }
        // TODO: azureResourceProvider: string
        if !span_data.links.is_empty() {
            partb_field_count += 1; // Type is an "array", but really it's just a string with a JSON array
        }

        self.add_struct("PartB", partb_field_count, 0);
        {
            self.add_str8("_typeName", "Span", OutType::Utf8, 0);
            if span_data.parent_span_id != SpanId::INVALID {
                self.add_str8(
                    "parentId",
                    &span_data.parent_span_id.to_string(),
                    OutType::Utf8,
                    0,
                );
            }
            self.add_str8("name", name, OutType::Utf8, 0);
            self.add_u8(
                "kind",
                match span_data.span_kind {
                    SpanKind::Internal => 0u8,
                    SpanKind::Server => 1,
                    SpanKind::Client => 2,
                    SpanKind::Producer => 3,
                    SpanKind::Consumer => 4,
                },
                OutType::Unsigned,
                0,
            );
            self.add_str8(
                "startTime",
                &chrono::DateTime::to_rfc3339(&chrono::DateTime::<chrono::Utc>::from(
                    span_data.end_time,
                )),
                OutType::Utf8,
                0,
            );
            self.add_u8(
                "success",
                match span_data.status {
                    Status::Ok => 1u8,
                    _ => 0u8,
                },
                OutType::Boolean,
                0,
            );
            if !status_message.is_empty() {
                self.add_str8("statusMessage", status_message.as_ref(), OutType::Utf8, 0);
            }
            // TODO: azureResourceProvider: string
            if !span_data.links.is_empty() {
                let mut links = String::with_capacity(2 + (78 * span_data.links.len()));
                links += "[";
                for link in span_data.links.iter() {
                    links += "{\"toTraceId\":\"";
                    links += &link.span_context.trace_id().to_string();
                    links += "\",\"toSpanId\":\"";
                    links += &link.span_context.span_id().to_string();
                    links += "\"}";
                }
                links += "]";

                self.add_str8("links", &links, OutType::Json, 0);
            }
            // TODO: promote HTTP, Database and Messaging fields
        }

        let partc_field_count = if export_payload_as_json {
            1u8
        } else {
            span_data.attributes.len() as u8
        };

        self.add_struct("PartC", partc_field_count, 0);
        {
            let mut added = false;

            #[cfg(feature = "json")]
            if export_payload_as_json {
                let json_string = json::get_attributes_as_json(&mut span_data.attributes.iter());
                self.add_str8("Payload", &json_string, OutType::Json, 0);
                added = true;
            }

            if !added {
                self.add_attributes_to_event(&mut span_data.attributes.iter(), true);
            }
        }

        let win32err = self.write(tlg_provider, None, None);

        if win32err != 0 {
            return Err(TraceError::ExportFailed(Box::new(Win32Error { win32err })));
        }

        Ok(())
    }
}

impl std::ops::Deref for EtwEventBuilderWrapper {
    type Target = EventBuilder;
    fn deref(&self) -> &Self::Target {
        &self.eb
    }
}

impl std::ops::DerefMut for EtwEventBuilderWrapper {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.eb
    }
}

pub(crate) struct EtwEventExporter {
    provider: Pin<Arc<Provider>>,
    bool_representation: InType,
}

impl EtwEventExporter {
    #[allow(dead_code)]
    pub(crate) fn new(provider: Pin<Arc<Provider>>, bool_representation: InType) -> EtwEventExporter {
        // Unfortunately we can't safely share a cached EventBuilder without adding undesirable locking
        EtwEventExporter {
            provider,
            bool_representation,
        }
    }
}

impl EventExporter for EtwEventExporter {
    fn enabled(&self, level: u8, keyword: u64) -> bool {
        self.provider.enabled(level.into(), keyword)
    }

    // Called by the real-time exporter when a span is started
    fn log_span_start<C, S>(&self, provider: &C, span: &S) -> ExportResult
    where
        C: ExporterConfig,
        S: opentelemetry_api::trace::Span + EtwSpan,
    {
        if !provider.get_export_span_events() {
            // Common schema events are logged at span end
            return Ok(());
        }

        let span_keywords = provider.get_span_keywords();

        if !self.provider.enabled(Level::Informational, span_keywords) {
            return Ok(());
        }

        let links_keywords = provider.get_links_keywords();
        let use_byte_for_bools = match self.bool_representation {
            InType::U8 => true,
            InType::Bool32 => false,
            _ => panic!("unsupported bool representation"),
        };
        let export_payload_as_json = provider.get_export_as_json();

        let span_context = opentelemetry_api::trace::Span::span_context(span);

        let span_data = span.get_span_data();

        let activities = Activities::generate(
            &span_context.span_id(),
            &span_data.parent_span_id,
            &span_context.trace_id(),
        );

        let mut ebw = EtwEventBuilderWrapper::new();

        ebw.write_span_event(
            &self.provider.as_ref(),
            &span_data.name,
            Level::Informational,
            span_keywords,
            &activities,
            &span_data.start_time,
            Some(&span_data.span_kind),
            &Status::Unset,
            &mut std::iter::empty(),
            true,
            false,
            use_byte_for_bools,
            export_payload_as_json,
        )?;

        ebw.write_span_links(
            &self.provider.as_ref(),
            Level::Verbose,
            links_keywords,
            &activities,
            &span_data.name,
            &span_data.start_time,
            &mut span_data.links.iter(),
            use_byte_for_bools,
        )?;

        Ok(())
    }

    // Called by the real-time exporter when a span is ended
    fn log_span_end<C, S>(&self, provider: &C, span: &S) -> ExportResult
    where
        C: ExporterConfig,
        S: opentelemetry_api::trace::Span + EtwSpan,
    {
        let span_keywords = provider.get_span_keywords();
        //let event_keywords = provider.get_event_keywords();
        let use_byte_for_bools = match self.bool_representation {
            InType::U8 => true,
            InType::Bool32 => false,
            _ => panic!("unsupported bool representation"),
        };
        let export_payload_as_json = provider.get_export_as_json();

        let span_data = span.get_span_data();

        let mut ebw = EtwEventBuilderWrapper::new();

        if self.provider.enabled(Level::Informational, span_keywords)
            && provider.get_export_span_events()
        {
            let activities = Activities::generate(
                &span_data.span_context.span_id(),
                &span_data.parent_span_id,
                &span_data.span_context.trace_id(),
            );

            ebw.write_span_event(
                &self.provider.as_ref(),
                &span_data.name,
                Level::Informational,
                span_keywords,
                &activities,
                &span_data.end_time,
                Some(&span_data.span_kind),
                &span_data.status,
                &mut span_data.attributes.iter(),
                false,
                false,
                use_byte_for_bools,
                export_payload_as_json,
            )?;
        }

        if self.provider.enabled(Level::Informational, span_keywords)
            && provider.get_export_common_schema_event()
        {
            let attributes = span_data.resource.iter().chain(span_data.attributes.iter());
            ebw.write_common_schema_span(
                &self.provider.as_ref(),
                &span_data.name,
                Level::Informational,
                span_keywords,
                span_data,
                span.span_context(),
                export_payload_as_json,
                attributes,
            )?;
        }

        Ok(())
    }

    // Called by the real-time exporter when an event is added to a span
    fn log_span_event<C, S>(
        &self,
        provider: &C,
        event: opentelemetry_api::trace::Event,
        span: &S,
    ) -> ExportResult
    where
        C: ExporterConfig,
        S: opentelemetry_api::trace::Span + EtwSpan,
    {
        let event_keywords = provider.get_event_keywords();

        if !self.provider.enabled(Level::Informational, event_keywords)
            || !provider.get_export_span_events()
        {
            // TODO: Common Schema PartB SpanEvent events
            return Ok(());
        }

        let use_byte_for_bools = match self.bool_representation {
            InType::U8 => true,
            InType::Bool32 => false,
            _ => panic!("unsupported bool representation"),
        };
        let export_payload_as_json = provider.get_export_as_json();
        let span_data = span.get_span_data();

        let activities = Activities::generate(
            &span_data.span_context.span_id(),
            &span_data.parent_span_id,
            &span_data.span_context.trace_id(),
        );

        let mut ebw = EtwEventBuilderWrapper::new();

        ebw.reset(
            &event.name,
            Level::Verbose,
            event_keywords,
            EVENT_TAG_IGNORE_EVENT_TIME,
        );
        ebw.opcode(Opcode::Info);

        ebw.add_filetime(
            "otel_event_time",
            win_filetime_from_systemtime!(event.timestamp),
            OutType::DateTimeUtc,
            FIELD_TAG_IS_REAL_EVENT_TIME,
        );
        ebw.add_win32_systemtime("time", &event.timestamp.into(), 0);

        ebw.add_str8("SpanId", &activities.span_id, OutType::Utf8, 0);

        if !activities.parent_span_id.is_empty() {
            ebw.add_str8("ParentId", &activities.parent_span_id, OutType::Utf8, 0);
        }

        ebw.add_str8("TraceId", &activities.trace_id_name, OutType::Utf8, 0);

        let mut added = false;

        #[cfg(feature = "json")]
        if export_payload_as_json {
            let json_string = json::get_attributes_as_json(
                &mut event.attributes.iter().map(|kv| (&kv.key, &kv.value)),
            );
            ebw.add_str8("Payload", &json_string, OutType::Json, 0);
            added = true;
        }

        if !added {
            ebw.add_attributes_to_event(
                &mut event.attributes.iter().map(|kv| (&kv.key, &kv.value)),
                use_byte_for_bools,
            );
        }

        let win32err = ebw.write(
            &self.provider,
            Some(Guid::from_bytes_be(&activities.activity_id)).as_ref(),
            activities
                .parent_activity_id
                .as_ref()
                .and_then(|g| Some(Guid::from_bytes_be(g)))
                .as_ref(),
        );

        if win32err != 0 {
            return Err(TraceError::ExportFailed(Box::new(Win32Error { win32err })));
        }

        Ok(())
    }

    // Called by the batch exporter sometime after span is completed
    fn log_span_data<C>(&self, provider: &C, span_data: &SpanData) -> ExportResult
    where
        C: ExporterConfig,
    {
        let span_keywords = provider.get_span_keywords();
        let event_keywords = provider.get_event_keywords();
        let links_keywords = provider.get_links_keywords();
        let use_byte_for_bools = match self.bool_representation {
            InType::U8 => true,
            InType::Bool32 => false,
            _ => panic!("unsupported bool representation"),
        };
        let export_payload_as_json = provider.get_export_as_json();

        let level = match span_data.status {
            Status::Ok => Level::Informational,
            Status::Error { .. } => Level::Error,
            Status::Unset => Level::Verbose,
        };

        EBW.with(|ebw| {
            let mut ebw = ebw.borrow_mut();
            let mut err = Ok(());

            if self.provider.enabled(level, span_keywords) && provider.get_export_span_events() {
                let activities = Activities::generate(
                    &span_data.span_context.span_id(),
                    &span_data.parent_span_id,
                    &span_data.span_context.trace_id(),
                );

                err = ebw
                    .write_span_event(
                        &self.provider.as_ref(),
                        &span_data.name,
                        level,
                        span_keywords,
                        &activities,
                        &span_data.start_time,
                        Some(&span_data.span_kind),
                        &span_data.status,
                        &mut std::iter::empty(),
                        true,
                        true,
                        use_byte_for_bools,
                        export_payload_as_json,
                    )
                    .and_then(|_| {
                        ebw.write_span_events(
                            &self.provider.as_ref(),
                            Level::Verbose,
                            event_keywords,
                            &activities,
                            &mut span_data.events.iter(),
                            use_byte_for_bools,
                            export_payload_as_json,
                        )
                    })
                    .and_then(|_| {
                        ebw.write_span_links(
                            &self.provider.as_ref(),
                            Level::Verbose,
                            links_keywords,
                            &activities,
                            &span_data.name,
                            &span_data.start_time,
                            &mut span_data.links.iter(),
                            use_byte_for_bools,
                        )
                    })
                    .and_then(|_| {
                        ebw.write_span_event(
                            &self.provider.as_ref(),
                            &span_data.name,
                            level,
                            span_keywords,
                            &activities,
                            &span_data.end_time,
                            Some(&span_data.span_kind),
                            &span_data.status,
                            &mut span_data.attributes.iter(),
                            false,
                            true,
                            use_byte_for_bools,
                            export_payload_as_json,
                        )
                    });
            }

            if self.provider.enabled(Level::Informational, span_keywords)
                && provider.get_export_common_schema_event()
            {
                let attributes = span_data.attributes.iter(); //.chain(span_data.resource.iter());

                let err2 = ebw.write_common_schema_span(
                    &self.provider.as_ref(),
                    &span_data.name,
                    Level::Informational,
                    span_keywords,
                    span_data,
                    &span_data.span_context,
                    export_payload_as_json,
                    attributes,
                );

                err = err.and(err2);
            }

            err
        })
    }
}

#[allow(dead_code)]
#[allow(unused_imports)]
mod tests {
    use super::*;
    use opentelemetry::{Key, StringValue};

    const TEST_KEY_STR: Key = Key::from_static_str("str");
    const TEST_KEY_BOOL: Key = Key::from_static_str("bool");
    const TEST_KEY_INT: Key = Key::from_static_str("int");
    const TEST_KEY_FLOAT: Key = Key::from_static_str("float");

    #[test]
    fn add_attributes() {
        let mut ebw = EtwEventBuilderWrapper::new();

        let attribs = vec![
            TEST_KEY_STR.string("is cool"),
            TEST_KEY_BOOL.bool(false),
            TEST_KEY_INT.i64(5),
            TEST_KEY_FLOAT.f64(7.1),
        ];

        ebw.add_attributes_to_event(&mut attribs.iter().map(|kv| (&kv.key, &kv.value)), false);
    }

    #[test]
    fn add_attribute_sequences() {
        let mut ebw = EtwEventBuilderWrapper::new();

        let attribs = vec![
            TEST_KEY_STR.array(vec![StringValue::from("is cool")]),
            TEST_KEY_BOOL.array(vec![false, true, false]),
            TEST_KEY_INT.array(vec![5, 6, 7]),
            TEST_KEY_FLOAT.array(vec![7.1, 0.9, -1.3]),
        ];

        ebw.add_attributes_to_event(&mut attribs.iter().map(|kv| (&kv.key, &kv.value)), true);
    }
}
