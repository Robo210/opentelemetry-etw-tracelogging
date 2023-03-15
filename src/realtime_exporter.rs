use crate::constants::*;
use crate::etw_exporter::*;
use opentelemetry::{trace::TraceResult, Context};
use opentelemetry_sdk::{export::trace::SpanData, trace::SpanProcessor};
use std::{fmt::Debug, pin::Pin, sync::Mutex};
use tracelogging_dynamic::*;

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
    fn on_start(&self, span: &mut opentelemetry_sdk::trace::Span, _cx: &Context) {
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
