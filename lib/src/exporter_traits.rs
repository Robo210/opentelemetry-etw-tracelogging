use std::{
    io::{Cursor, Write},
    mem::MaybeUninit,
};

use opentelemetry::trace::{SpanId, TraceId};

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

pub trait EtwSpan {
    fn get_span_data(&self) -> &opentelemetry_sdk::export::trace::SpanData;
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
