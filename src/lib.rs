use chrono::{Datelike, Timelike};
use opentelemetry::{
    sdk::export::{
        trace::{ExportResult, SpanData, SpanExporter},
        ExportError,
    },
    trace::{SpanId, TraceError},
    Key, Value,
};

use futures_util::future::BoxFuture;
use opentelemetry_api::{global, trace::TracerProvider};
use std::{fmt::Debug};

#[derive(Debug)]
pub struct PipelineBuilder {
    provider_name: String,
    trace_config: Option<opentelemetry_sdk::trace::Config>,
}

pub fn new_pipeline() -> PipelineBuilder {
    PipelineBuilder::default()
}

impl Default for PipelineBuilder {
    fn default() -> Self {
        Self {
            provider_name: "TraceLogging-OpenTelemetry".to_owned(),
            trace_config: None,
        }
    }
}

impl PipelineBuilder {
    pub fn with_name(mut self, name: &str) -> Self {
        self.provider_name = name.to_owned();
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

fn add_attributes_to_event2(
    eb: &mut tracelogging_dynamic::EventBuilder,
    attribs: &mut dyn Iterator<Item = (&Key, &Value)>,
) {
    for attrib in attribs {
        match attrib.1 {
            Value::Bool(b) => {
                eb.add_bool32(
                    &attrib.0.to_string(),
                    b.to_owned().into(),
                    tracelogging_dynamic::OutType::Boolean,
                    0,
                );
            }
            Value::I64(i) => {
                eb.add_i64(
                    &attrib.0.to_string(),
                    *i,
                    tracelogging_dynamic::OutType::Signed,
                    0,
                );
            }
            Value::F64(f) => {
                eb.add_f64(
                    &attrib.0.to_string(),
                    *f,
                    tracelogging_dynamic::OutType::Signed,
                    0,
                );
            }
            Value::String(s) => {
                eb.add_str8(
                    &attrib.0.to_string(),
                    &s.to_string(),
                    tracelogging_dynamic::OutType::String,
                    0,
                );
            }
            Value::Array(_) => {
                panic!("go away");
            }
        }
    }
}

#[derive(Debug)]
pub struct Exporter {
    provider: std::pin::Pin<Box<tracelogging_dynamic::Provider>>,
}

impl Exporter {
    pub fn new(provider_name: &str) -> Self {
        let mut provider = Box::pin(tracelogging_dynamic::Provider::new());
        unsafe {
            provider
                .as_mut()
                .register(provider_name, &tracelogging_dynamic::Provider::options());
        }
        Exporter { provider }
    }
}

impl SpanExporter for Exporter {
    fn export(&mut self, batch: Vec<SpanData>) -> BoxFuture<'static, ExportResult> {
        let mut eb = tracelogging_dynamic::EventBuilder::new();

        for span in batch {
            if self
                .provider
                .enabled(tracelogging_dynamic::Level::Informational, 0)
            {
                let activity =
                    tracelogging_dynamic::Guid::from_name(&span.span_context.span_id().to_string());
                let parent_activity = if span.parent_span_id == SpanId::INVALID {
                    None
                } else {
                    Some(tracelogging_dynamic::Guid::from_name(
                        &span.parent_span_id.to_string(),
                    ))
                };

                let start_time = chrono::DateTime::from(span.start_time);
                let start_time_data: [u16; 8] = [
                    start_time.year() as u16,
                    start_time.month() as u16,
                    0,
                    start_time.day() as u16,
                    start_time.hour() as u16,
                    start_time.minute() as u16,
                    start_time.second() as u16,
                    (start_time.nanosecond() / 1000000) as u16,
                ];

                eb.reset(&span.name, tracelogging_dynamic::Level::Informational, 0, 0);
                eb.opcode(tracelogging_dynamic::Opcode::Start);
                eb.add_systemtime(
                    "start_time",
                    &start_time_data,
                    tracelogging_dynamic::OutType::DateTime,
                    0,
                );

                add_attributes_to_event2(&mut eb, &mut span.attributes.iter());

                let mut win32err =
                    eb.write(&self.provider, Some(&activity), parent_activity.as_ref());

                if win32err != 0 {
                    return Box::pin(std::future::ready(Err(TraceError::ExportFailed(Box::new(
                        Error { _win32err: win32err },
                    )))));
                }

                for event in span.events {
                    let event_time = chrono::DateTime::from(event.timestamp);
                    let event_time_data: [u16; 8] = [
                        event_time.year() as u16,
                        event_time.month() as u16,
                        0,
                        event_time.day() as u16,
                        event_time.hour() as u16,
                        event_time.minute() as u16,
                        event_time.second() as u16,
                        (event_time.nanosecond() / 1000000) as u16,
                    ];

                    eb.reset(&event.name, tracelogging_dynamic::Level::Verbose, 0, 0);
                    eb.opcode(tracelogging_dynamic::Opcode::Info);
                    eb.add_systemtime(
                        "time",
                        &event_time_data,
                        tracelogging_dynamic::OutType::DateTime,
                        0,
                    );

                    add_attributes_to_event2(
                        &mut eb,
                        &mut event.attributes.iter().map(|kv| (&kv.key, &kv.value)),
                    );

                    win32err = eb.write(&self.provider, Some(&activity), parent_activity.as_ref());

                    if win32err != 0 {
                        return Box::pin(std::future::ready(Err(TraceError::ExportFailed(
                            Box::new(Error { _win32err: win32err }),
                        ))));
                    }
                }

                let end_time = chrono::DateTime::from(span.end_time);
                let end_time_data: [u16; 8] = [
                    end_time.year() as u16,
                    end_time.month() as u16,
                    0,
                    end_time.day() as u16,
                    end_time.hour() as u16,
                    end_time.minute() as u16,
                    end_time.second() as u16,
                    (end_time.nanosecond() / 1000000) as u16,
                ];

                eb.reset(&span.name, tracelogging_dynamic::Level::Informational, 0, 0);
                eb.opcode(tracelogging_dynamic::Opcode::Stop);
                eb.add_systemtime(
                    "end_time",
                    &end_time_data,
                    tracelogging_dynamic::OutType::DateTime,
                    0,
                );

                win32err = eb.write(&self.provider, Some(&activity), parent_activity.as_ref());

                if win32err != 0 {
                    return Box::pin(std::future::ready(Err(TraceError::ExportFailed(Box::new(
                        Error { _win32err: win32err },
                    )))));
                }
            }
        }

        Box::pin(std::future::ready(Ok(())))
    }
}

#[derive(Debug)]
pub struct Error {
    _win32err: u32,
}

impl std::fmt::Display for Error {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}
impl std::error::Error for Error {}

impl ExportError for Error {
    fn exporter_name(&self) -> &'static str {
        "TraceLogging"
    }
}
