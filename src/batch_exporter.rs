use crate::constants::*;
use crate::etw_exporter::*;
use futures_util::future::BoxFuture;
use opentelemetry::sdk::export::trace::{ExportResult, SpanData, SpanExporter};
use std::{fmt::Debug, pin::Pin};
use tracelogging_dynamic::*;

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

#[allow(unused_imports)]
mod tests {
    use super::*;

    #[test]
    fn create_batch_exporter() {
        let _ = BatchExporter::new("my_provider_name");
    }
}
