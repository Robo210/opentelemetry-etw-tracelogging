#![allow(unused_imports, unused_mut, unused_variables)]

use crate::{exporter_traits::*, common::{json, activities::*, EtwSpan, *}};
use chrono::{Datelike, Timelike};
use opentelemetry::Array;
use opentelemetry::{
    trace::{Event, Link, SpanContext, SpanId, SpanKind, Status, TraceError},
    Key, Value,
};
use opentelemetry_api::logs::{AnyValue, LogError, LogRecord};
use opentelemetry_sdk::export::{trace::{SpanData, self}, logs::{LogData, self}};
use std::borrow::Cow;
use std::cell::RefCell;
use std::io::{Cursor, Write};
use std::mem::MaybeUninit;
use std::sync::Arc;
use std::{pin::Pin, time::SystemTime};
use tracelogging_dynamic::*;

thread_local! {static EBW: RefCell<EtwEventBuilderWrapper> = RefCell::new(EtwEventBuilderWrapper::new());}

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

    fn add_attributes_to_event<'a, C>(
        &mut self,
        attribs: C,
        use_byte_for_bools: bool,
    ) where C: Iterator<Item = (&'a Key, &'a Value)> {
        for attrib in attribs {
            let field_name = attrib.0.as_str();
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

    // LogRecord's attributes are of type (Key, AnyValue) while a SpanData's are (Key, Value),
    // so we have to duplicate entirely too much code just to compensate for this... choice.
    fn add_log_attributes_to_event<'a, C>(
        &mut self,
        attribs: &mut C,
        use_byte_for_bools: bool,
    ) where C: Iterator<Item = (&'a Key, &'a AnyValue)> {
        for attrib in attribs {
            let field_name = attrib.0.as_str();
            match attrib.1 {
                AnyValue::Boolean(b) => {
                    if use_byte_for_bools {
                        self.add_u8(field_name, *b as u8, OutType::Boolean, 0);
                    } else {
                        self.add_bool32(field_name, *b as i32, OutType::Boolean, 0);
                    }
                }
                AnyValue::Int(i) => {
                    self.add_i64(field_name, *i, OutType::Signed, 0);
                }
                AnyValue::Double(f) => {
                    self.add_f64(field_name, *f, OutType::Signed, 0);
                }
                AnyValue::String(s) => {
                    self.add_str8(field_name, &s.as_str(), OutType::Utf8, 0);
                }
                AnyValue::ListAny(list) => {
                    self.add_str8_sequence(
                        field_name,
                        list.iter().map(|s| s.to_string()),
                        OutType::Utf8,
                        0,
                    );
                },
                AnyValue::Bytes(bs) => {
                    self.add_u8_sequence(field_name, bs, OutType::Unsigned, 0);
                }
                AnyValue::Map(_) => {
                    // TODO
                    self.add_str8(field_name, attrib.1.to_string(), OutType::Json, 0);
                }
            }
        }
    }

    fn write_span_links<'a, C>(
        &mut self,
        tlg_provider: &Pin<&Provider>,
        level: Level,
        keywords: u64,
        activities: &Activities,
        event_name: &str,
        span_timestamp: &SystemTime,
        links: &mut C,
        use_byte_for_bools: bool,
    ) -> trace::ExportResult
    where
        C: Iterator<Item = &'a Link>
    {
        for link in links {
            self.reset(event_name, level, keywords, EVENT_TAG_IGNORE_EVENT_TIME);
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

        Ok(())
    }

    fn write_span_events<'a, C>(
        &mut self,
        tlg_provider: &Pin<&Provider>,
        level: Level,
        keywords: u64,
        activities: &Activities,
        events: &mut C,
        use_byte_for_bools: bool,
        export_payload_as_json: bool,
    ) -> trace::ExportResult
    where
        C: Iterator<Item = &'a Event>
    {
        for event in events {
            self.reset(&event.name, level, keywords, EVENT_TAG_IGNORE_EVENT_TIME);
            self.opcode(Opcode::Info);

            self.add_filetime(
                "otel_event_time",
                win_filetime_from_systemtime!(event.timestamp),
                OutType::DateTimeUtc,
                FIELD_TAG_IS_REAL_EVENT_TIME,
            );
            self.add_win32_systemtime("time", &event.timestamp.into(), 0);

            self.add_str8("SpanId", &activities.span_id, OutType::Utf8, 0);

            if activities.parent_span_id[0] != 0 {
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

        Ok(())
    }

    fn write_log_event<'a, C>(
        &mut self,
        tlg_provider: &Pin<&Provider>,
        event_name: &str,
        level: Level,
        keywords: u64,
        activities: &Activities,
        log_record: &LogRecord,
        attributes: C,
        use_byte_for_bools: bool,
        export_payload_as_json: bool,
    ) -> logs::ExportResult
    where
        C: Iterator<Item = (&'a Key, &'a AnyValue)> + Clone,
    {
        self.reset(&event_name, level, keywords, EVENT_TAG_IGNORE_EVENT_TIME);
        self.opcode(Opcode::Info);

        let observed_timestamp = log_record.observed_timestamp.unwrap_or_else(|| SystemTime::UNIX_EPOCH);
        let timestamp = log_record.timestamp.unwrap_or_else(|| observed_timestamp);

        if timestamp != SystemTime::UNIX_EPOCH {
            self.add_filetime(
                "otel_event_time",
                win_filetime_from_systemtime!(timestamp),
                OutType::DateTimeUtc,
                FIELD_TAG_IS_REAL_EVENT_TIME,
            );
            self.add_win32_systemtime("time", &timestamp.into(), 0);
        }

        if observed_timestamp != SystemTime::UNIX_EPOCH {
            self.add_win32_systemtime("observed time", &observed_timestamp.into(), 0);
        }

        if activities.span_id[0] != 0 {
            self.add_str8("SpanId", &activities.span_id, OutType::Utf8, 0);
        }

        if activities.parent_span_id[0] != 0 {
            self.add_str8("ParentId", &activities.parent_span_id, OutType::Utf8, 0);
        }

        if activities.trace_id_name[0] != 0 {
            self.add_str8("TraceId", &activities.trace_id_name, OutType::Utf8, 0);
        }

        if let Some(ref body) = log_record.body {
            self.add_str8("Body", body.to_string(), OutType::Utf8, 0);
        }

        let mut added = false;

        #[cfg(feature = "json")]
        if export_payload_as_json {
            let json_string = json::get_log_attributes_as_json(
                &mut attributes.clone(),
            );
            self.add_str8("Payload", &json_string, OutType::Json, 0);
            added = true;
        }

        if !added {
            self.add_log_attributes_to_event(
                &mut attributes.clone(),
                use_byte_for_bools,
            );
        }

        let activity_id = if activities.span_id[0] != 0 {
            Some(Guid::from_bytes_be(&activities.activity_id))
        } else {
            None
        };

        let win32err = self.write(
            tlg_provider,
            activity_id.as_ref(),
            activities
                .parent_activity_id
                .as_ref()
                .and_then(|g| Some(Guid::from_bytes_be(g)))
                .as_ref(),
        );

        if win32err != 0 {
            return Err(LogError::ExportFailed(Box::new(Win32Error { win32err })));
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn write_span_event<'a, C>(
        &mut self,
        tlg_provider: &Pin<&tracelogging_dynamic::Provider>,
        name: &str,
        level: Level,
        keywords: u64,
        activities: &Activities,
        event_time: &SystemTime,
        span_kind: Option<&SpanKind>,
        status: &Status,
        attributes: C,
        is_start: bool,
        add_tags: bool,
        use_byte_for_bools: bool,
        export_payload_as_json: bool,
    ) -> trace::ExportResult
    where
        C: Iterator<Item = (&'a Key, &'a Value)> + Clone
    {
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
            let json_string = json::get_attributes_as_json(attributes.clone());
            self.add_str8("Payload", &json_string, OutType::Json, 0);
            added = true;
        }

        if !added {
            self.add_attributes_to_event(attributes.clone(), use_byte_for_bools);
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
    ) -> trace::ExportResult
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

    fn write_common_schema_log_event<'a, C>(
        &mut self,
        tlg_provider: &Pin<&Provider>,
        event_name: &str,
        level: Level,
        keywords: u64,
        log_record: &LogRecord,
        export_payload_as_json: bool,
        attributes: C,
    ) -> logs::ExportResult
    where
        C: Iterator<Item = (&'a Key, &'a AnyValue)> + ExactSizeIterator + Clone,
    {
        // Avoid allocations for these fixed-length strings

        let trace_id = if let Some(ref trace_context) = log_record.trace_context {
            unsafe {
                let mut trace_id = MaybeUninit::<[u8; 32]>::uninit();
                let mut cur = Cursor::new((&mut *trace_id.as_mut_ptr()).as_mut_slice());
                write!(&mut cur, "{:32x}", trace_context.trace_id).expect("!write");
                trace_id.assume_init()
            }
        } else {
            unsafe {
                core::mem::zeroed()
            }
        };

        let span_id = if let Some(ref trace_context) = log_record.trace_context {
            unsafe {
                let mut span_id = MaybeUninit::<[u8; 16]>::uninit();
                let mut cur = Cursor::new((&mut *span_id.as_mut_ptr()).as_mut_slice());
                write!(&mut cur, "{:16x}", trace_context.span_id).expect("!write");
                span_id.assume_init()
            }
        } else {
            unsafe {
                core::mem::zeroed()
            }
        };

        let event_tags: u32 = 0; // TODO
        self.reset(event_name, level, keywords, event_tags);
        self.opcode(Opcode::Info);

        // Promoting values from PartC to PartA extensions is apparently just a draft spec
        // and not necessary / supported by consumers.
        // let exts = json::extract_common_schema_parta_exts(attributes);

        self.add_u16("__csver__", 0x0401, OutType::Signed, 0);
        self.add_struct("PartA", 2 /* + exts.len() as u8*/, 0);
        {
            let observed_timestamp = log_record.observed_timestamp.unwrap_or_else(|| SystemTime::UNIX_EPOCH);
            let timestamp = log_record.timestamp.unwrap_or_else(|| observed_timestamp);

            let time: String = chrono::DateTime::to_rfc3339(
                &chrono::DateTime::<chrono::Utc>::from(timestamp),
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

        let mut partb_field_count = 2u8;

        let severity_number = if let Some(sev) = log_record.severity_number {
            partb_field_count += 1;
            sev as u8
        } else {
            0u8
        };

        let severity_text =  if let Some(ref txt) = log_record.severity_text {
            partb_field_count += 1;
            txt.clone()
        } else {
            Cow::default()
        };

        let mut has_timestamp = false;
        let timestamp = if let Some(time) = log_record.timestamp {
            partb_field_count += 1;
            has_timestamp = true;
            time
        } else if let Some(obv_time) = log_record.observed_timestamp {
            partb_field_count += 1;
            has_timestamp = true;
            obv_time
        } else {
            SystemTime::UNIX_EPOCH // The ETW event will have a timestamp on the header
        };

        self.add_struct("PartB", partb_field_count, 0);
        {
            self.add_str8("_typeName", "Log", OutType::Utf8, 0);
            self.add_str8("name", event_name, OutType::Utf8, 0);

            if has_timestamp { // We don't check for UNIX_EPOCH just in case that is the officially recorded timestamp
                self.add_str8(
                    "eventTime",
                    &chrono::DateTime::to_rfc3339(&chrono::DateTime::<chrono::Utc>::from(
                        timestamp,
                    )),
                    OutType::Utf8,
                    0,
                );
            }

            if severity_number > 0 {
                self.add_u8("severityNumber", severity_number, OutType::Unsigned, 0);
            }

            if !severity_text.is_empty() {
                self.add_str8("severityText", severity_text.as_ref(), OutType::Utf8, 0);
            }
        }

        let partc_field_count = if export_payload_as_json {
            1u8
        } else {
            attributes.len() as u8
        };

        self.add_struct("PartC", partc_field_count, 0);
        {
            let mut added = false;

            #[cfg(feature = "json")]
            if export_payload_as_json {
                let json_string = json::get_log_attributes_as_json(&mut attributes.clone());
                self.add_str8("Payload", &json_string, OutType::Json, 0);
                added = true;
            }

            if !added {
                self.add_log_attributes_to_event(&mut attributes.clone(), true);
            }
        }

        let win32err = self.write(tlg_provider, None, None);

        if win32err != 0 {
            return Err(LogError::ExportFailed(Box::new(Win32Error { win32err })));
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

pub(crate) struct EtwEventExporter<C: KeywordLevelProvider> {
    provider: Pin<Arc<Provider>>,
    exporter_config: ExporterConfig<C>,
    bool_representation: InType,
}

impl<C: KeywordLevelProvider> EtwEventExporter<C> {
    #[allow(dead_code)]
    pub(crate) fn new(
        provider: Pin<Arc<Provider>>,
        exporter_config: ExporterConfig<C>,
        bool_representation: InType,
    ) -> Self {
        // Unfortunately we can't safely share a cached EventBuilder without adding undesirable locking
        EtwEventExporter {
            provider,
            exporter_config,
            bool_representation,
        }
    }
}

impl<C: KeywordLevelProvider> EventExporter for EtwEventExporter<C> {
    fn enabled(&self, level: u8, keyword: u64) -> bool {
        self.provider.enabled(level.into(), keyword)
    }

    // Called by the real-time exporter when a span is started
    fn log_span_start<S>(&self, span: &S) -> trace::ExportResult
    where
        S: opentelemetry_api::trace::Span + EtwSpan,
    {
        if !self.exporter_config.get_export_etw_activity_events() {
            // Common schema events are logged at span end
            return Ok(());
        }

        let span_keywords = self.exporter_config.get_span_keywords();
        let span_level = self.exporter_config.get_span_level().into();

        if !self.provider.enabled(span_level, span_keywords) {
            return Ok(());
        }

        let use_byte_for_bools = match self.bool_representation {
            InType::U8 => true,
            InType::Bool32 => false,
            _ => panic!("unsupported bool representation"),
        };
        let export_payload_as_json = self.exporter_config.get_export_as_json();

        let span_context = opentelemetry_api::trace::Span::span_context(span);

        let span_data = span.get_span_data();

        let activities = Activities::generate(
            &span_context.span_id(),
            &span_data.parent_span_id,
            &span_context.trace_id(),
        );

        EBW.with(|ebw| {
            let mut ebw = ebw.borrow_mut();

            ebw.write_span_event(
                &self.provider.as_ref(),
                &span_data.name,
                span_level,
                span_keywords,
                &activities,
                &span_data.start_time,
                Some(&span_data.span_kind),
                &Status::Unset,
                std::iter::empty(),
                true,
                false,
                use_byte_for_bools,
                export_payload_as_json,
            )?;

            let links_keywords = self.exporter_config.get_span_links_keywords();
            let links_level = self.exporter_config.get_span_links_level().into();

            if self.provider.enabled(links_level, links_keywords) {
                ebw.write_span_links(
                    &self.provider.as_ref(),
                    links_level,
                    links_keywords,
                    &activities,
                    &span_data.name,
                    &span_data.start_time,
                    &mut span_data.links.iter(),
                    use_byte_for_bools,
                )?;
            }

            Ok(())
        })
    }

    // Called by the real-time exporter when a span is ended
    fn log_span_end<S>(&self, span: &S) -> trace::ExportResult
    where
        S: opentelemetry_api::trace::Span + EtwSpan,
    {
        let span_keywords = self.exporter_config.get_span_keywords();
        let span_level = self.exporter_config.get_span_level().into();

        let use_byte_for_bools = match self.bool_representation {
            InType::U8 => true,
            InType::Bool32 => false,
            _ => panic!("unsupported bool representation"),
        };
        let export_payload_as_json = self.exporter_config.get_export_as_json();

        let span_data = span.get_span_data();

        EBW.with(|ebw| {
            let mut ebw = ebw.borrow_mut();

            if self.provider.enabled(span_level, span_keywords)
                && self.exporter_config.get_export_etw_activity_events()
            {
                let activities = Activities::generate(
                    &span_data.span_context.span_id(),
                    &span_data.parent_span_id,
                    &span_data.span_context.trace_id(),
                );

                ebw.write_span_event(
                    &self.provider.as_ref(),
                    &span_data.name,
                    span_level,
                    span_keywords,
                    &activities,
                    &span_data.end_time,
                    Some(&span_data.span_kind),
                    &span_data.status,
                    span_data.attributes.iter(),
                    false,
                    false,
                    use_byte_for_bools,
                    export_payload_as_json,
                )?;
            }

            if self.provider.enabled(span_level, span_keywords)
                && self.exporter_config.get_export_common_schema_events()
            {
                let attributes = span_data.resource.iter().chain(span_data.attributes.iter());
                ebw.write_common_schema_span(
                    &self.provider.as_ref(),
                    &span_data.name,
                    span_level,
                    span_keywords,
                    span_data,
                    span.span_context(),
                    export_payload_as_json,
                    attributes,
                )?;
            }

            Ok(())
        })
    }

    // Called by the real-time exporter when an event is added to a span
    fn log_span_event<S>(&self, event: opentelemetry_api::trace::Event, span: &S) -> trace::ExportResult
    where
        S: opentelemetry_api::trace::Span + EtwSpan,
    {
        let event_keywords = self.exporter_config.get_span_event_keywords();
        let event_level = self.exporter_config.get_span_event_level().into();

        if !self.provider.enabled(event_level, event_keywords)
            || !self.exporter_config.get_export_etw_activity_events()
        {
            // TODO: Common Schema PartB SpanEvent events
            return Ok(());
        }

        let use_byte_for_bools = match self.bool_representation {
            InType::U8 => true,
            InType::Bool32 => false,
            _ => panic!("unsupported bool representation"),
        };
        let export_payload_as_json = self.exporter_config.get_export_as_json();
        let span_data = span.get_span_data();

        let activities = Activities::generate(
            &span_data.span_context.span_id(),
            &span_data.parent_span_id,
            &span_data.span_context.trace_id(),
        );

        EBW.with(|ebw| {
            let mut ebw = ebw.borrow_mut();

            ebw.reset(
                &event.name,
                event_level,
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
                Err(TraceError::ExportFailed(Box::new(Win32Error { win32err })))
            } else {
                Ok(())
            }
        })
    }

    // Called by the batch exporter sometime after span is completed
    fn log_span_data(&self, span_data: &SpanData) -> trace::ExportResult {
        let span_keywords = self.exporter_config.get_span_keywords();

        let use_byte_for_bools = match self.bool_representation {
            InType::U8 => true,
            InType::Bool32 => false,
            _ => panic!("unsupported bool representation"),
        };
        let export_payload_as_json = self.exporter_config.get_export_as_json();

        let level = match span_data.status {
            Status::Ok => Level::Informational,
            Status::Error { .. } => Level::Error,
            Status::Unset => Level::Verbose,
        };

        EBW.with(|ebw| {
            let mut ebw = ebw.borrow_mut();
            let mut err = Ok(());

            if self.provider.enabled(level, span_keywords)
                && self.exporter_config.get_export_etw_activity_events()
            {
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
                        std::iter::empty(),
                        true,
                        true,
                        use_byte_for_bools,
                        export_payload_as_json,
                    )
                    .and_then(|_| {
                        let event_keywords = self.exporter_config.get_span_event_keywords();
                        let event_level = self.exporter_config.get_span_event_level().into();

                        if self.provider.enabled(event_level, event_keywords) {
                            ebw.write_span_events(
                                &self.provider.as_ref(),
                                event_level,
                                event_keywords,
                                &activities,
                                &mut span_data.events.iter(),
                                use_byte_for_bools,
                                export_payload_as_json,
                            )
                        } else {
                            Ok(())
                        }
                    })
                    .and_then(|_| {
                        let links_keywords = self.exporter_config.get_span_links_keywords();
                        let links_level = self.exporter_config.get_span_links_level().into();

                        if self.provider.enabled(links_level, links_keywords) {
                            ebw.write_span_links(
                                &self.provider.as_ref(),
                                links_level,
                                links_keywords,
                                &activities,
                                &span_data.name,
                                &span_data.start_time,
                                &mut span_data.links.iter(),
                                use_byte_for_bools,
                            )
                        } else {
                            Ok(())
                        }
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
                            span_data.attributes.iter(),
                            false,
                            true,
                            use_byte_for_bools,
                            export_payload_as_json,
                        )
                    });
            }

            if self.provider.enabled(Level::Informational, span_keywords)
                && self.exporter_config.get_export_common_schema_events()
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

    fn log_log_data(&self, log_data: &LogData) -> logs::ExportResult {
        let log_keywords = self.exporter_config.get_log_event_keywords();
        let level = if let Some(lvl) = log_data.record.severity_number {
            Level::from_int(7 - ((lvl as u8 + 3) / 4))
        } else {
            Level::Informational
        };

        let use_byte_for_bools = match self.bool_representation {
            InType::U8 => true,
            InType::Bool32 => false,
            _ => panic!("unsupported bool representation"),
        };
        let export_payload_as_json = self.exporter_config.get_export_as_json();

        let log_record = &log_data.record;

        let body = log_record.body.as_ref();

        let _default = opentelemetry::OrderMap::<Key, AnyValue, std::collections::hash_map::RandomState>::default();

        let attributes = if let Some(ref attrs) = log_record.attributes {
            attrs.iter()
        } else {
            _default.iter()
        };

        let mut event_name = "Event"; // TODO
        for attrib in attributes.clone() {
            if attrib.0.as_str() == "event.name" {
                if let AnyValue::String(ref name) = attrib.1 {
                    event_name = name.as_str();
                    break;
                }
            }
        }

        EBW.with(|ebw| {
            let mut ebw = ebw.borrow_mut();
            let mut err = Ok(());

            if self.provider.enabled(level, log_keywords)
                && self.exporter_config.get_export_etw_activity_events()
            {
                let activities = if let Some(ref trace_context) = log_record.trace_context {
                    Activities::generate(
                        &trace_context.span_id,
                        &SpanId::INVALID,
                        &trace_context.trace_id,
                    )
                } else {
                    Activities::default()
                };

                err = ebw.write_log_event(
                    &self.provider.as_ref(),
                    event_name,
                    level,
                    log_keywords,
                    &activities,
                    log_record,
                    attributes.clone(),
                    use_byte_for_bools,
                    export_payload_as_json,
                );
            }

            if self.provider.enabled(Level::Informational, log_keywords)
                && self.exporter_config.get_export_common_schema_events()
            {
                let err2 = ebw.write_common_schema_log_event(
                    &self.provider.as_ref(),
                    event_name,
                    Level::Informational,
                    log_keywords,
                    log_record,
                    export_payload_as_json,
                    attributes.clone(),
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
