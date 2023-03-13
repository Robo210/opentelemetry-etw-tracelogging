//! # ETW Span Exporter
//!
//! The ETW [`SpanExporter`] logs spans as ETW events.
//! Spans are logged as activity start and stop events,
//! using auto-generated activity IDs.
//! Events in a span are logged as ETW events using the
//! span's activity ID.
//! 
//! This crate is a no-op when running on Linux.
//!
//! The ETW provider ID is generated from a hash of the
//! specified provider name.
//!
//! The ETW provider is joined to the group
//! `{e60ec51a-8e54-5a4f-2fb260a4f9213b3a}`. Events in this
//! group should be interpreted according to the event and
//! field tags on each event.
//!
//! By default, span start and stop events are logged with
//! keyword 1 and [`Level::Informational`]. Events attached
//! to the span are logged with keyword 2 and ['Level::Verbose`].
//!
//! # ETW Timestamps
//!
//! Spans are exported asynchronously and in batches.
//! Because of this, the timestamps on the ETW events
//! do not represent the time the span was originally
//! started or ended.
//!
//! When an ETW event has the EVENT_TAG_IGNORE_EVENT_TIME tag,
//! the timestamp on the EVENT_RECORD should be ignored when
//! processing the event. To get the real time of the event,
//! look for a field tagged with FIELD_TAG_IS_REAL_EVENT_TIME.
//!
//! # Examples
//!
//! ```no_run
//! use opentelemetry_api::global::shutdown_tracer_provider;
//! use opentelemetry_api::trace::Tracer;
//!
//! let tracer = opentelemetry_etw_tracelogging::new_pipeline("MyEtwProviderName")
//!     .install_simple();
//!
//! tracer.in_span("doing_work", |cx| {
//!     // Traced app logic here...
//! });
//!
//! shutdown_tracer_provider(); // sending remaining spans
//! ```
use chrono::{Datelike, Timelike};
use opentelemetry::{
    sdk::export::{
        trace::{ExportResult, SpanData, SpanExporter},
        ExportError,
    },
    trace::{SpanId, SpanKind, Status, TraceError},
    Key, Value,
};

use futures_util::future::BoxFuture;
use opentelemetry_api::{global, trace::TracerProvider};
use std::fmt::Debug;
use tracelogging::filetime_from_systemtime;
use tracelogging_dynamic::*;

/// {e60ec51a-8e54-5a4f-2fb260a4f9213b3a}
/// Events in this group were (re)logged from OpenTelemetry.
/// Use the event tags and field tags to properly interpret these events.
pub const GROUP_ID: Guid = Guid::from_fields(
    0xe60ec51a,
    0x8e54,
    0x5a4f,
    [0x2f, 0xb2, 0x60, 0xa4, 0xf9, 0x21, 0x3b, 0x3a],
);
/// The ETW event's timestamp is not meaningful.
/// Use the field tags to find the timestamp value to use.
pub const EVENT_TAG_IGNORE_EVENT_TIME: u32 = 12345;
/// This field contains the actual timestamp of the event.
pub const FIELD_TAG_IS_REAL_EVENT_TIME: u32 = 98765;

#[derive(Debug)]
pub struct PipelineBuilder {
    provider_name: String,
    provider_id: Guid,
    trace_config: Option<opentelemetry_sdk::trace::Config>,
}

pub fn new_pipeline(name: &str) -> PipelineBuilder {
    PipelineBuilder {
        provider_name: name.to_owned(),
        provider_id: Guid::from_name(name),
        trace_config: None,
    }
}

impl PipelineBuilder {
    /// For advanced scenarios.
    /// Assign a provider ID to the ETW provider rather than use
    /// one generated from the provider name.
    pub fn with_provider_id(mut self, guid: &Guid) -> Self {
        self.provider_id = guid.to_owned();
        self
    }

    /// Assign the SDK trace configuration.
    pub fn with_trace_config(mut self, config: opentelemetry_sdk::trace::Config) -> Self {
        self.trace_config = Some(config);
        self
    }
}

impl PipelineBuilder {
    pub fn install_simple(mut self) -> opentelemetry_sdk::trace::Tracer {
        let exporter = Exporter::new(&self.provider_name);

        let mut provider_builder =
            opentelemetry_sdk::trace::TracerProvider::builder().with_simple_exporter(exporter);

        if let Some(config) = self.trace_config.take() {
            provider_builder = provider_builder.with_config(config);
        }

        let provider = provider_builder.build();

        let tracer = provider.versioned_tracer(
            "opentelemetry-tracelogging",
            Some(env!("CARGO_PKG_VERSION")),
            None,
        );
        let _ = global::set_tracer_provider(provider);

        tracer
    }
}

fn add_attributes_to_event(
    eb: &mut EventBuilder,
    attribs: &mut dyn Iterator<Item = (&Key, &Value)>,
) {
    for attrib in attribs {
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
                eb.add_i64(&attrib.0.to_string(), *i, OutType::Signed, 0);
            }
            Value::F64(f) => {
                eb.add_f64(&attrib.0.to_string(), *f, OutType::Signed, 0);
            }
            Value::String(s) => {
                eb.add_str8(&attrib.0.to_string(), &s.to_string(), OutType::Utf8, 0);
            }
            Value::Array(_) => {
                panic!("go away");
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

struct EventBuilderWrapper {
    eb: EventBuilder,
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

#[derive(Debug)]
pub struct Exporter {
    provider: std::pin::Pin<Box<Provider>>,
    span_keywords: u64,
    event_keywords: u64,
}

impl Exporter {
    pub fn new(provider_name: &str) -> Self {
        let mut provider = Box::pin(Provider::new());
        unsafe {
            provider
                .as_mut()
                .register(provider_name, Provider::options().group_id(&GROUP_ID));
        }
        Exporter {
            provider,
            span_keywords: 1,
            event_keywords: 2,
        }
    }
}

impl SpanExporter for Exporter {
    fn export(&mut self, batch: Vec<SpanData>) -> BoxFuture<'static, ExportResult> {
        let mut ebw = EventBuilderWrapper {
            eb: EventBuilder::new(),
        };

        for span in batch {
            // We probably want to allow more fine-grained control over the keywords in the future
            let (level, keyword) = match span.status {
                Status::Ok => (Level::Informational, self.span_keywords),
                Status::Error { .. } => (Level::Error, self.span_keywords),
                Status::Unset => (Level::Verbose, self.span_keywords),
            };

            if self.provider.enabled(level, keyword) {
                let activity = Guid::from_name(&span.span_context.span_id().to_string());
                let parent_activity = if span.parent_span_id == SpanId::INVALID {
                    None
                } else {
                    Some(Guid::from_name(&span.parent_span_id.to_string()))
                };

                ebw.eb
                    .reset(&span.name, level, keyword, EVENT_TAG_IGNORE_EVENT_TIME);
                ebw.eb.opcode(Opcode::Start);

                ebw.eb.add_filetime(
                    "otel_event_time",
                    filetime_from_systemtime!(span.start_time),
                    OutType::DateTimeUtc,
                    FIELD_TAG_IS_REAL_EVENT_TIME,
                );
                ebw.add_win32_systemtime("start_time", &span.start_time.into(), 0);

                ebw.add_string(
                    "span_kind",
                    match span.span_kind {
                        SpanKind::Client => "Client",
                        SpanKind::Server => "Server",
                        SpanKind::Producer => "Producer",
                        SpanKind::Consumer => "Consumer",
                        SpanKind::Internal => "Internal",
                    },
                    0,
                );

                if let Status::Error { description } = span.status {
                    ebw.add_string("error", description.to_string(), 0);
                };

                add_attributes_to_event(&mut ebw, &mut span.attributes.iter());

                let mut win32err =
                    ebw.eb
                        .write(&self.provider, Some(&activity), parent_activity.as_ref());

                if win32err != 0 {
                    return Box::pin(std::future::ready(Err(TraceError::ExportFailed(Box::new(
                        Error { win32err },
                    )))));
                }

                if self.provider.enabled(Level::Verbose, self.event_keywords) {
                    for event in span.events {
                        ebw.eb.reset(
                            &event.name,
                            Level::Verbose,
                            self.event_keywords,
                            EVENT_TAG_IGNORE_EVENT_TIME,
                        );
                        ebw.eb.opcode(Opcode::Info);

                        ebw.eb.add_filetime(
                            "otel_event_time",
                            filetime_from_systemtime!(event.timestamp),
                            OutType::DateTimeUtc,
                            FIELD_TAG_IS_REAL_EVENT_TIME,
                        );
                        ebw.add_win32_systemtime("time", &event.timestamp.into(), 0);

                        add_attributes_to_event(
                            &mut ebw,
                            &mut event.attributes.iter().map(|kv| (&kv.key, &kv.value)),
                        );

                        win32err =
                            ebw.eb
                                .write(&self.provider, Some(&activity), parent_activity.as_ref());

                        if win32err != 0 {
                            return Box::pin(std::future::ready(Err(TraceError::ExportFailed(
                                Box::new(Error { win32err }),
                            ))));
                        }
                    }
                }

                ebw.eb
                    .reset(&span.name, level, keyword, EVENT_TAG_IGNORE_EVENT_TIME);
                ebw.eb.opcode(Opcode::Stop);

                ebw.eb.add_filetime(
                    "otel_event_time",
                    filetime_from_systemtime!(span.end_time),
                    OutType::DateTimeUtc,
                    FIELD_TAG_IS_REAL_EVENT_TIME,
                );
                ebw.add_win32_systemtime("end_time", &span.end_time.into(), 0);

                win32err = ebw
                    .eb
                    .write(&self.provider, Some(&activity), parent_activity.as_ref());

                if win32err != 0 {
                    return Box::pin(std::future::ready(Err(TraceError::ExportFailed(Box::new(
                        Error { win32err },
                    )))));
                }
            }
        }

        Box::pin(std::future::ready(Ok(())))
    }
}

#[derive(Debug)]
pub struct Error {
    win32err: u32,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("Win32 error: {}", self.win32err))
    }
}
impl std::error::Error for Error {}

impl ExportError for Error {
    fn exporter_name(&self) -> &'static str {
        "TraceLogging"
    }
}
