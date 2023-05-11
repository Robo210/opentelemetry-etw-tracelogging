use crate::builder::ProviderGroup;
#[allow(unused_imports)]
use crate::etw::*;
use crate::exporter_traits::*;
#[allow(unused_imports)]
use crate::user_events;
#[allow(unused_imports)]
use crate::user_events::*;
use futures_util::future::BoxFuture;
use opentelemetry::sdk::export::trace::{ExportResult, SpanData, SpanExporter};
use std::fmt::Debug;
use std::sync::Arc;

pub(crate) struct BatchExporter<E: EventExporter + Send + Sync> {
    ebw: E,
}

#[cfg(all(target_os = "windows"))]
impl<C: KeywordLevelProvider> BatchExporter<EtwEventExporter<C>> {
    pub(crate) fn new(
        provider_name: &str,
        provider_group: ProviderGroup,
        use_byte_for_bools: bool,
        exporter_config: ExporterConfig<C>,
    ) -> Self {
        let mut options = tracelogging_dynamic::Provider::options();
        if let ProviderGroup::Windows(guid) = provider_group {
            options = *options.group_id(&guid);
        }

        let provider = Arc::pin(tracelogging_dynamic::Provider::new(provider_name, &options));
        unsafe {
            provider.as_ref().register();
        }
        BatchExporter {
            ebw: EtwEventExporter::new(
                provider,
                exporter_config,
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
impl<C: KeywordLevelProvider> BatchExporter<UserEventsExporter<C>> {
    pub(crate) fn new(
        provider_name: &str,
        provider_group: ProviderGroup,
        _use_byte_for_bools: bool,
        exporter_config: ExporterConfig<C>,
    ) -> Self {
        let mut options = eventheader_dynamic::Provider::new_options();
        if let ProviderGroup::Linux(ref name) = provider_group {
            options = *options.group_name(&name);
        }
        let mut provider = eventheader_dynamic::Provider::new(provider_name, &options);
        user_events::register_eventsets(&mut provider, &exporter_config);

        BatchExporter {
            ebw: UserEventsExporter::new(Arc::new(provider), exporter_config),
        }
    }
}

impl<E: EventExporter + Send + Sync> Debug for BatchExporter<E> {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

impl<E: EventExporter + Send + Sync> SpanExporter for BatchExporter<E> {
    fn export(&mut self, batch: Vec<SpanData>) -> BoxFuture<'static, ExportResult> {
        for span in batch {
            let _ = self.ebw.log_span_data(&span);
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
            ProviderGroup::Unset,
            true,
            ExporterConfig::<DefaultKeywordLevelProvider> {
                kwl: DefaultKeywordLevelProvider,
                json: false,
                common_schema: true,
                etw_activities: true,
            },
        );
    }
}
