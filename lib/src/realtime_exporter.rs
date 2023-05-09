#[allow(unused_imports)]
use crate::etw_exporter::*;
use crate::exporter_traits::*;
use crate::builder::ProviderGroup;
#[allow(unused_imports)]
use crate::user_events_exporter::*;
use opentelemetry::InstrumentationLibrary;
use opentelemetry::{
    trace::{
        Event, SpanBuilder, SpanContext, SpanId, SpanKind, TraceContextExt, TraceFlags, TraceState,
    },
    Context,
};
use opentelemetry_api::trace::SpanRef;
use opentelemetry_sdk::{
    export::trace::SpanData,
    trace::{EvictedHashMap, EvictedQueue},
};
use std::borrow::Cow;
use std::sync::{atomic::*, Arc, Weak};
use std::time::SystemTime;
use tracelogging_dynamic::*;

pub struct RealtimeSpan<C: KeywordLevelProvider, E: EventExporter> {
    exporter_config: Weak<ExporterConfig<C>>,
    event_exporter: Weak<E>,
    span_data: SpanData,
    ended: AtomicBool,
}

impl<C: KeywordLevelProvider, E: EventExporter> RealtimeSpan<C, E> {
    fn build(
        builder: SpanBuilder,
        exporter_config: Weak<ExporterConfig<C>>,
        otel_config: Weak<opentelemetry_sdk::trace::Config>,
        event_exporter: Weak<E>,
        parent_span: Option<SpanRef>,
        instrumentation_lib: InstrumentationLibrary,
    ) -> Self {
        let parent_span_id =
            parent_span.map_or_else(|| SpanId::INVALID, |s| s.span_context().span_id());
        let strong = otel_config.upgrade();
        let otel_config = if let Some(config) = &strong {
            config.as_ref()
        } else {
            panic!()
        };

        let attributes = builder.attributes.unwrap_or_default().into_iter();

        let mut span = RealtimeSpan {
            exporter_config,
            event_exporter,
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
                attributes: EvictedHashMap::new(
                    otel_config.span_limits.max_attributes_per_span,
                    attributes.len(),
                ),
                events: EvictedQueue::new(otel_config.span_limits.max_events_per_span),
                links: EvictedQueue::new(otel_config.span_limits.max_links_per_span),
                status: builder.status,
                resource: otel_config.resource.clone(), // TODO: This is really inefficient
                instrumentation_lib, // TODO: Currently this is never used, so making all the copies of it is wasteful
            },
            ended: AtomicBool::new(false),
        };

        for attribute in attributes {
            span.span_data
                .attributes
                .insert(opentelemetry_api::KeyValue {
                    key: attribute.0,
                    value: attribute.1,
                });
        }

        span.span_data
            .events
            .extend(builder.events.unwrap_or_default().into_iter());
        span.span_data
            .links
            .extend(builder.links.unwrap_or_default().into_iter());

        span
    }

    fn start(&mut self) {
        self.span_data.start_time = SystemTime::now();
        self.span_data.end_time = self.span_data.start_time; // The spec requires this, even though it doesn't make sense.

        let mut strong = self.exporter_config.upgrade();
        if let Some(prov) = strong.as_mut() {
            if let Some(event_exporter) = self.event_exporter.upgrade() {
                let _ = event_exporter.log_span_start(prov.as_ref(), self);
            }
        }
    }
}

impl<C: KeywordLevelProvider, E: EventExporter> opentelemetry_api::trace::Span for RealtimeSpan<C, E> {
    fn add_event_with_timestamp<N>(
        &mut self,
        name: N,
        timestamp: std::time::SystemTime,
        attributes: Vec<opentelemetry::KeyValue>,
    ) where
        N: Into<std::borrow::Cow<'static, str>>,
    {
        let event = Event::new(name, timestamp, attributes, 0);

        let mut strong = self.exporter_config.upgrade();
        if let Some(prov) = strong.as_mut() {
            if let Some(event_exporter) = self.event_exporter.upgrade() {
                let _ = event_exporter.log_span_event(prov.as_ref(), event, self);
            }
        }
    }

    fn end_with_timestamp(&mut self, timestamp: std::time::SystemTime) {
        self.span_data.end_time = timestamp;

        // Not really sure why we bother using an atomic for ended but just blindly assign the end time...
        let already_ended = self.ended.swap(true, Ordering::Acquire);

        if !already_ended {
            if let Some(config) = self.exporter_config.upgrade() {
                if let Some(event_exporter) = self.event_exporter.upgrade() {
                    let _ = event_exporter.log_span_end(config.as_ref(), self);
                }
            }
        }
    }

    fn is_recording(&self) -> bool {
        if let Some(config) = self.exporter_config.upgrade() {
            if let Some(event_exporter) = self.event_exporter.upgrade() {
                event_exporter.enabled(Level::Informational.as_int(), config.get_span_keywords())
            } else {
                false
            }
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

    fn update_name<N>(&mut self, new_name: N)
    where
        N: Into<std::borrow::Cow<'static, str>>,
    {
        self.span_data.name = new_name.into();
    }
}

impl<C: KeywordLevelProvider, E: EventExporter> Drop for RealtimeSpan<C, E> {
    fn drop(&mut self) {
        <Self as opentelemetry_api::trace::Span>::end(self);
    }
}

impl<C: KeywordLevelProvider, E: EventExporter> EtwSpan for RealtimeSpan<C, E> {
    fn get_span_data(&self) -> &SpanData {
        &self.span_data
    }
}

pub struct RealtimeTracer<C: KeywordLevelProvider, E: EventExporter> {
    exporter_config: Weak<ExporterConfig<C>>,
    otel_config: Weak<opentelemetry_sdk::trace::Config>,
    event_exporter: Weak<E>,
    instrumentation_lib: InstrumentationLibrary,
}

impl<C: KeywordLevelProvider, E: EventExporter> RealtimeTracer<C, E> {
    fn new(
        exporter_config: Weak<ExporterConfig<C>>,
        otel_config: Weak<opentelemetry_sdk::trace::Config>,
        event_exporter: Weak<E>,
        instrumentation_lib: InstrumentationLibrary,
    ) -> Self {
        RealtimeTracer {
            exporter_config,
            otel_config,
            event_exporter,
            instrumentation_lib,
        }
    }
}

impl<C: KeywordLevelProvider, E: EventExporter> opentelemetry_api::trace::Tracer
    for RealtimeTracer<C, E>
{
    type Span = RealtimeSpan<C, E>;

    fn build_with_context(&self, builder: SpanBuilder, parent_cx: &Context) -> Self::Span {
        let parent_span = if parent_cx.has_active_span() {
            Some(parent_cx.span())
        } else {
            None
        };

        let mut span = RealtimeSpan::build(
            builder,
            self.exporter_config.clone(),
            self.otel_config.clone(),
            self.event_exporter.clone(),
            parent_span,
            self.instrumentation_lib.clone(),
        );
        span.start();
        span
    }
}

pub struct RealtimeTracerProvider<C: KeywordLevelProvider, E: EventExporter> {
    exporter_config: Arc<ExporterConfig<C>>,
    otel_config: Arc<opentelemetry_sdk::trace::Config>,
    event_exporter: Arc<E>,
}

#[cfg(all(target_os = "windows"))]
impl<C: KeywordLevelProvider> RealtimeTracerProvider<C, EtwEventExporter> {
    pub(crate) fn new(
        provider_name: &str,
        provider_group: ProviderGroup,
        otel_config: opentelemetry_sdk::trace::Config,
        use_byte_for_bools: bool,
        exporter_config: ExporterConfig<C>,
    ) -> Self {
        let mut options = Provider::options();
        if let ProviderGroup::Windows(guid) = provider_group {
            options = *options.group_id(&guid);
        }

        let provider = Arc::pin(Provider::new(
            provider_name,
            &options,
        ));
        unsafe {
            provider.as_ref().register();
        }

        let exporter_config = Arc::new(exporter_config);

        RealtimeTracerProvider {
            exporter_config,
            otel_config: Arc::new(otel_config),
            event_exporter: Arc::new(EtwEventExporter::new(
                provider,
                if use_byte_for_bools {
                    InType::U8
                } else {
                    InType::Bool32
                },
            )),
        }
    }
}

#[cfg(all(target_os = "linux"))]
impl<C: ExporterConfig + KeywordLevelProvider> RealtimeTracerProvider<C, UserEventsExporter> {
    pub(crate) fn new(
        provider_name: &str,
        provider_group: ProviderGroup,
        otel_config: opentelemetry_sdk::trace::Config,
        _use_byte_for_bools: bool,
        exporter_config: C,
    ) -> Self {
        let mut options = linux_tld::Provider::options();
        if let ProviderGroup::Linux(ref name) = provider_group {
            options = *options.group_name(&name);
        }
        let mut provider = linux_tld::Provider::new(
            provider_name,
            &options,
        );

        #[cfg(not(test))]
        {
            // Standard real-time level/keyword pairs
            provider.register_set(linux_tlg::Level::Informational, exporter_config.get_span_keywords());
            provider.register_set(linux_tlg::Level::Verbose, exporter_config.get_event_keywords());
            provider.register_set(linux_tlg::Level::Verbose, exporter_config.get_links_keywords());

            // Common Schema events use a level based on a span's Status
            provider.register_set(linux_tlg::Level::Error, exporter_config.get_span_keywords());
            provider.register_set(linux_tlg::Level::Verbose, exporter_config.get_span_keywords());
        }
        #[cfg(test)]
        {
            // Standard real-time level/keyword pairs
            provider.create_unregistered(true, linux_tlg::Level::Informational, exporter_config.get_span_keywords());
            provider.create_unregistered(true, linux_tlg::Level::Verbose, exporter_config.get_event_keywords());
            provider.create_unregistered(true, linux_tlg::Level::Verbose, exporter_config.get_links_keywords());

            // Common Schema events use a level based on a span's Status
            provider.create_unregistered(true, linux_tlg::Level::Error, exporter_config.get_span_keywords());
            provider.create_unregistered(true, linux_tlg::Level::Verbose, exporter_config.get_span_keywords());
        }

        let exporter_config = Arc::new(exporter_config);

        RealtimeTracerProvider {
            exporter_config,
            otel_config: Arc::new(otel_config),
            event_exporter: Arc::new(UserEventsExporter::new(Arc::new(provider))),
        }
    }
}

impl<C: KeywordLevelProvider, E: EventExporter> opentelemetry_api::trace::TracerProvider
    for RealtimeTracerProvider<C, E>
{
    type Tracer = RealtimeTracer<C, E>;

    fn tracer(&self, name: impl Into<std::borrow::Cow<'static, str>>) -> Self::Tracer {
        self.versioned_tracer(name, Some("4.0"), Some("https://microsoft.com/etw"))
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
        let instrumentation_lib = InstrumentationLibrary::new(
            component_name,
            version.map(Into::into),
            schema_url.map(Into::into),
        );

        RealtimeTracer::new(
            Arc::downgrade(&self.exporter_config),
            Arc::downgrade(&self.otel_config),
            Arc::downgrade(&self.event_exporter),
            instrumentation_lib,
        )
    }
}
