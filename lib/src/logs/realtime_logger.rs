use std::{sync::{Arc, Weak}, borrow::Cow};

use opentelemetry_api::{InstrumentationLibrary, logs::LogRecord};
use opentelemetry_sdk::{export::logs::LogData};
use tracelogging::InType;
use tracelogging_dynamic::Provider;

use crate::{etw, EventExporter, KeywordLevelProvider, ProviderGroup, ExporterConfig};


pub struct RealtimeLogger<E: EventExporter> {
    otel_config: Weak<opentelemetry_sdk::logs::Config>,
    event_exporter: Weak<E>,
    instrumentation_lib: InstrumentationLibrary,
}

impl<E: EventExporter> RealtimeLogger<E> {
    fn new(
        otel_config: Weak<opentelemetry_sdk::logs::Config>,
        event_exporter: Weak<E>,
        instrumentation_lib: InstrumentationLibrary,
    ) -> Self {
        RealtimeLogger {
            otel_config,
            event_exporter,
            instrumentation_lib,
        }
    }
}

impl<E: EventExporter> opentelemetry_api::logs::Logger for RealtimeLogger<E> {
    fn emit(&self, record: LogRecord) {
         if let Some(exporter) = self.event_exporter.upgrade() {
            let config = if let Some(config) = self.otel_config.upgrade() {
                config.clone()
            } else {
                Default::default()
            };

            let data = LogData {
                record,
                resource: config.resource.clone(),
                instrumentation: self.instrumentation_lib.clone(),
            };
            let _ = exporter.log_log_data(&data);
         }
    }
}

pub struct RealtimeLoggerProvider<C: KeywordLevelProvider, E: EventExporter> {
    otel_config: Arc<opentelemetry_sdk::logs::Config>,
    event_exporter: Arc<E>,
    _x: core::marker::PhantomData<C>,
}

#[cfg(all(target_os = "windows"))]
impl<C: KeywordLevelProvider> RealtimeLoggerProvider<C, etw::EtwEventExporter<C>> {
    pub(crate) fn new(
        provider_name: &str,
        provider_group: ProviderGroup,
        otel_config: opentelemetry_sdk::logs::Config,
        use_byte_for_bools: bool,
        exporter_config: ExporterConfig<C>,
    ) -> Self {
        let mut options = Provider::options();
        if let ProviderGroup::Windows(guid) = provider_group {
            options = *options.group_id(&guid);
        }

        let provider = Arc::pin(Provider::new(provider_name, &options));
        unsafe {
            provider.as_ref().register();
        }

        RealtimeLoggerProvider {
            otel_config: Arc::new(otel_config),
            event_exporter: Arc::new(etw::EtwEventExporter::new(
                provider,
                exporter_config,
                if use_byte_for_bools {
                    InType::U8
                } else {
                    InType::Bool32
                },
            )),
            _x: core::marker::PhantomData,
        }
    }
}

#[cfg(all(target_os = "linux"))]
impl<C: KeywordLevelProvider> RealtimeLoggerProvider<C, user_events::UserEventsExporter<C>> {
    pub(crate) fn new(
        provider_name: &str,
        provider_group: ProviderGroup,
        otel_config: opentelemetry_sdk::logs::Config,
        _use_byte_for_bools: bool,
        exporter_config: ExporterConfig<C>,
    ) -> Self {
        let mut options = eventheader_dynamic::Provider::new_options();
        if let ProviderGroup::Linux(ref name) = provider_group {
            options = *options.group_name(&name);
        }
        let mut provider = eventheader_dynamic::Provider::new(provider_name, &options);
        user_events::register_eventsets(&mut provider, &exporter_config);

        RealtimeLoggerProvider {
            otel_config: Arc::new(otel_config),
            event_exporter: Arc::new(user_events::UserEventsExporter::new(Arc::new(provider), exporter_config)),
            _x: core::marker::PhantomData,
        }
    }
}

impl<C: KeywordLevelProvider, E: EventExporter> opentelemetry_api::logs::LoggerProvider
    for RealtimeLoggerProvider<C, E>
{
    type Logger = RealtimeLogger<E>;

    fn logger(&self, name: impl Into<Cow<'static, str>>) -> Self::Logger {
        self.versioned_logger(
            name,
            Some(Cow::Borrowed(env!("CARGO_PKG_VERSION"))),
            Some(Cow::Borrowed("https://microsoft.com/etw")),
            None,
            false
        )
    }

    fn versioned_logger(
            &self,
            name: impl Into<Cow<'static, str>>,
            version: Option<Cow<'static, str>>,
            schema_url: Option<Cow<'static, str>>,
            attributes: Option<Vec<opentelemetry_api::KeyValue>>,
            include_trace_context: bool
        ) -> Self::Logger {
        let name = name.into();
        // Use default value if name is invalid empty string
        let component_name = if name.is_empty() {
            Cow::Borrowed("opentelemetry-etw-user_events")
        } else {
            name
        };
        let instrumentation_lib = InstrumentationLibrary::new(
            component_name,
            version,
            schema_url,
            attributes
        );

        RealtimeLogger::new(
            Arc::downgrade(&self.otel_config),
            Arc::downgrade(&self.event_exporter),
            instrumentation_lib,
        )
    }
}
