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
    bool_intype: InType,
    json: bool,
}

pub struct RealtimeExporter {
    // Must be boxed because SpanProcessor doesn't use mutable self,
    // but EtwExporter and EventBuilder must be mutable.
    config: Mutex<ExporterConfig>,
    ebw: Mutex<EventBuilderWrapper>,
}

impl RealtimeExporter {
    pub(crate) fn new(provider_name: &str, use_byte_for_bools: bool, export_payload_as_json: bool) -> Self {
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
                bool_intype: if use_byte_for_bools {
                    InType::U8
                } else {
                    InType::Bool32
                },
                json: export_payload_as_json,
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

    fn get_bool_representation(&self) -> InType {
        self.bool_intype
    }

    fn get_export_as_json(&self) -> bool {
        self.json
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

#[allow(unused_imports)]
mod tests {
    use super::*;

    #[test]
    fn create_realtime_exporter() {
        let _ = RealtimeExporter::new("my_provider_name", false, false);
    }
}
