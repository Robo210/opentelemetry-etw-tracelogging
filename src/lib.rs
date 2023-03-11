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
use tracelogging_dynamic::*;

#[derive(Debug)]
pub struct PipelineBuilder {
    provider_name: String,
    keyword: u64,
    trace_config: Option<opentelemetry_sdk::trace::Config>,
}

pub fn new_pipeline() -> PipelineBuilder {
    PipelineBuilder::default()
}

impl Default for PipelineBuilder {
    fn default() -> Self {
        Self {
            provider_name: "TraceLogging-OpenTelemetry".to_owned(),
            keyword: 1,
            trace_config: None,
        }
    }
}

impl PipelineBuilder {
    pub fn with_name(mut self, name: &str) -> Self {
        self.provider_name = name.to_owned();
        self
    }

    pub fn with_keyword(mut self, keyword: u64) -> Self {
        self.keyword = keyword;
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
            OutType::DateTime,
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
                .register(provider_name, &Provider::options());
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

                ebw.eb.reset(&span.name, level, keyword, 0);
                ebw.eb.opcode(Opcode::Start);
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
                        ebw.eb
                            .reset(&event.name, Level::Verbose, self.event_keywords, 0);
                        ebw.eb.opcode(Opcode::Info);

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

                ebw.eb.reset(&span.name, level, keyword, 0);
                ebw.eb.opcode(Opcode::Stop);

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
