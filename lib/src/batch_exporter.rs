use crate::constants::*;
use crate::etw_exporter::*;
use futures_util::future::BoxFuture;
use opentelemetry::sdk::export::trace::{ExportResult, SpanData, SpanExporter};
use std::fmt::Debug;
use std::sync::Arc;
use tracelogging_dynamic::*;

pub struct BatchExporter {
    config: EtwExporterConfig,
    ebw: EventBuilderWrapper,
}

impl BatchExporter {
    pub(crate) fn new(
        provider_name: &str,
        use_byte_for_bools: bool,
        export_payload_as_json: bool,
        common_schema: bool,
        etw_activities: bool,
    ) -> Self {
        let provider = Arc::pin(Provider::new(
            provider_name,
            Provider::options().group_id(&GROUP_ID),
        ));
        unsafe {
            provider.as_ref().register();
        }
        BatchExporter {
            config: EtwExporterConfig {
                provider,
                span_keywords: 1,
                event_keywords: 2,
                links_keywords: 4,
                bool_intype: if use_byte_for_bools {
                    InType::U8
                } else {
                    InType::Bool32
                },
                json: export_payload_as_json,
                common_schema,
                etw_activities,
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

impl SpanExporter for BatchExporter {
    fn export(&mut self, batch: Vec<SpanData>) -> BoxFuture<'static, ExportResult> {
        for span in batch {
            let _ = self.ebw.log_span_data(&self.config, &span);
        }

        Box::pin(std::future::ready(Ok(())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(feature = "tokio"))]
    #[tokio::test]
    async fn create_batch_exporter() {
        let _ = BatchExporter::new("my_provider_name", true, true, true, true);
    }
}
