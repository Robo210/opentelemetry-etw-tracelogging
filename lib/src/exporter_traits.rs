use std::{pin::Pin, sync::Arc};

use opentelemetry::trace::{SpanId, TraceId};

pub enum ProviderWrapper {
    //fn enabled(&self, level: u8, keyword: u64) -> bool;
    Etw(Pin<Arc<tracelogging_dynamic::Provider>>),
    UserEvents(Arc<linux_tld::Provider>),
}

impl ProviderWrapper {
    pub(crate) fn enabled(&self, level: u8, keyword: u64) -> bool {
        match self {
            ProviderWrapper::Etw(p) => p.enabled(level.into(), keyword),
            ProviderWrapper::UserEvents(p) => {
                let es = p.find_set(level.into(), keyword);
                if let Some(es) = es {
                    es.enabled()
                } else {
                    false
                }
            }
        }
    }
}

pub trait ExporterConfig {
    fn get_provider(&self) -> ProviderWrapper;
    fn get_span_keywords(&self) -> u64;
    fn get_event_keywords(&self) -> u64;
    fn get_links_keywords(&self) -> u64;
    fn get_export_as_json(&self) -> bool;
    fn get_export_common_schema_event(&self) -> bool;
    fn get_export_span_events(&self) -> bool;
}

pub trait EtwSpan {
    fn get_span_data(&self) -> &opentelemetry_sdk::export::trace::SpanData;
}

pub trait EventExporter {
    // Called by the real-time exporter when a span is started
    fn log_span_start<C, S>(
        &self,
        provider: &C,
        span: &S,
    ) -> opentelemetry_sdk::export::trace::ExportResult
    where
        C: ExporterConfig,
        S: opentelemetry_api::trace::Span + EtwSpan;

    // Called by the real-time exporter when a span is ended
    fn log_span_end<C, S>(&self, provider: &C, span: &S) -> opentelemetry_sdk::export::trace::ExportResult
    where
        C: ExporterConfig,
        S: opentelemetry_api::trace::Span + EtwSpan;

    // Called by the real-time exporter when an event is added to a span
    fn log_span_event<C, S>(
        &self,
        provider: &C,
        event: opentelemetry_api::trace::Event,
        span: &S,
    ) -> opentelemetry_sdk::export::trace::ExportResult
    where
        C: ExporterConfig,
        S: opentelemetry_api::trace::Span + EtwSpan;

    // Called by the batch exporter sometime after span is completed
    fn log_span_data<C>(
        &self,
        provider: &C,
        span_data: &opentelemetry_sdk::export::trace::SpanData,
    ) -> opentelemetry_sdk::export::trace::ExportResult
    where
        C: ExporterConfig;
}

pub(crate) struct Activities {
    pub(crate) span_id: String,
    pub(crate) activity_id: [u8; 16],
    pub(crate) parent_activity_id: Option<[u8; 16]>,
    pub(crate) parent_span_id: String,
    pub(crate) trace_id_name: String,
}

impl Activities {
    pub(crate) fn generate(span_id: &SpanId, parent_span_id: &SpanId, trace_id: &TraceId) -> Activities {
        let name = span_id.to_string();
        let activity_id = tracelogging::Guid::from_name(&name);
        let (parent_activity_id, parent_span_name) = if *parent_span_id == SpanId::INVALID {
            (None, String::default())
        } else {
            let parent_span_name = parent_span_id.to_string();
            (Some(tracelogging::Guid::from_name(&parent_span_name).to_bytes_be()), parent_span_name)
        };

        Activities {
            span_id: name,
            activity_id: activity_id.to_bytes_be(),
            parent_activity_id,
            parent_span_id: parent_span_name,
            trace_id_name: trace_id.to_string(),
        }
    }
}
