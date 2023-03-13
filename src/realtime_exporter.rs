use opentelemetry::{sdk::export::trace::SpanData, trace::TraceResult, Context};

use crate::constants::*;
use crate::etw_exporter::*;
use opentelemetry_api::{global, trace::TracerProvider};
use opentelemetry_sdk::trace::{Span, SpanProcessor};
use std::{fmt::Debug, pin::Pin, sync::Mutex};
use tracelogging_dynamic::*;

#[derive(Debug)]
pub struct RealtimeExporterBuilder {
    provider_name: String,
    provider_id: Guid,
    trace_config: Option<opentelemetry_sdk::trace::Config>,
}

pub fn new_realtime_exporter(name: &str) -> RealtimeExporterBuilder {
    RealtimeExporterBuilder {
        provider_name: name.to_owned(),
        provider_id: Guid::from_name(name),
        trace_config: None,
    }
}

impl RealtimeExporterBuilder {
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
        let exporter = RealtimeExporter::new(&self.provider_name);

        let mut provider_builder =
            opentelemetry_sdk::trace::TracerProvider::builder().with_span_processor(exporter);

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

pub struct RealtimeExporter {
    // Must be boxed because SpanProcessor doesn't use mutable self,
    // bug EtwExporter and EventBuilder must be mutable.
    config: Mutex<ExporterConfig>,
    ebw: Mutex<EventBuilderWrapper>,
}

impl RealtimeExporter {
    pub fn new(provider_name: &str) -> Self {
        let mut provider = Box::pin(Provider::new());
        unsafe {
            provider
                .as_mut()
                .register(provider_name, Provider::options().group_id(&GROUP_ID));
        }
        RealtimeExporter {
            config: Mutex::new(ExporterConfig {
                provider,
                span_keywords: 1,
                event_keywords: 2,
            }),
            ebw: Mutex::new(EventBuilderWrapper::new()),
        }
    }
}

impl Debug for RealtimeExporter {
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

impl SpanProcessor for RealtimeExporter {
    fn on_start(&self, span: &mut Span, _cx: &Context) {
        let mut config = self.config.lock().unwrap();
        let _ = self.ebw.lock().unwrap().log_span_start(&mut *config, span);
    }

    fn on_end(&self, span: SpanData) {
        let mut config = self.config.lock().unwrap();
        let _ = self.ebw.lock().unwrap().log_span_end(&mut *config, &span);
    }

    fn force_flush(&self) -> TraceResult<()> {
        Ok(())
    }

    fn shutdown(&mut self) -> TraceResult<()> {
        Ok(())
    }
}
