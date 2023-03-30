use crate::batch_exporter::*;
use crate::realtime_exporter::*;
use opentelemetry_api::{global, trace::TracerProvider};
use tracelogging_dynamic::Guid;

#[derive(Debug)]
pub struct EtwExporterBuilder {
    provider_name: String,
    provider_id: Guid,
    use_byte_for_bools: bool,
    json: bool,
    emit_common_schema_events: bool,
    emit_only_common_schema_events: bool,
    trace_config: Option<opentelemetry_sdk::trace::Config>,
}

pub fn new_etw_exporter(name: &str) -> EtwExporterBuilder {
    EtwExporterBuilder {
        provider_name: name.to_owned(),
        provider_id: Guid::from_name(name),
        use_byte_for_bools: false,
        json: false,
        emit_common_schema_events: false,
        emit_only_common_schema_events: false,
        trace_config: None,
    }
}

impl EtwExporterBuilder {
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
    pub fn with_byte_sized_bools(mut self) -> Self {
        self.use_byte_for_bools = true;
        self
    }

    /// Assign the SDK trace configuration.
    pub fn with_trace_config(mut self, config: opentelemetry_sdk::trace::Config) -> Self {
        self.trace_config = Some(config);
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
    /// unless `without_normal_events` is also called.
    /// Common Schema events take longer to generate, and should only be
    /// used with the Batch Exporter.
    pub fn with_common_schema_events(mut self) -> Self {
        self.emit_common_schema_events = true;
        self
    }

    /// For advanced scenarios.
    /// Emit *only* events that follows the Common Schema 4.0 mapping.
    /// Recommended only for compatibility with specialized event consumers.
    /// Most ETW consumers will not benefit from events in this schema, and may perform worse.
    pub fn without_normal_events(mut self) -> Self {
        self.emit_common_schema_events = true;
        self.emit_only_common_schema_events = true;
        self
    }

    /// Install the exporter as a batch span exporter.
    /// Spans will be exported some time after the span has ended.
    /// The timestamps of the ETW events, and the duration of time between
    /// events, will not be accurate.
    /// Requires the "async" feature to be enabled on the crate.
    #[cfg(any(feature = "async"))]
    #[cfg_attr(docsrs, doc(cfg(feature = "async")))]
    pub fn install_batch<R: opentelemetry_sdk::trace::TraceRuntime>(
        mut self,
        runtime: R,
    ) -> opentelemetry_sdk::trace::Tracer {
        let exporter = BatchExporter::new(
            &self.provider_name,
            self.use_byte_for_bools,
            self.json,
            self.emit_common_schema_events,
            !self.emit_only_common_schema_events,
        );

        let provider_builder = opentelemetry_sdk::trace::TracerProvider::builder()
            .with_batch_exporter(exporter, runtime);

        let builder = if let Some(config) = self.trace_config.take() {
            provider_builder.with_config(config)
        } else {
            provider_builder
        };

        let provider = builder.build();

        let tracer = provider.versioned_tracer(
            "opentelemetry-etw",
            Some(env!("CARGO_PKG_VERSION")),
            Some("https://microsoft.com/etw"),
        );
        let _ = global::set_tracer_provider(provider);

        tracer
    }

    /// Install the exporter as a tracer provider.
    /// Spans will be exported almost immediately after they are started
    /// and ended. Events that were added to the span will be exported
    /// at the same time as the end event. The timestamps of the start
    /// and end ETW events will roughly match the actual start and end of the span.
    pub fn install_realtime(mut self) -> RealtimeTracer {
        let otel_config = if let Some(config) = self.trace_config.take() {
            config
        } else {
            opentelemetry_sdk::trace::config()
        };

        let provider = RealtimeTracerProvider::new(
            &self.provider_name,
            otel_config,
            self.use_byte_for_bools,
            self.json,
            self.emit_common_schema_events,
            !self.emit_only_common_schema_events,
        );

        let tracer = provider.versioned_tracer(
            "opentelemetry-etw",
            Some(env!("CARGO_PKG_VERSION")),
            Some("https://microsoft.com/etw"),
        );
        let _ = global::set_tracer_provider(provider);

        tracer
    }
}

#[allow(unused_imports)]
mod tests {
    use super::*;

    #[test]
    fn create_builder() {
        let builder = new_etw_exporter("my_provider_name");
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

    #[cfg(any(feature = "async"))]
    #[tokio::test]
    async fn install_batch() {
        new_etw_exporter("my_provider_name")
            .with_common_schema_events()
            .without_normal_events()
            .install_batch(opentelemetry_sdk::runtime::Tokio);
    }

    #[test]
    fn install_realtime() {
        new_etw_exporter("my_provider_name").install_realtime();
    }
}
