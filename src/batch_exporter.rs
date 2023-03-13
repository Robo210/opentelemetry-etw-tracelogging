use opentelemetry::sdk::export::trace::{ExportResult, SpanData, SpanExporter};

use crate::constants::*;
use crate::etw_exporter::*;
use futures_util::future::BoxFuture;
use opentelemetry_api::{global, trace::TracerProvider};
use std::{fmt::Debug, pin::Pin};
use tracelogging_dynamic::*;

#[derive(Debug)]
pub struct BatchExporterBuilder {
    provider_name: String,
    provider_id: Guid,
    trace_config: Option<opentelemetry_sdk::trace::Config>,
}

pub fn new_batch_exporter(name: &str) -> BatchExporterBuilder {
    BatchExporterBuilder {
        provider_name: name.to_owned(),
        provider_id: Guid::from_name(name),
        trace_config: None,
    }
}

impl BatchExporterBuilder {
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

    pub fn install_simple(mut self) -> opentelemetry_sdk::trace::Tracer {
        let exporter = BatchExporter::new(&self.provider_name);

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

struct ExporterConfig {
    provider: Pin<Box<Provider>>,
    span_keywords: u64,
    event_keywords: u64,
}

pub struct BatchExporter {
    config: ExporterConfig,
    ebw: EventBuilderWrapper,
}

impl BatchExporter {
    pub fn new(provider_name: &str) -> Self {
        let mut provider = Box::pin(Provider::new());
        unsafe {
            provider
                .as_mut()
                .register(provider_name, Provider::options().group_id(&GROUP_ID));
        }
        BatchExporter {
            config: ExporterConfig {
                provider,
                span_keywords: 1,
                event_keywords: 2,
            },
            ebw: EventBuilderWrapper::new(),
        }
    }
}

impl Debug for BatchExporter {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

impl EtwExporter for ExporterConfig {
    fn get_provider(&mut self) -> Pin<&mut Provider> {
        self.provider.as_mut()
    }

    fn get_span_keywords(&self) -> u64 {
        self.span_keywords
    }

    fn get_event_keywords(&self) -> u64 {
        self.event_keywords
    }
}

impl SpanExporter for BatchExporter {
    fn export(&mut self, batch: Vec<SpanData>) -> BoxFuture<'static, ExportResult> {
        for span in batch {
            self.ebw.log_spandata(&mut self.config, &span);
        }

        Box::pin(std::future::ready(Ok(())))
    }
}
