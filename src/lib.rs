use opentelemetry::{sdk::export::{
    trace::{ExportResult, SpanData, SpanExporter},
    ExportError,
}, trace::TraceError};

use futures_util::future::BoxFuture;
use opentelemetry_api::{global, trace::TracerProvider};
use std::{fmt::Debug};

/// Pipeline builder
#[derive(Debug)]
pub struct PipelineBuilder {
    provider_name: String,
    trace_config: Option<opentelemetry_sdk::trace::Config>
}

/// Create a new stdout exporter pipeline builder.
pub fn new_pipeline() -> PipelineBuilder {
    PipelineBuilder::default()
}

impl Default for PipelineBuilder {
    fn default() -> Self {
        Self {
            provider_name: "TraceLogging-OpenTelemetry".to_owned(),
            trace_config: None
        }
    }
}

impl PipelineBuilder {
    /// Specify the pretty print setting.
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

impl PipelineBuilder
{
    /// Install the stdout exporter pipeline with the recommended defaults.
    pub fn install_simple(mut self) -> opentelemetry_sdk::trace::Tracer {
        let exporter = Exporter::new(&self.provider_name);

        let mut provider_builder = opentelemetry_sdk::trace::TracerProvider::builder().with_simple_exporter(exporter);
        if let Some(config) = self.trace_config.take() {
            provider_builder = provider_builder.with_config(config);
        }

        let provider = provider_builder.build();

        let tracer = provider.versioned_tracer("opentelemetry-tracelogging", Some(env!("CARGO_PKG_VERSION")), None);
        let _ = global::set_tracer_provider(provider);

        tracer
    }
}

#[derive(Debug)]
pub struct Exporter {
    provider: std::pin::Pin<Box<tracelogging_dynamic::Provider>>
}

impl Exporter {
    pub fn new(provider_name: &str) -> Self {
        let mut provider = Box::pin(tracelogging_dynamic::Provider::new());
        unsafe {
            provider.as_mut().register(provider_name, &tracelogging_dynamic::Provider::options());
        }
        Exporter {
            provider
        }
    }
}

impl SpanExporter for Exporter
{
    fn export(&mut self, batch: Vec<SpanData>) -> BoxFuture<'static, ExportResult> {
        let mut eb = tracelogging_dynamic::EventBuilder::new();
        
        for span in batch {
            if self.provider.enabled(tracelogging_dynamic::Level::Informational, 0) {
                let attribs = format!("{:#?}", span.attributes);
                eb.reset(&span.name, tracelogging_dynamic::Level::Informational, 0, 0);
                eb.add_str8("Attributes", &attribs, tracelogging_dynamic::OutType::Json, 0);
                let win32err = eb.write(&self.provider, None, None);

                if win32err != 0 {
                    return Box::pin(std::future::ready(Err(TraceError::ExportFailed(Box::new(Error{win32err})))));
                }
            }
        }

        Box::pin(std::future::ready(Ok(())))
    }
}

#[derive(Debug)]
pub struct Error {
    win32err: u32
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}
impl std::error::Error for Error {}

impl ExportError for Error {
    fn exporter_name(&self) -> &'static str {
        "TraceLogging"
    }
}
