use crate::constants::*;
use crate::error::*;
use chrono::{Datelike, Timelike};
use futures_util::future::BoxFuture;
use opentelemetry::trace::Event;
use opentelemetry::trace::TraceId;
use opentelemetry::Array;
use opentelemetry::{
    trace::{SpanId, SpanKind, Status, TraceError},
    Key, Value,
};
use opentelemetry_sdk::export::trace::{ExportResult, SpanData};
use std::{pin::Pin, time::SystemTime};
use tracelogging_dynamic::*;

pub trait EtwExporter {
    fn get_provider(&mut self) -> Pin<&mut Provider>;
    fn get_span_keywords(&self) -> u64;
    fn get_event_keywords(&self) -> u64;
    fn get_bool_representation(&self) -> InType;
    fn get_export_as_json(&self) -> bool;
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

pub struct EventBuilderWrapper {
    eb: EventBuilder,
}

impl EventBuilderWrapper {
    pub fn new() -> EventBuilderWrapper {
        EventBuilderWrapper {
            eb: EventBuilder::new(),
        }
    }
}

struct Activities {
    span_id: String,
    activity_id: Guid,
    parent_activity_id: Option<Guid>,
    parent_span_id: String,
    trace_id_name: String,
}

fn get_activities(span_id: &SpanId, parent_span_id: &SpanId, trace_id: &TraceId) -> Activities {
    let name = span_id.to_string();
    let activity_id = Guid::from_name(&name);
    let (parent_activity_id, parent_span_name) = if *parent_span_id == SpanId::INVALID {
        (None, String::default())
    } else {
        let parent_span_name = parent_span_id.to_string();
        (Some(Guid::from_name(&parent_span_name)), parent_span_name)
    };

    Activities {
        span_id: name,
        activity_id,
        parent_activity_id,
        parent_span_id: parent_span_name,
        trace_id_name: trace_id.to_string(),
    }
}

impl EventBuilderWrapper {
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

    #[cfg(feature = "json")]
    fn add_attributes_to_event_as_json(
        &mut self,
        attribs: &mut dyn Iterator<Item = (&Key, &Value)>,
    ) {
        let mut payload: std::collections::BTreeMap<String, serde_json::Value> = Default::default();

        for attrib in attribs {
            let field_name = &attrib.0.to_string();
            match attrib.1 {
                Value::Bool(b) => {
                    payload.insert(field_name.clone(), serde_json::Value::Bool(*b));
                }
                Value::I64(i) => {
                    payload.insert(field_name.clone(), serde_json::Value::Number(serde_json::Number::from(*i)));
                }
                Value::F64(f) => {
                    payload.insert(field_name.clone(), serde_json::Value::Number(serde_json::Number::from_f64(*f).unwrap()));
                }
                Value::String(s) => {
                    payload.insert(field_name.clone(), serde_json::Value::String(s.to_string()));
                }
                Value::Array(array) => match array {
                    Array::Bool(v) => {
                        payload.insert(field_name.clone(), serde_json::Value::Array(v.iter().map(|b| serde_json::Value::Bool(*b)).collect()));
                    }
                    Array::I64(v) => {
                        payload.insert(field_name.clone(), serde_json::Value::Array(v.iter().map(|i| serde_json::Value::Number(serde_json::Number::from(*i))).collect()));
                    }
                    Array::F64(v) => {
                        payload.insert(field_name.clone(), serde_json::Value::Array(v.iter().map(|f| serde_json::Value::Number(serde_json::Number::from_f64(*f).unwrap())).collect()));
                    }
                    Array::String(v) => {
                        payload.insert(field_name.clone(), serde_json::Value::Array(v.iter().map(|s| serde_json::Value::String(s.to_string())).collect()));
                    }
                },
            }
        }

        let json_string = serde_json::to_string(&payload);
        if json_string.is_ok() {
            self.add_str8("Payload", &json_string.unwrap(), OutType::Json, 0);
        }
    }

    fn write_events(
        &mut self,
        tlg_provider: &Pin<&mut Provider>,
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
                    self.add_attributes_to_event_as_json(&mut event.attributes.iter().map(|kv| (&kv.key, &kv.value)));
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
                    Some(&activities.activity_id),
                    activities.parent_activity_id.as_ref(),
                );

                if win32err != 0 {
                    return Err(TraceError::ExportFailed(Box::new(Error { win32err })));
                }
            }
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn write_span_event(
        &mut self,
        tlg_provider: &Pin<&mut Provider>,
        name: &str,
        level: Level,
        keywords: u64,
        activities: &Activities,
        event_time: SystemTime,
        span_kind: Option<&SpanKind>,
        status: &Status,
        attributes: &mut dyn Iterator<Item = (&Key, &Value)>,
        is_start: bool,
        add_tags: bool,
        use_byte_for_bools: bool,
        export_payload_as_json: bool
    ) -> ExportResult {
        let (event_tags, field_tags) = if add_tags {
            (EVENT_TAG_IGNORE_EVENT_TIME, FIELD_TAG_IS_REAL_EVENT_TIME)
        } else {
            (0, 0)
        };
        let (opcode, event_name) = if is_start {
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
        self.add_win32_systemtime(event_name, &event_time.into(), 0);

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
            self.add_attributes_to_event_as_json(attributes);
            added = true;
        }

        if !added {
            self.add_attributes_to_event(attributes, use_byte_for_bools);
        }

        let win32err = self.write(
            tlg_provider,
            Some(&activities.activity_id),
            activities.parent_activity_id.as_ref(),
        );

        if win32err != 0 {
            return Err(TraceError::ExportFailed(Box::new(Error { win32err })));
        }

        Ok(())
    }

    pub fn log_span_start(
        &mut self,
        provider: &mut dyn EtwExporter,
        span: &opentelemetry::sdk::trace::Span,
    ) -> ExportResult {
        let span_keywords = provider.get_span_keywords();
        let use_byte_for_bools = match provider.get_bool_representation() {
            InType::U8 => true,
            InType::Bool32 => false,
            _ => panic!("unsupported bool reprsentation"),
        };
        let export_payload_as_json = provider.get_export_as_json();
        let (level, keyword) = (Level::Informational, span_keywords);
        let tlg_provider = provider.get_provider();

        if tlg_provider.enabled(level, keyword) {
            let span_context = opentelemetry_api::trace::Span::span_context(span);

            let span_data = span.exported_data(); // Would be nice if span.data was public
            let (name, start_time, parent_activity_id, kind, add_tags) = match span_data {
                Some(data) => (
                    data.name.to_string(),
                    data.start_time,
                    data.parent_span_id,
                    Some(data.span_kind),
                    false,
                ),
                None => (
                    span_context.span_id().to_string(),
                    SystemTime::now(),
                    SpanId::INVALID,
                    None,
                    true,
                ),
            };

            let activities = get_activities(
                &span_context.span_id(),
                &parent_activity_id,
                &span_context.trace_id(),
            );

            self.write_span_event(
                &tlg_provider,
                &name,
                level,
                keyword,
                &activities,
                start_time,
                kind.as_ref(),
                &Status::Unset,
                &mut std::iter::empty(),
                true,
                add_tags,
                use_byte_for_bools,
                export_payload_as_json,
            )?;
        }

        Ok(())
    }

    pub fn log_span_end(
        &mut self,
        provider: &mut dyn EtwExporter,
        span_data: &SpanData,
    ) -> ExportResult {
        let span_keywords = provider.get_span_keywords();
        let event_keywords = provider.get_event_keywords();
        let use_byte_for_bools = match provider.get_bool_representation() {
            InType::U8 => true,
            InType::Bool32 => false,
            _ => panic!("unsupported bool reprsentation"),
        };
        let export_payload_as_json = provider.get_export_as_json();
        let (level, keyword) = (Level::Informational, span_keywords);
        let tlg_provider = provider.get_provider();

        if tlg_provider.enabled(level, keyword) {
            let activities = get_activities(
                &span_data.span_context.span_id(),
                &span_data.parent_span_id,
                &span_data.span_context.trace_id(),
            );

            self.write_events(
                &tlg_provider,
                Level::Verbose,
                event_keywords,
                &activities,
                &mut span_data.events.iter(),
                use_byte_for_bools,
                export_payload_as_json,
            )?;

            self.write_span_event(
                &tlg_provider,
                &span_data.name,
                level,
                keyword,
                &activities,
                span_data.end_time,
                Some(&span_data.span_kind),
                &span_data.status,
                &mut span_data.attributes.iter(),
                false,
                false,
                use_byte_for_bools,
                export_payload_as_json,
            )?;
        }

        Ok(())
    }

    pub fn log_spandata(
        &mut self,
        provider: &mut dyn EtwExporter,
        span: &SpanData,
    ) -> BoxFuture<'static, ExportResult> {
        let span_keywords = provider.get_span_keywords();
        let event_keywords = provider.get_event_keywords();
        let use_byte_for_bools = match provider.get_bool_representation() {
            InType::U8 => true,
            InType::Bool32 => false,
            _ => panic!("unsupported bool reprsentation"),
        };
        let export_payload_as_json = provider.get_export_as_json();
        let tlg_provider = provider.get_provider();

        let (level, keyword) = match span.status {
            Status::Ok => (Level::Informational, span_keywords),
            Status::Error { .. } => (Level::Error, span_keywords),
            Status::Unset => (Level::Verbose, span_keywords),
        };

        let activities = get_activities(
            &span.span_context.span_id(),
            &span.parent_span_id,
            &span.span_context.trace_id(),
        );

        if tlg_provider.enabled(level, keyword) {
            let mut err = self.write_span_event(
                &tlg_provider,
                &span.name,
                level,
                keyword,
                &activities,
                span.start_time,
                Some(&span.span_kind),
                &span.status,
                &mut std::iter::empty(),
                true,
                true,
                use_byte_for_bools,
                export_payload_as_json,
            );
            if err.is_err() {
                return Box::pin(std::future::ready(err));
            }

            err = self.write_events(
                &tlg_provider,
                Level::Verbose,
                event_keywords,
                &activities,
                &mut span.events.iter(),
                use_byte_for_bools,
                export_payload_as_json,
            );
            if err.is_err() {
                return Box::pin(std::future::ready(err));
            }

            err = self.write_span_event(
                &tlg_provider,
                &span.name,
                level,
                keyword,
                &activities,
                span.end_time,
                Some(&span.span_kind),
                &span.status,
                &mut span.attributes.iter(),
                false,
                true,
                use_byte_for_bools,
                export_payload_as_json,
            );
            if err.is_err() {
                return Box::pin(std::future::ready(err));
            }
        }

        Box::pin(std::future::ready(Ok(())))
    }
}

impl std::ops::Deref for EventBuilderWrapper {
    type Target = EventBuilder;
    fn deref(&self) -> &Self::Target {
        &self.eb
    }
}

impl std::ops::DerefMut for EventBuilderWrapper {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.eb
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
        let mut ebw = EventBuilderWrapper::new();

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
        let mut ebw = EventBuilderWrapper::new();

        let attribs = vec![
            TEST_KEY_STR.array(vec![StringValue::from("is cool")]),
            TEST_KEY_BOOL.array(vec![false, true, false]),
            TEST_KEY_INT.array(vec![5, 6, 7]),
            TEST_KEY_FLOAT.array(vec![7.1, 0.9, -1.3]),
        ];

        ebw.add_attributes_to_event(&mut attribs.iter().map(|kv| (&kv.key, &kv.value)), true);
    }
}
