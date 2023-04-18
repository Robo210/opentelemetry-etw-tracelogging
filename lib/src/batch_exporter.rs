use crate::constants::*;
#[allow(unused_imports)]
use crate::etw_exporter::*;
use crate::exporter_traits::*;
#[allow(unused_imports)]
use crate::user_events_exporter::*;
use futures_util::future::BoxFuture;
use opentelemetry::sdk::export::trace::{ExportResult, SpanData, SpanExporter};
use std::fmt::Debug;
use std::sync::Arc;

pub struct BatchExporter<C: ExporterConfig + Send + Sync, E: EventExporter + Send + Sync> {
    config: C,
    ebw: E,
}

#[cfg(all(target_os = "windows"))]
impl<C: ExporterConfig + Send + Sync> BatchExporter<C, EtwEventExporter> {
    pub(crate) fn new(provider_name: &str, use_byte_for_bools: bool, exporter_config: C) -> Self {
        let provider = Arc::pin(tracelogging_dynamic::Provider::new(
            provider_name,
            tracelogging_dynamic::Provider::options().group_id(&GROUP_ID),
        ));
        unsafe {
            provider.as_ref().register();
        }
        BatchExporter {
            config: exporter_config,
            ebw: EtwEventExporter::new(
                provider,
                if use_byte_for_bools {
                    tracelogging::InType::U8
                } else {
                    tracelogging::InType::Bool32
                },
            ),
        }
    }
}

#[cfg(all(target_os = "linux"))]
impl<C: ExporterConfig + Send + Sync> BatchExporter<C, UserEventsExporter> {
    pub(crate) fn new(provider_name: &str, _use_byte_for_bools: bool, exporter_config: C) -> Self {
        let mut provider = linux_tld::Provider::new(
            provider_name,
            linux_tld::Provider::options().group_name(GROUP_NAME),
        );

        // Standard real-time level/keyword pairs
        provider.register_set(linux_tlg::Level::Informational, 1);
        provider.register_set(linux_tlg::Level::Verbose, 2);
        provider.register_set(linux_tlg::Level::Verbose, 4);

        // Common Schema events use a level based on a span's Status
        provider.register_set(linux_tlg::Level::Error, 1);
        provider.register_set(linux_tlg::Level::Verbose, 1);

        let exporter = BatchExporter {
            config: exporter_config,
            ebw: UserEventsExporter::new(Arc::new(provider)),
        };

        exporter
    }
}

impl<C: ExporterConfig + Send + Sync, E: EventExporter + Send + Sync> Debug
    for BatchExporter<C, E>
{
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

impl<C: ExporterConfig + Send + Sync, E: EventExporter + Send + Sync> SpanExporter
    for BatchExporter<C, E>
{
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

    #[test]
    fn create_batch_exporter() {
        let _ = BatchExporter::new(
            "my_provider_name",
            true,
            DefaultExporterConfig {
                common_schema: true,
                etw_activities: true,
            },
        );
    }
}
