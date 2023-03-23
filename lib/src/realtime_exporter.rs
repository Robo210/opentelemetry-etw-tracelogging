use crate::constants::*;
use crate::etw_exporter::*;
use opentelemetry::trace::TraceContextExt;
use opentelemetry::trace::{
    Event, SpanBuilder, SpanContext, SpanId, SpanKind, TraceFlags, TraceState,
};
use opentelemetry::Context;
use opentelemetry_api::trace::SpanRef;
use opentelemetry_sdk::{
    export::trace::SpanData,
    trace::{EvictedHashMap, EvictedQueue},
};
use std::borrow::Cow;
use std::pin::Pin;
use std::sync::Mutex;
use std::sync::{Arc, Weak, atomic::*};
use std::time::SystemTime;
use tracelogging_dynamic::*;

struct ExporterConfig {
    provider: Pin<Box<Provider>>,
    span_keywords: u64,
    event_keywords: u64,
    links_keywords: u64,
    bool_intype: InType,
    json: bool,
    config: opentelemetry_sdk::trace::Config,
}

impl EtwExporter for ExporterConfig {
    fn get_provider(&self) -> Pin<&Provider> {
        self.provider.as_ref()
    }

    fn get_span_keywords(&self) -> u64 {
        self.span_keywords
    }

    fn get_event_keywords(&self) -> u64 {
        self.event_keywords
    }

    fn get_links_keywords(&self) -> u64 {
        self.links_keywords
    }

    fn get_bool_representation(&self) -> InType {
        self.bool_intype
    }

    fn get_export_as_json(&self) -> bool {
        self.json
    }
}

pub struct RealtimeSpan {
    ebw: Mutex<EventBuilderWrapper>,
    etw_config: Weak<ExporterConfig>,
    span_data: SpanData,
    ended: AtomicBool,
}

impl RealtimeSpan {
    fn build(
        builder: SpanBuilder,
        etw_config: Weak<ExporterConfig>,
        parent_span: Option<SpanRef>,
    ) -> Self {
        let parent_span_id =
            parent_span.map_or_else(|| SpanId::INVALID, |s| s.span_context().span_id());
        let strong = etw_config.upgrade();
        let otel_config = if let Some(config) = &strong {
            &config.as_ref().config
        } else {
            panic!()
        };

        RealtimeSpan {
            ebw: Mutex::new(EventBuilderWrapper::new()),
            etw_config,
            span_data: SpanData {
                span_context: SpanContext::new(
                    builder
                        .trace_id
                        .unwrap_or(otel_config.id_generator.new_trace_id()),
                    builder
                        .span_id
                        .unwrap_or(otel_config.id_generator.new_span_id()),
                    TraceFlags::SAMPLED,
                    false,
                    TraceState::default(),
                ),
                parent_span_id,
                span_kind: builder.span_kind.unwrap_or(SpanKind::Internal),
                name: builder.name,
                start_time: builder.start_time.unwrap_or(SystemTime::UNIX_EPOCH),
                end_time: builder.end_time.unwrap_or(SystemTime::UNIX_EPOCH),
                attributes: EvictedHashMap::new(otel_config.span_limits.max_attributes_per_span, 5), // TODO - insert data
                events: EvictedQueue::new(otel_config.span_limits.max_events_per_span),
                links: EvictedQueue::new(otel_config.span_limits.max_links_per_span),
                status: builder.status,
                resource: Cow::default(),                // TODO
                instrumentation_lib: Default::default(), // TODO
            },
            ended: AtomicBool::new(false),
        }
    }

    fn start(&mut self) {
        self.span_data.start_time = SystemTime::now();
        self.span_data.end_time = self.span_data.start_time; // The spec requires this, even though it doesn't make sense.

        let mut strong = self.etw_config.upgrade();
        if let Some(prov) = strong.as_mut() {
            if let Ok(mut ebw) = self.ebw.lock() {
                let _ = ebw.log_span_start(prov.as_ref(), self);
            }
        }
    }
}

impl opentelemetry_api::trace::Span for RealtimeSpan {
    fn add_event_with_timestamp<T>(
        &mut self,
        name: T,
        timestamp: std::time::SystemTime,
        attributes: Vec<opentelemetry::KeyValue>,
    ) where
        T: Into<std::borrow::Cow<'static, str>>,
    {
        let event = Event::new(name, timestamp, attributes, 0);

        let mut strong = self.etw_config.upgrade();
        if let Some(prov) = strong.as_mut() {
            if let Ok(mut ebw) = self.ebw.lock() {
                let _ = ebw.log_span_event(prov.as_ref(), event, self);
            }
        }
    }

    fn end_with_timestamp(&mut self, timestamp: std::time::SystemTime) {
        self.span_data.end_time = timestamp;

        // Not really sure why we bother using an atomic for ended but just blindly assign the end time...
        let ended = self.ended.compare_exchange(false, true, Ordering::Acquire, Ordering::Acquire);

        let _ = ended.and_then(|_| {
            let mut strong = self.etw_config.upgrade();
            if let Some(config) = strong.as_mut() {
                if let Ok(mut ebw) = self.ebw.lock() {
                    let _ = ebw.log_span_end(config.as_ref(), self);
                }
            }

            Ok(())
        });
    }

    fn is_recording(&self) -> bool {
        let strong = self.etw_config.upgrade();
        if let Some(config) = strong {
            config
                .provider
                .enabled(Level::Informational, config.span_keywords)
        } else {
            false
        }
    }

    fn set_attribute(&mut self, attribute: opentelemetry::KeyValue) {
        self.span_data.attributes.insert(attribute);
    }

    fn set_status(&mut self, status: opentelemetry::trace::Status) {
        self.span_data.status = status;
    }

    fn span_context(&self) -> &opentelemetry::trace::SpanContext {
        &self.span_data.span_context
    }

    fn update_name<T>(&mut self, new_name: T)
    where
        T: Into<std::borrow::Cow<'static, str>>,
    {
        self.span_data.name = new_name.into();
    }
}

impl Drop for RealtimeSpan {
    fn drop(&mut self) {
        <Self as opentelemetry_api::trace::Span>::end(self);
    }
}

impl EtwSpan for RealtimeSpan {
    fn get_span_data(&self) -> &SpanData {
        &self.span_data
    }
}

pub struct RealtimeTracer {
    etw_config: Weak<ExporterConfig>,
}

impl RealtimeTracer {
    fn new(
        etw_config: Weak<ExporterConfig>,
        lib: opentelemetry_api::InstrumentationLibrary,
    ) -> Self {
        RealtimeTracer {
            etw_config: etw_config,
        }
    }
}

impl opentelemetry_api::trace::Tracer for RealtimeTracer {
    type Span = RealtimeSpan;

    fn build_with_context(&self, builder: SpanBuilder, parent_cx: &Context) -> Self::Span {
        let parent_span = if parent_cx.has_active_span() {
            Some(parent_cx.span())
        } else {
            None
        };

        let mut span = RealtimeSpan::build(builder, self.etw_config.clone(), parent_span);
        span.start();
        span
    }
}

pub struct RealtimeTracerProvider {
    etw_config: Arc<ExporterConfig>,
}

impl RealtimeTracerProvider {
    pub(crate) fn new(
        provider_name: &str,
        config: opentelemetry_sdk::trace::Config,
        use_byte_for_bools: bool,
        export_payload_as_json: bool,
    ) -> Self {
        let mut provider = Box::pin(Provider::new());
        unsafe {
            provider
                .as_mut()
                .register(provider_name, Provider::options().group_id(&GROUP_ID));
        }

        let etw_config = Arc::new(ExporterConfig {
            provider,
            span_keywords: 1,
            event_keywords: 2,
            links_keywords: 4,
            bool_intype: if use_byte_for_bools {
                InType::U8
            } else {
                InType::Bool32
            },
            json: export_payload_as_json,
            config: config,
        });

        RealtimeTracerProvider { etw_config }
    }
}

impl opentelemetry_api::trace::TracerProvider for RealtimeTracerProvider {
    type Tracer = RealtimeTracer;

    fn tracer(&self, name: impl Into<std::borrow::Cow<'static, str>>) -> Self::Tracer {
        self.versioned_tracer(name, Some("1.0"), Some("https://microsoft.com/etw"))
    }

    fn versioned_tracer(
        &self,
        name: impl Into<std::borrow::Cow<'static, str>>,
        version: Option<&'static str>,
        schema_url: Option<&'static str>,
    ) -> Self::Tracer {
        let name = name.into();
        // Use default value if name is invalid empty string
        let component_name = if name.is_empty() {
            Cow::Borrowed("DEFAULT_COMPONENT_NAME") // TODO
        } else {
            name
        };
        let instrumentation_lib = opentelemetry_api::InstrumentationLibrary::new(
            component_name,
            version.map(Into::into),
            schema_url.map(Into::into),
        );

        RealtimeTracer::new(Arc::downgrade(&self.etw_config), instrumentation_lib)
    }
}

// pub struct RealtimeExporter {
//     // Must be boxed because SpanProcessor doesn't use mutable self,
//     // but EtwExporter and EventBuilder must be mutable.
//     config: Mutex<ExporterConfig>,
//     ebw: Mutex<EventBuilderWrapper>,
// }

// impl RealtimeExporter {
//     pub(crate) fn new(
//         provider_name: &str,
//         use_byte_for_bools: bool,
//         export_payload_as_json: bool,
//     ) -> Self {
//         let mut provider = Box::pin(Provider::new());
//         unsafe {
//             provider
//                 .as_mut()
//                 .register(provider_name, Provider::options().group_id(&GROUP_ID));
//         }
//         RealtimeExporter {
//             config: Mutex::new(ExporterConfig {
//                 provider,
//                 span_keywords: 1,
//                 event_keywords: 2,
//                 links_keywords: 4,
//                 bool_intype: if use_byte_for_bools {
//                     InType::U8
//                 } else {
//                     InType::Bool32
//                 },
//                 json: export_payload_as_json,
//             }),
//             ebw: Mutex::new(EventBuilderWrapper::new()),
//         }
//     }
// }

// impl Debug for RealtimeExporter {
//     fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         todo!()
//     }
// }

// impl SpanProcessor for RealtimeExporter {
//     fn on_start(&self, span: &mut opentelemetry_sdk::trace::Span, _cx: &Context) {
//         let mut config = self.config.lock().unwrap();
//         let _ = self.ebw.lock().unwrap().log_span_start(&mut *config, span);
//     }

//     fn on_end(&self, span: SpanData) {
//         let mut config = self.config.lock().unwrap();
//         let _ = self.ebw.lock().unwrap().log_span_end(&mut *config, &span);
//     }

//     fn force_flush(&self) -> TraceResult<()> {
//         Ok(())
//     }

//     fn shutdown(&mut self) -> TraceResult<()> {
//         Ok(())
//     }
// }

// #[allow(unused_imports)]
// mod tests {
//     use super::*;

//     #[test]
//     fn create_realtime_exporter() {
//         let _ = RealtimeExporter::new("my_provider_name", false, false);
//     }
// }
