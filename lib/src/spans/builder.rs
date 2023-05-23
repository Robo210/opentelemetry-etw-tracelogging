use opentelemetry_api::global;
use opentelemetry_api::global::GlobalTracerProvider;
use opentelemetry_api::trace::TracerProvider;

use crate::ExporterBuilder;
use crate::ExporterConfig;
use crate::DefaultKeywordLevelProvider;
use crate::EtwExporterAsyncRuntime;
use crate::spans;

pub struct SpanExporterBuilder {
    pub(crate) parent: ExporterBuilder,
    pub(crate) trace_config: Option<opentelemetry_sdk::trace::Config>,
}

impl SpanExporterBuilder {
    /// Assign the SDK trace configuration.
    pub fn with_trace_config(mut self, config: opentelemetry_sdk::trace::Config) -> Self {
        self.trace_config = Some(config);
        self
    }

    /// Install the exporter as the
    /// [global tracer provider](https://docs.rs/opentelemetry_api/latest/opentelemetry_api/global/index.html).
    pub fn install_span_exporter(
        &mut self,
    ) -> <GlobalTracerProvider as opentelemetry_api::trace::TracerProvider>::Tracer {
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
                            opentelemetry_sdk::trace::TracerProvider::builder()
                                .with_simple_exporter(spans::BatchExporter::new(
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
                        None => opentelemetry_sdk::trace::TracerProvider::builder()
                            .with_simple_exporter(spans::BatchExporter::new(
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

                    let provider_builder = match self.parent.exporter_config {
                        Some(exporter_config) => {
                            opentelemetry_sdk::trace::TracerProvider::builder().with_batch_exporter(
                                spans::BatchExporter::new(
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
                        None => opentelemetry_sdk::trace::TracerProvider::builder()
                            .with_batch_exporter(
                                spans::BatchExporter::new(
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

            match self.parent.exporter_config {
                Some(exporter_config) => {
                    let provider = spans::RealtimeTracerProvider::new(
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

                    let _ = global::set_tracer_provider(provider);
                }
                None => {
                    let provider = spans::RealtimeTracerProvider::new(
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

                    let _ = global::set_tracer_provider(provider);
                }
            }
        }

        global::tracer_provider().tracer(
            #[cfg(all(target_os = "windows"))]
            "opentelemetry-etw",
            #[cfg(all(target_os = "linux"))]
            "opentelemetry-user_events",
        )
    }
}
