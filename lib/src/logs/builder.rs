use opentelemetry_api::global;
use opentelemetry_api::global::GlobalLoggerProvider;
use opentelemetry_api::logs::LoggerProvider;

use crate::ExporterBuilder;
use crate::ExporterConfig;
use crate::DefaultKeywordLevelProvider;
use crate::EtwExporterAsyncRuntime;
use crate::logs;

pub struct LogsExporterBuilder {
    pub(crate) parent: ExporterBuilder,
    pub(crate) log_config: Option<opentelemetry_sdk::logs::Config>,
}

impl LogsExporterBuilder {
    /// Install the exporter as the
    /// [global logger provider](https://docs.rs/opentelemetry_api/latest/opentelemetry_api/global/index.html).
    pub fn install_log_exporter(
        mut self,
    ) -> <GlobalLoggerProvider as opentelemetry_api::logs::LoggerProvider>::Logger {
        self.parent.validate_config();

        // This will always return a boxed trait object.
        // Hopefully that won't cause too much of a performance issue, since that is a limitation of the global tracer as well.

        // Avoid adding an extra dyn indirection by making sure BatchExporter/RealtimeExporter can be specialized for the keyword provider type.
        // Non-default keyword providers will always be boxed trait objects, but that shouldn't be the common case.

        if !self.parent.emit_realtime_events {
            let provider_builder = match self.parent.runtime {
                None => {
                    let provider_builder = match self.parent.exporter_config {
                        Some(exporter_config) => {
                            opentelemetry_sdk::logs::LoggerProvider::builder()
                                .with_simple_exporter(logs::BatchExporter::new(
                                    &self.parent.provider_name,
                                    self.parent.provider_group,
                                    self.parent.use_byte_for_bools,
                                    ExporterConfig {
                                        kwl: exporter_config,
                                        json: self.parent.json,
                                        common_schema: self.parent.emit_common_schema_events,
                                        etw_activities: self.parent.emit_realtime_events,
                                    },
                                ))
                        }
                        None => opentelemetry_sdk::logs::LoggerProvider::builder()
                            .with_simple_exporter(logs::BatchExporter::new(
                                &self.parent.provider_name,
                                self.parent.provider_group,
                                self.parent.use_byte_for_bools,
                                ExporterConfig {
                                    kwl: DefaultKeywordLevelProvider,
                                    json: self.parent.json,
                                    common_schema: self.parent.emit_common_schema_events,
                                    etw_activities: self.parent.emit_realtime_events,
                                },
                            )),
                    };

                    if let Some(config) = self.log_config.take() {
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

                    let provider_builder = match self.parent.exporter_config {
                        Some(exporter_config) => {
                            opentelemetry_sdk::logs::LoggerProvider::builder().with_batch_exporter(
                                logs::BatchExporter::new(
                                    &self.parent.provider_name,
                                    self.parent.provider_group,
                                    self.parent.use_byte_for_bools,
                                    ExporterConfig {
                                        kwl: exporter_config,
                                        json: self.parent.json,
                                        common_schema: self.parent.emit_common_schema_events,
                                        etw_activities: self.parent.emit_realtime_events,
                                    },
                                ),
                                runtime,
                            )
                        }
                        None => opentelemetry_sdk::logs::LoggerProvider::builder()
                            .with_batch_exporter(
                                logs::BatchExporter::new(
                                    &self.parent.provider_name,
                                    self.parent.provider_group,
                                    self.parent.use_byte_for_bools,
                                    ExporterConfig {
                                        kwl: DefaultKeywordLevelProvider,
                                        json: self.parent.json,
                                        common_schema: self.parent.emit_common_schema_events,
                                        etw_activities: self.parent.emit_realtime_events,
                                    },
                                ),
                                runtime,
                            ),
                    };

                    if let Some(config) = self.log_config.take() {
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
            let _ = global::set_logger_provider(provider);
        } else {
            let otel_config = if let Some(config) = self.log_config.take() {
                config
            } else {
                opentelemetry_sdk::logs::config()
            };

            match self.parent.exporter_config {
                Some(exporter_config) => {
                    let provider = logs::RealtimeLoggerProvider::new(
                        &self.parent.provider_name,
                        self.parent.provider_group,
                        otel_config,
                        self.parent.use_byte_for_bools,
                        ExporterConfig {
                            kwl: exporter_config,
                            json: self.parent.json,
                            common_schema: self.parent.emit_common_schema_events,
                            etw_activities: self.parent.emit_realtime_events,
                        },
                    );

                    let _ = global::set_logger_provider(provider);
                }
                None => {
                    let provider = logs::RealtimeLoggerProvider::new(
                        &self.parent.provider_name,
                        self.parent.provider_group,
                        otel_config,
                        self.parent.use_byte_for_bools,
                        ExporterConfig {
                            kwl: DefaultKeywordLevelProvider,
                            json: self.parent.json,
                            common_schema: self.parent.emit_common_schema_events,
                            etw_activities: self.parent.emit_realtime_events,
                        },
                    );

                    let _ = global::set_logger_provider(provider);
                }
            }
        }

        global::logger_provider().logger(
            #[cfg(all(target_os = "windows"))]
            "opentelemetry-etw",
            #[cfg(all(target_os = "linux"))]
            "opentelemetry-user_events",
        )
    }
}
