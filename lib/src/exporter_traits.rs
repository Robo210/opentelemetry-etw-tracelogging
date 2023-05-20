use crate::common::EtwSpan;

/// Implement this trait to provide an override for
/// event keywords or levels.
///
/// By default, span start and stop events are logged with keyword 0x1 and
/// [`tracelogging::Level::Informational`].
/// Events attached to the span are logged with keyword 0x10 and [`tracelogging::Level::Verbose`].
/// Span Links are logged as events with keyword 0x100 and [`tracelogging::Level::Verbose`].
pub trait KeywordLevelProvider: Send + Sync {
    /// The keyword(s) to use for Span start/stop events.
    fn get_span_keywords(&self) -> u64;
    /// The keyword(s) to use for Span Event events.
    fn get_event_keywords(&self) -> u64;
    /// The keyword(s) to use for Span Link events.
    fn get_links_keywords(&self) -> u64;

    /// The level to use for Span start/stop events.
    fn get_span_level(&self) -> u8;
    /// The keyword(s) to use for Span Event events.
    fn get_event_level(&self) -> u8;
    /// The keyword(s) to use for Span Link events.
    fn get_links_level(&self) -> u8;
}

pub(crate) struct ExporterConfig<T: KeywordLevelProvider> {
    pub(crate) kwl: T,
    pub(crate) json: bool,
    pub(crate) common_schema: bool,
    pub(crate) etw_activities: bool,
}

pub(crate) struct DefaultKeywordLevelProvider;

impl KeywordLevelProvider for DefaultKeywordLevelProvider {
    #[inline(always)]
    fn get_span_keywords(&self) -> u64 {
        0x1
    }

    #[inline(always)]
    fn get_event_keywords(&self) -> u64 {
        0x10
    }

    #[inline(always)]
    fn get_links_keywords(&self) -> u64 {
        0x100
    }

    #[inline(always)]
    fn get_span_level(&self) -> u8 {
        4 // Level::Informational
    }

    #[inline(always)]
    fn get_event_level(&self) -> u8 {
        5 // Level::Verbose
    }

    #[inline(always)]
    fn get_links_level(&self) -> u8 {
        5 // Level::Verbose
    }
}

impl KeywordLevelProvider for Box<dyn KeywordLevelProvider> {
    #[inline(always)]
    fn get_span_keywords(&self) -> u64 {
        self.as_ref().get_span_keywords()
    }

    #[inline(always)]
    fn get_event_keywords(&self) -> u64 {
        self.as_ref().get_event_keywords()
    }

    #[inline(always)]
    fn get_links_keywords(&self) -> u64 {
        self.as_ref().get_links_keywords()
    }

    #[inline(always)]
    fn get_span_level(&self) -> u8 {
        self.as_ref().get_span_level()
    }

    #[inline(always)]
    fn get_event_level(&self) -> u8 {
        self.as_ref().get_event_level()
    }

    #[inline(always)]
    fn get_links_level(&self) -> u8 {
        self.as_ref().get_links_level()
    }
}

impl<T: KeywordLevelProvider> KeywordLevelProvider for ExporterConfig<T> {
    #[inline(always)]
    fn get_span_keywords(&self) -> u64 {
        self.kwl.get_span_keywords()
    }

    #[inline(always)]
    fn get_event_keywords(&self) -> u64 {
        self.kwl.get_event_keywords()
    }

    #[inline(always)]
    fn get_links_keywords(&self) -> u64 {
        self.kwl.get_links_keywords()
    }

    #[inline(always)]
    fn get_span_level(&self) -> u8 {
        self.kwl.get_span_level()
    }

    #[inline(always)]
    fn get_event_level(&self) -> u8 {
        self.kwl.get_event_level()
    }

    #[inline(always)]
    fn get_links_level(&self) -> u8 {
        self.kwl.get_links_level()
    }
}

impl<T: KeywordLevelProvider> ExporterConfig<T> {
    #[inline(always)]
    pub(crate) fn get_export_as_json(&self) -> bool {
        self.json
    }

    #[inline(always)]
    pub(crate) fn get_export_common_schema_event(&self) -> bool {
        self.common_schema
    }

    #[inline(always)]
    pub(crate) fn get_export_span_events(&self) -> bool {
        self.etw_activities
    }
}

#[doc(hidden)]
pub trait EventExporter {
    fn enabled(&self, level: u8, keyword: u64) -> bool;

    // Called by the real-time exporter when a span is started
    fn log_span_start<S>(&self, span: &S) -> opentelemetry_sdk::export::trace::ExportResult
    where
        S: opentelemetry_api::trace::Span + EtwSpan;

    // Called by the real-time exporter when a span is ended
    fn log_span_end<S>(&self, span: &S) -> opentelemetry_sdk::export::trace::ExportResult
    where
        S: opentelemetry_api::trace::Span + EtwSpan;

    // Called by the real-time exporter when an event is added to a span
    fn log_span_event<S>(
        &self,
        event: opentelemetry_api::trace::Event,
        span: &S,
    ) -> opentelemetry_sdk::export::trace::ExportResult
    where
        S: opentelemetry_api::trace::Span + EtwSpan;

    // Called by the batch exporter sometime after span is completed
    fn log_span_data(
        &self,
        span_data: &opentelemetry_sdk::export::trace::SpanData,
    ) -> opentelemetry_sdk::export::trace::ExportResult;

    // Called by the batch exporter sometime after span is completed
    fn log_log_data(
        &self,
        log_data: &opentelemetry_sdk::export::logs::LogData,
    ) -> opentelemetry_sdk::export::logs::ExportResult;
}

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
