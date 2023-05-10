use crate::builder::ProviderGroup;
#[allow(unused_imports)]
use crate::etw_exporter::*;
use crate::exporter_traits::*;
#[allow(unused_imports)]
use crate::user_events_exporter::*;
use futures_util::future::BoxFuture;
use opentelemetry::sdk::export::trace::{ExportResult, SpanData, SpanExporter};
use std::fmt::Debug;
use std::sync::Arc;

pub(crate) struct BatchExporter<C: KeywordLevelProvider, E: EventExporter + Send + Sync> {
    config: ExporterConfig<C>,
    ebw: E,
}

#[cfg(all(target_os = "windows"))]
impl<C: KeywordLevelProvider> BatchExporter<C, EtwEventExporter> {
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
impl<C: KeywordLevelProvider> BatchExporter<C, UserEventsExporter> {
    pub(crate) fn new(
        provider_name: &str,
        provider_group: ProviderGroup,
        _use_byte_for_bools: bool,
        exporter_config: ExporterConfig<C>,
    ) -> Self {
        let mut options = linux_tld::Provider::new_options();
        if let ProviderGroup::Linux(ref name) = provider_group {
            options = *options.group_name(&name);
        }
        let mut provider = linux_tld::Provider::new(provider_name, &options);

        #[cfg(not(test))]
        {
            // Standard real-time level/keyword pairs
            provider.register_set(
                linux_tlg::Level::Informational,
                exporter_config.get_span_keywords(),
            );
            provider.register_set(
                linux_tlg::Level::Verbose,
                exporter_config.get_event_keywords(),
            );
            provider.register_set(
                linux_tlg::Level::Verbose,
                exporter_config.get_links_keywords(),
            );

            // Common Schema events use a level based on a span's Status
            provider.register_set(linux_tlg::Level::Error, exporter_config.get_span_keywords());
            provider.register_set(
                linux_tlg::Level::Verbose,
                exporter_config.get_span_keywords(),
            );
        }
        #[cfg(test)]
        {
            // Standard real-time level/keyword pairs
            provider.create_unregistered(
                true,
                linux_tlg::Level::Informational,
                exporter_config.get_span_keywords(),
            );
            provider.create_unregistered(
                true,
                linux_tlg::Level::Verbose,
                exporter_config.get_event_keywords(),
            );
            provider.create_unregistered(
                true,
                linux_tlg::Level::Verbose,
                exporter_config.get_links_keywords(),
            );

            // Common Schema events use a level based on a span's Status
            provider.create_unregistered(
                true,
                linux_tlg::Level::Error,
                exporter_config.get_span_keywords(),
            );
            provider.create_unregistered(
                true,
                linux_tlg::Level::Verbose,
                exporter_config.get_span_keywords(),
            );
        }

        BatchExporter {
            config: exporter_config,
            ebw: UserEventsExporter::new(Arc::new(provider)),
        }
    }
}

impl<C: KeywordLevelProvider, E: EventExporter + Send + Sync> Debug for BatchExporter<C, E> {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

impl<C: KeywordLevelProvider, E: EventExporter + Send + Sync> SpanExporter for BatchExporter<C, E> {
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
