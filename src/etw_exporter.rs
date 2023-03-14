use std::{pin::Pin, time::SystemTime};

use crate::constants::*;
use crate::error::*;
use chrono::{Datelike, Timelike};
use futures_util::future::BoxFuture;
use opentelemetry::Array;
use opentelemetry::trace::Event;
use opentelemetry::{
    trace::{SpanId, SpanKind, Status, TraceError},
    Key, Value,
};
use opentelemetry_sdk::export::trace::{ExportResult, SpanData};
use tracelogging_dynamic::*;

pub trait EtwExporter {
    fn get_provider(&mut self) -> Pin<&mut Provider>;
    fn get_span_keywords(&self) -> u64;
    fn get_event_keywords(&self) -> u64;
}

fn add_attributes_to_event(
    eb: &mut EventBuilder,
    attribs: &mut dyn Iterator<Item = (&Key, &Value)>,
) {
    for attrib in attribs {
        let field_name = &attrib.0.to_string();
        match attrib.1 {
            Value::Bool(b) => {
                eb.add_bool32(
                    &attrib.0.to_string(),
                    b.to_owned().into(),
                    OutType::Boolean,
                    0,
                );
            }
            Value::I64(i) => {
                eb.add_i64(field_name, *i, OutType::Signed, 0);
            }
            Value::F64(f) => {
                eb.add_f64(field_name, *f, OutType::Signed, 0);
            }
            Value::String(s) => {
                eb.add_str8(field_name, &s.to_string(), OutType::Utf8, 0);
            }
            Value::Array(array) => {
                match array {
                    Array::Bool(_v) => {
                        panic!("eb.add_bool32_sequence isn't really useable");
                    }
                    Array::I64(v) => {
                        eb.add_i64_sequence(field_name, v.iter(), OutType::Signed, 0);
                    }
                    Array::F64(v) => {
                        eb.add_f64_sequence(field_name, v.iter(), OutType::Signed, 0);
                    }
                    Array::String(v) => {
                        eb.add_str8_sequence(field_name, v.iter().map(|s| { s.to_string() }), OutType::Utf8, 0);
                    }
                }
            }
        }
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
    activity_id: Guid,
    parent_activity_id: Option<Guid>,
}

fn get_activities(name: &str, parent_span_id: &SpanId) -> Activities {
    let activity_id = Guid::from_name(name);
    let parent_activity_id = if *parent_span_id == SpanId::INVALID {
        None
    } else {
        Some(Guid::from_name(&parent_span_id.to_string()))
    };

    Activities {
        activity_id,
        parent_activity_id,
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

    fn write_events(
        &mut self,
        tlg_provider: &Pin<&mut Provider>,
        level: Level,
        keywords: u64,
        activities: &Activities,
        events: &mut dyn Iterator<Item = &Event>,
    ) -> ExportResult {
        if tlg_provider.enabled(level, keywords) {
            for event in events {
                self.eb.reset(
                    &event.name,
                    Level::Verbose,
                    keywords,
                    EVENT_TAG_IGNORE_EVENT_TIME,
                );
                self.eb.opcode(Opcode::Info);

                self.eb.add_filetime(
                    "otel_event_time",
                    win_filetime_from_systemtime!(event.timestamp),
                    OutType::DateTimeUtc,
                    FIELD_TAG_IS_REAL_EVENT_TIME,
                );
                self.add_win32_systemtime("time", &event.timestamp.into(), 0);

                add_attributes_to_event(
                    self,
                    &mut event.attributes.iter().map(|kv| (&kv.key, &kv.value)),
                );

                let win32err = self.eb.write(
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
    ) -> ExportResult {
        let (event_tags, field_tags) = if add_tags {
            (EVENT_TAG_IGNORE_EVENT_TIME, FIELD_TAG_IS_REAL_EVENT_TIME)
        } else {
            (0, 0)
        };
        let (opcode, event_name) = if is_start {
            (Opcode::Start, "start_time")
        } else {
            (Opcode::Stop, "end_time")
        };

        self.eb.reset(name, level, keywords, event_tags);
        self.eb.opcode(opcode);

        self.eb.add_filetime(
            "otel_event_time",
            win_filetime_from_systemtime!(event_time),
            OutType::DateTimeUtc,
            field_tags,
        );
        self.add_win32_systemtime(event_name, &event_time.into(), 0);

        if let Some(sk) = span_kind {
            self.add_string(
                "span_kind",
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
            self.add_string("error", description.to_string(), 0);
        };

        add_attributes_to_event(self, attributes);

        let win32err = self.eb.write(
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
        let (level, keyword) = (Level::Informational, span_keywords);
        let tlg_provider = provider.get_provider();

        if tlg_provider.enabled(level, keyword) {
            let span_context = opentelemetry_api::trace::Span::span_context(span);
            //let api_span = span as &dyn opentelemetry_api::trace::Span;
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

            let activities =
                get_activities(&span_context.span_id().to_string(), &parent_activity_id);

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
        let (level, keyword) = (Level::Informational, span_keywords);
        let tlg_provider = provider.get_provider();

        if tlg_provider.enabled(level, keyword) {
            let activities = get_activities(
                &span_data.span_context.span_id().to_string(),
                &span_data.parent_span_id,
            );

            self.write_events(
                &tlg_provider,
                Level::Verbose,
                event_keywords,
                &activities,
                &mut span_data.events.iter(),
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
        let tlg_provider = provider.get_provider();

        let (level, keyword) = match span.status {
            Status::Ok => (Level::Informational, span_keywords),
            Status::Error { .. } => (Level::Error, span_keywords),
            Status::Unset => (Level::Verbose, span_keywords),
        };

        let activities = get_activities(
            &span.span_context.span_id().to_string(),
            &span.parent_span_id,
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
