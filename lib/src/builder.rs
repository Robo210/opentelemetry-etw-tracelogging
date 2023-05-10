use std::borrow::Cow;

use crate::batch_exporter::*;
use crate::exporter_traits::*;
use crate::realtime_exporter::*;
use opentelemetry::global::GlobalTracerProvider;
use opentelemetry_api::{global, trace::TracerProvider};
use tracelogging_dynamic::Guid;

/// The async runtime to use with OpenTelemetry-Rust's BatchExporter.
/// See <https://docs.rs/opentelemetry/latest/opentelemetry/index.html#crate-feature-flags>
/// for more details.
#[derive(Debug)]
pub enum EtwExporterAsyncRuntime {
    #[cfg(any(feature = "rt-tokio"))]
    #[cfg_attr(docsrs, doc(cfg(feature = "rt-tokio")))]
    Tokio,
    #[cfg(any(feature = "rt-tokio-current-thread"))]
    #[cfg_attr(docsrs, doc(cfg(feature = "rt-tokio-current-thread")))]
    TokioCurrentThread,
    #[cfg(any(feature = "rt-async-std"))]
    #[cfg_attr(docsrs, doc(cfg(feature = "rt-async-std")))]
    AsyncStd,
}

pub(crate) enum ProviderGroup {
    Unset,
    #[allow(dead_code)]
    Windows(Guid),
    #[allow(dead_code)]
    Linux(Cow<'static, str>),
}

/// Create a new exporter builder by calling [`ExporterBuilder::new'].
pub struct ExporterBuilder {
    provider_name: String,
    provider_id: Guid,
    provider_group: ProviderGroup,
    use_byte_for_bools: bool,
    json: bool,
    emit_common_schema_events: bool,
    emit_realtime_events: bool,
    runtime: Option<EtwExporterAsyncRuntime>,
    trace_config: Option<opentelemetry_sdk::trace::Config>,
    exporter_config: Option<Box<dyn KeywordLevelProvider>>,
}

/// Create an exporter builder. After configuring the builder,
/// call [`ExporterBuilder::install`] to set it as the
/// [global tracer provider](https://docs.rs/opentelemetry_api/latest/opentelemetry_api/global/index.html).
pub fn new_exporter(name: &str) -> ExporterBuilder {
    ExporterBuilder {
        provider_name: name.to_owned(),
        provider_id: Guid::from_name(name),
        provider_group: ProviderGroup::Unset,
        use_byte_for_bools: false,
        json: false,
        emit_common_schema_events: false,
        emit_realtime_events: true,
        runtime: None,
        trace_config: None,
        exporter_config: None,
    }
}

impl ExporterBuilder {
    /// For advanced scenarios.
    /// Assign a provider ID to the ETW provider rather than use
    /// one generated from the provider name.
    pub fn with_provider_id(mut self, guid: Guid) -> Self {
        self.provider_id = guid;
        self
    }

    /// Get the current provider ID that will be used for the ETW provider.
    /// This is a convenience function to help with tools that do not implement
    /// the standard provider name to ID algorithm.
    pub fn get_provider_id(&self) -> Guid {
        self.provider_id
    }

    /// Log bool attributes using an InType of `xs:byte` instead of `win:Boolean`.
    /// This is non-standard and not recommended except if compatibility with the
    /// C++ ETW exporter is required.
    /// This option has no effect for Linux user_events.
    pub fn with_byte_sized_bools(mut self) -> Self {
        self.use_byte_for_bools = true;
        self
    }

    /// Assign the SDK trace configuration.
    pub fn with_trace_config(mut self, config: opentelemetry_sdk::trace::Config) -> Self {
        self.trace_config = Some(config);
        self
    }

    /// Override the default keywords and levels for events.
    /// Provide an implementation of the [`KeywordLevelProvider`] trait that will
    /// return the desired keywords and level values for each type of event.
    pub fn with_exporter_config(mut self, config: impl KeywordLevelProvider + 'static) -> Self {
        self.exporter_config = Some(Box::new(config));
        self
    }

    /// For advanced scenarios.
    /// Encode the event payload as a single JSON string rather than multiple fields.
    /// Recommended only for compatibility with the C++ ETW exporter. In general,
    /// the textual representation of the event payload should be left to the event
    /// consumer.
    /// Requires the `json` feature to be enabled on the crate.
    #[cfg(any(feature = "json"))]
    #[cfg_attr(docsrs, doc(cfg(feature = "json")))]
    pub fn with_json_payload(mut self) -> Self {
        self.json = true;
        self
    }

    /// For advanced scenarios.
    /// Emit extra events that follow the Common Schema 4.0 mapping.
    /// Recommended only for compatibility with specialized event consumers.
    /// Most ETW consumers will not benefit from events in this schema, and
    /// may perform worse.
    /// These events are emitted in addition to the normal ETW events,
    /// unless `without_realtime_events` is also called.
    /// Common Schema events take longer to generate, and should only be
    /// used with the Batch Exporter.
    pub fn with_common_schema_events(mut self) -> Self {
        self.emit_common_schema_events = true;
        self
    }

    /// For advanced scenarios.
    /// Do not emit realtime events. Use this option in conjunction with
    /// [`Self::with_common_schema_events`] to only emit Common Schema events.
    /// Recommended only for compatibility with specialized event consumers.
    /// Most ETW consumers will not benefit from Common Schema events, and may perform worse.
    pub fn without_realtime_events(mut self) -> Self {
        self.emit_realtime_events = false;
        self
    }

    /// For advanced scenarios.
    /// Set the ETW provider group to join this provider to.
    #[cfg(any(target_os = "windows", doc))]
    pub fn with_provider_group(mut self, group_id: Guid) -> Self {
        self.provider_group = ProviderGroup::Windows(group_id);
        self
    }

    /// For advanced scenarios.
    /// Set the EventHeader provider group to join this provider to.
    #[cfg(any(target_os = "linux", doc))]
    pub fn with_provider_group(mut self, name: &str) -> Self {
        self.provider_group = ProviderGroup::Linux(Cow::Owned(name.to_owned()));
        self
    }

    /// Set which OpenTelemetry-Rust async runtime to use.
    /// See <https://docs.rs/opentelemetry/latest/opentelemetry/index.html#crate-feature-flags>
    /// for more details.
    /// Requires one of the "rt-" features to be enabled on the crate.
    #[cfg(any(
        feature = "rt-tokio",
        feature = "rt-tokio-current-thread",
        feature = "rt-async-std",
        doc
    ))]
    pub fn with_async_runtime(mut self, runtime: EtwExporterAsyncRuntime) -> Self {
        self.runtime = Some(runtime);
        self
    }

    fn validate_config(&self) {
        if !self.emit_common_schema_events && !self.emit_realtime_events {
            panic!("at least one ETW event type must be enabled");
        }

        match self.runtime.as_ref() {
            None => (),
            Some(x) => match x {
                #[cfg(any(feature = "rt-tokio"))]
                EtwExporterAsyncRuntime::Tokio => (),
                #[cfg(any(feature = "rt-tokio-current-thread"))]
                EtwExporterAsyncRuntime::TokioCurrentThread => (),
                #[cfg(any(feature = "rt-async-std"))]
                EtwExporterAsyncRuntime::AsyncStd => (),
            },
        }

        match &self.provider_group {
            ProviderGroup::Unset => (),
            ProviderGroup::Windows(guid) => {
                assert_ne!(guid, &Guid::zero(), "Provider GUID must not be zeroes");
            }
            ProviderGroup::Linux(name) => {
                assert!(
                    eventheader_dynamic::ProviderOptions::is_valid_option_value(&name),
                    "Provider names must be lower case ASCII or numeric digits"
                );
            }
        }

        #[cfg(all(target_os = "linux"))]
        if self
            .provider_name
            .contains(|f: char| !f.is_ascii_alphanumeric())
        {
            // The perf command is very particular about the provider names it accepts.
            // The Linux kernel itself cares less, and other event consumers should also presumably not need this check.
            //panic!("Linux provider names must be ASCII alphanumeric");
        }
    }

    /// Install the exporter as the
    /// [global tracer provider](https://docs.rs/opentelemetry_api/latest/opentelemetry_api/global/index.html).
    pub fn install(
        mut self,
    ) -> <GlobalTracerProvider as opentelemetry_api::trace::TracerProvider>::Tracer {
        self.validate_config();

        // This will always return a boxed trait object.
        // Hopefully that won't cause too much of a performance issue, since that is a limitation of the global tracer as well.

        // Avoid adding an extra dyn indirection by making sure BatchExporter/RealtimeExporter can be specialized for the keyword provider type.
        // Non-default keyword providers will always be boxed trait objects, but that shouldn't be the common case.

        if !self.emit_realtime_events {
            let provider_builder = match self.runtime {
                None => {
                    let provider_builder = match self.exporter_config {
                        Some(exporter_config) => {
                            opentelemetry_sdk::trace::TracerProvider::builder()
                                .with_simple_exporter(BatchExporter::new(
                                    &self.provider_name,
                                    self.provider_group,
                                    self.use_byte_for_bools,
                                    ExporterConfig {
                                        kwl: exporter_config,
                                        json: self.json,
                                        common_schema: self.emit_common_schema_events,
                                        etw_activities: self.emit_realtime_events,
                                    },
                                ))
                        }
                        None => opentelemetry_sdk::trace::TracerProvider::builder()
                            .with_simple_exporter(BatchExporter::new(
                                &self.provider_name,
                                self.provider_group,
                                self.use_byte_for_bools,
                                ExporterConfig {
                                    kwl: DefaultKeywordLevelProvider,
                                    json: self.json,
                                    common_schema: self.emit_common_schema_events,
                                    etw_activities: self.emit_realtime_events,
                                },
                            )),
                    };

                    if let Some(config) = self.trace_config.take() {
                        provider_builder.with_config(config)
                    } else {
                        provider_builder
                    }
                }
                #[cfg(any(
                    feature = "rt-tokio",
                    feature = "rt-tokio-current-thread",
                    feature = "rt-async-std"
                ))]
                Some(runtime) => {
                    // If multiple runtimes are enabled this won't compile due to mismatched arms in the match
                    let runtime = match runtime {
                        #[cfg(any(feature = "rt-tokio"))]
                        EtwExporterAsyncRuntime::Tokio => opentelemetry_sdk::runtime::Tokio,
                        #[cfg(any(feature = "rt-tokio-current-thread"))]
                        EtwExporterAsyncRuntime::TokioCurrentThread => {
                            opentelemetry_sdk::runtime::TokioCurrentThread
                        }
                        #[cfg(any(feature = "rt-async-std"))]
                        EtwExporterAsyncRuntime::AsyncStd => opentelemetry_sdk::runtime::AsyncStd,
                    };

                    let provider_builder = match self.exporter_config {
                        Some(exporter_config) => {
                            opentelemetry_sdk::trace::TracerProvider::builder().with_batch_exporter(
                                BatchExporter::new(
                                    &self.provider_name,
                                    self.provider_group,
                                    self.use_byte_for_bools,
                                    ExporterConfig {
                                        kwl: exporter_config,
                                        json: self.json,
                                        common_schema: self.emit_common_schema_events,
                                        etw_activities: self.emit_realtime_events,
                                    },
                                ),
                                runtime,
                            )
                        }
                        None => opentelemetry_sdk::trace::TracerProvider::builder()
                            .with_batch_exporter(
                                BatchExporter::new(
                                    &self.provider_name,
                                    self.provider_group,
                                    self.use_byte_for_bools,
                                    ExporterConfig {
                                        kwl: DefaultKeywordLevelProvider,
                                        json: self.json,
                                        common_schema: self.emit_common_schema_events,
                                        etw_activities: self.emit_realtime_events,
                                    },
                                ),
                                runtime,
                            ),
                    };

                    if let Some(config) = self.trace_config.take() {
                        provider_builder.with_config(config)
                    } else {
                        provider_builder
                    }
                }
                #[cfg(not(any(
                    feature = "rt-tokio",
                    feature = "rt-tokio-current-thread",
                    feature = "rt-async-std"
                )))]
                Some(_) => todo!(), // Unreachable
            };

            let provider = provider_builder.build();
            let _ = global::set_tracer_provider(provider);
        } else {
            let otel_config = if let Some(config) = self.trace_config.take() {
                config
            } else {
                opentelemetry_sdk::trace::config()
            };

            match self.exporter_config {
                Some(exporter_config) => {
                    let provider = RealtimeTracerProvider::new(
                        &self.provider_name,
                        self.provider_group,
                        otel_config,
                        self.use_byte_for_bools,
                        ExporterConfig {
                            kwl: exporter_config,
                            json: self.json,
                            common_schema: self.emit_common_schema_events,
                            etw_activities: self.emit_realtime_events,
                        },
                    );

                    let _ = global::set_tracer_provider(provider);
                }
                None => {
                    let provider = RealtimeTracerProvider::new(
                        &self.provider_name,
                        self.provider_group,
                        otel_config,
                        self.use_byte_for_bools,
                        ExporterConfig {
                            kwl: DefaultKeywordLevelProvider,
                            json: self.json,
                            common_schema: self.emit_common_schema_events,
                            etw_activities: self.emit_realtime_events,
                        },
                    );

                    let _ = global::set_tracer_provider(provider);
                }
            }
        }

        global::tracer_provider().versioned_tracer(
            #[cfg(all(target_os = "windows"))]
            "opentelemetry-etw",
            #[cfg(all(target_os = "linux"))]
            "opentelemetry-user_events",
            Some(env!("CARGO_PKG_VERSION")),
            Some("https://microsoft.com/etw"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_builder() {
        let builder = new_exporter("my_provider_name");
        assert!(
            builder.get_provider_id()
                == Guid::from_fields(
                    0x6386e494,
                    0x0d79,
                    0x57b7,
                    [0xa8, 0x38, 0x16, 0x8d, 0xc4, 0x19, 0xf5, 0x42]
                )
        );

        let builder = builder.with_provider_id(Guid::from_fields(
            0x1fa0f771,
            0x9607,
            0x4fe2,
            [0x8c, 0x39, 0x2b, 0x6c, 0x61, 0x43, 0xbb, 0x87],
        ));
        assert!(
            builder.get_provider_id()
                == Guid::from_fields(
                    0x1fa0f771,
                    0x9607,
                    0x4fe2,
                    [0x8c, 0x39, 0x2b, 0x6c, 0x61, 0x43, 0xbb, 0x87]
                )
        );
    }

    #[cfg(any(feature = "rt-tokio"))]
    #[tokio::test]
    async fn install_batch() {
        new_exporter("my_provider_name")
            .with_common_schema_events()
            .without_realtime_events()
            .with_async_runtime(EtwExporterAsyncRuntime::Tokio)
            .install();
    }

    #[test]
    fn install_realtime() {
        new_exporter("my_provider_name").install();
    }
}
