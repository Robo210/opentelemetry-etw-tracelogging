use std::{
    io::{Cursor, Write},
    mem::MaybeUninit,
};

use opentelemetry::trace::{SpanId, TraceId};

/// Implement this trait to provide an override for
/// event keywords or levels.
///
/// By default, span start and stop events are logged with keyword 1 and
/// [`tracelogging::Level::Informational`].
/// Events attached to the span are logged with keyword 2 and [`tracelogging::Level::Verbose`].
/// Span Links are logged as events with keyword 4 and [`tracelogging::Level::Verbose`].
pub trait KeywordLevelProvider : Send + Sync {
    /// The keyword(s) to use for Span start/stop events.
    fn get_span_keywords(&self) -> u64;
    /// The keyword(s) to use for Span Event events.
    fn get_event_keywords(&self) -> u64;
    /// The keyword(s) to use for Span Link events.
    fn get_links_keywords(&self) -> u64;
}

// Public because RealtimeSpan is parameterized for ExporterConfig<C: KeywordLevelProvider>
// Should otherwise be pub(crate).
#[derive(Clone)]
#[doc(hidden)]
pub struct ExporterConfig<T: KeywordLevelProvider> {
    pub(crate) kwl: T,
    pub(crate) json: bool,
    pub(crate) common_schema: bool,
    pub(crate) etw_activities: bool,
}

pub(crate) struct DefaultKeywordLevelProvider;

impl KeywordLevelProvider for DefaultKeywordLevelProvider {
    fn get_span_keywords(&self) -> u64 {
        1
    }

    fn get_event_keywords(&self) -> u64 {
        2
    }

    fn get_links_keywords(&self) -> u64 {
        4
    }
}

impl<> KeywordLevelProvider for Box<dyn KeywordLevelProvider> {
    fn get_span_keywords(&self) -> u64 {
        self.as_ref().get_span_keywords()
    }

    fn get_event_keywords(&self) -> u64 {
        self.as_ref().get_event_keywords()
    }

    fn get_links_keywords(&self) -> u64 {
        self.as_ref().get_links_keywords()
    }
}

impl<T: KeywordLevelProvider> KeywordLevelProvider for ExporterConfig<T> {
    fn get_span_keywords(&self) -> u64 {
        self.kwl.get_span_keywords()
    }

    fn get_event_keywords(&self) -> u64 {
        self.kwl.get_event_keywords()
    }

    fn get_links_keywords(&self) -> u64 {
        self.kwl.get_links_keywords()
    }
}

impl<T: KeywordLevelProvider> ExporterConfig<T> {
    pub(crate) fn get_export_as_json(&self) -> bool {
        self.json
    }

    pub(crate) fn get_export_common_schema_event(&self) -> bool {
        self.common_schema
    }

    pub(crate) fn get_export_span_events(&self) -> bool {
        self.etw_activities
    }
}

pub trait EtwSpan {
    fn get_span_data(&self) -> &opentelemetry_sdk::export::trace::SpanData;
}

#[doc(hidden)]
pub trait EventExporter {
    fn enabled(&self, level: u8, keyword: u64) -> bool;

    // Called by the real-time exporter when a span is started
    fn log_span_start<C, S>(
        &self,
        provider: &ExporterConfig<C>,
        span: &S,
    ) -> opentelemetry_sdk::export::trace::ExportResult
    where
        C: KeywordLevelProvider,
        S: opentelemetry_api::trace::Span + EtwSpan;

    // Called by the real-time exporter when a span is ended
    fn log_span_end<C, S>(
        &self,
        provider: &ExporterConfig<C>,
        span: &S,
    ) -> opentelemetry_sdk::export::trace::ExportResult
    where
        C: KeywordLevelProvider,
        S: opentelemetry_api::trace::Span + EtwSpan;

    // Called by the real-time exporter when an event is added to a span
    fn log_span_event<C, S>(
        &self,
        provider: &ExporterConfig<C>,
        event: opentelemetry_api::trace::Event,
        span: &S,
    ) -> opentelemetry_sdk::export::trace::ExportResult
    where
        C: KeywordLevelProvider,
        S: opentelemetry_api::trace::Span + EtwSpan;

    // Called by the batch exporter sometime after span is completed
    fn log_span_data<C>(
        &self,
        provider: &ExporterConfig<C>,
        span_data: &opentelemetry_sdk::export::trace::SpanData,
    ) -> opentelemetry_sdk::export::trace::ExportResult
    where
        C: KeywordLevelProvider;
}

pub(crate) struct Activities {
    pub(crate) span_id: [u8; 16],                    // Hex string
    pub(crate) activity_id: [u8; 16],                // Guid
    pub(crate) parent_activity_id: Option<[u8; 16]>, // Guid
    pub(crate) parent_span_id: [u8; 16],             // Hex string
    pub(crate) trace_id_name: [u8; 32],              // Hex string
}

impl Activities {
    #[allow(invalid_value)]
    pub(crate) fn generate(
        span_id: &SpanId,
        parent_span_id: &SpanId,
        trace_id: &TraceId,
    ) -> Activities {
        let mut activity_id: [u8; 16] = [0; 16];
        let (_, half) = activity_id.split_at_mut(8);
        half.copy_from_slice(&span_id.to_bytes());

        let (parent_activity_id, parent_span_name) = if *parent_span_id == SpanId::INVALID {
            (None, [0; 16])
        } else {
            let mut buf: [u8; 16] = unsafe { MaybeUninit::uninit().assume_init() };
            let mut cur = Cursor::new(&mut buf[..]);
            write!(&mut cur, "{:16x}", span_id).expect("!write");

            let mut activity_id: [u8; 16] = [0; 16];
            let (_, half) = activity_id.split_at_mut(8);
            half.copy_from_slice(&parent_span_id.to_bytes());
            (Some(activity_id), buf)
        };

        let mut buf: [u8; 16] = unsafe { MaybeUninit::uninit().assume_init() };
        let mut cur = Cursor::new(&mut buf[..]);
        write!(&mut cur, "{:16x}", span_id).expect("!write");

        let mut buf2: [u8; 32] = unsafe { MaybeUninit::uninit().assume_init() };
        let mut cur2 = Cursor::new(&mut buf2[..]);
        write!(&mut cur2, "{:32x}", trace_id).expect("!write");

        Activities {
            span_id: buf,
            activity_id,
            parent_activity_id,
            parent_span_id: parent_span_name,
            trace_id_name: buf2,
        }
    }
}
