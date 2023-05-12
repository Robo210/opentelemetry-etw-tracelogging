use crate::spans::builder::ProviderGroup;
#[allow(unused_imports)]
use crate::etw;
use crate::exporter_traits::*;
use crate::common::EtwSpan;
#[allow(unused_imports)]
use crate::user_events;
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
#[allow(unused_imports)]
use tracelogging_dynamic::*;

pub struct RealtimeSpan<E: EventExporter> {
    event_exporter: Weak<E>,
    span_data: SpanData,
    ended: AtomicBool,
}

impl<E: EventExporter> RealtimeSpan<E> {
    fn build(
        builder: SpanBuilder,
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
                resource: otel_config.resource.clone(), // TODO: This clone is really inefficient
                instrumentation_lib,                    // This is never used
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

        if let Some(event_exporter) = self.event_exporter.upgrade() {
            let _ = event_exporter.log_span_start(self);
        }
    }
}

impl<E: EventExporter> opentelemetry_api::trace::Span for RealtimeSpan<E> {
    fn add_event_with_timestamp<N>(
        &mut self,
        name: N,
        timestamp: std::time::SystemTime,
        attributes: Vec<opentelemetry::KeyValue>,
    ) where
        N: Into<std::borrow::Cow<'static, str>>,
    {
        let event = Event::new(name, timestamp, attributes, 0);

        if let Some(event_exporter) = self.event_exporter.upgrade() {
            let _ = event_exporter.log_span_event(event, self);
        }
    }

    fn end_with_timestamp(&mut self, timestamp: std::time::SystemTime) {
        self.span_data.end_time = timestamp;

        // Not really sure why we bother using an atomic for ended but just blindly assign the end time...
        let already_ended = self.ended.swap(true, Ordering::Acquire);

        if !already_ended {
            if let Some(event_exporter) = self.event_exporter.upgrade() {
                let _ = event_exporter.log_span_end(self);
            }
        }
    }

    fn is_recording(&self) -> bool {
        if let Some(_event_exporter) = self.event_exporter.upgrade() {
            // TODO: We want to know if anything is enabled at all
            //event_exporter.enabled(Level::Informational.as_int(), config.get_span_keywords())
            true
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

impl<E: EventExporter> Drop for RealtimeSpan<E> {
    fn drop(&mut self) {
        <Self as opentelemetry_api::trace::Span>::end(self);
    }
}

impl<E: EventExporter> EtwSpan for RealtimeSpan<E> {
    fn get_span_data(&self) -> &SpanData {
        &self.span_data
    }
}

pub struct RealtimeTracer<E: EventExporter> {
    otel_config: Weak<opentelemetry_sdk::trace::Config>,
    event_exporter: Weak<E>,
    instrumentation_lib: InstrumentationLibrary,
}

impl<E: EventExporter> RealtimeTracer<E> {
    fn new(
        otel_config: Weak<opentelemetry_sdk::trace::Config>,
        event_exporter: Weak<E>,
        instrumentation_lib: InstrumentationLibrary,
    ) -> Self {
        RealtimeTracer {
            otel_config,
            event_exporter,
            instrumentation_lib,
        }
    }
}

impl<E: EventExporter> opentelemetry_api::trace::Tracer for RealtimeTracer<E> {
    type Span = RealtimeSpan<E>;

    fn build_with_context(&self, builder: SpanBuilder, parent_cx: &Context) -> Self::Span {
        let parent_span = if parent_cx.has_active_span() {
            Some(parent_cx.span())
        } else {
            None
        };

        let mut span = RealtimeSpan::build(
            builder,
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
    otel_config: Arc<opentelemetry_sdk::trace::Config>,
    event_exporter: Arc<E>,
    _x: core::marker::PhantomData<C>,
}

#[cfg(all(target_os = "windows"))]
impl<C: KeywordLevelProvider> RealtimeTracerProvider<C, etw::EtwEventExporter<C>> {
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

        let provider = Arc::pin(Provider::new(provider_name, &options));
        unsafe {
            provider.as_ref().register();
        }

        RealtimeTracerProvider {
            otel_config: Arc::new(otel_config),
            event_exporter: Arc::new(etw::EtwEventExporter::new(
                provider,
                exporter_config,
                if use_byte_for_bools {
                    InType::U8
                } else {
                    InType::Bool32
                },
            )),
            _x: core::marker::PhantomData,
        }
    }
}

#[cfg(all(target_os = "linux"))]
impl<C: KeywordLevelProvider> RealtimeTracerProvider<C, user_events::UserEventsExporter<C>> {
    pub(crate) fn new(
        provider_name: &str,
        provider_group: ProviderGroup,
        otel_config: opentelemetry_sdk::trace::Config,
        _use_byte_for_bools: bool,
        exporter_config: ExporterConfig<C>,
    ) -> Self {
        let mut options = eventheader_dynamic::Provider::new_options();
        if let ProviderGroup::Linux(ref name) = provider_group {
            options = *options.group_name(&name);
        }
        let mut provider = eventheader_dynamic::Provider::new(provider_name, &options);
        user_events::register_eventsets(&mut provider, &exporter_config);

        RealtimeTracerProvider {
            otel_config: Arc::new(otel_config),
            event_exporter: Arc::new(user_events::UserEventsExporter::new(Arc::new(provider), exporter_config)),
            _x: core::marker::PhantomData,
        }
    }
}

impl<C: KeywordLevelProvider, E: EventExporter> opentelemetry_api::trace::TracerProvider
    for RealtimeTracerProvider<C, E>
{
    type Tracer = RealtimeTracer<E>;

    fn tracer(&self, name: impl Into<std::borrow::Cow<'static, str>>) -> Self::Tracer {
        self.versioned_tracer(
            name,
            Some(env!("CARGO_PKG_VERSION")),
            Some("https://microsoft.com/etw"),
        )
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
            Cow::Borrowed("opentelemetry-etw-user_events")
        } else {
            name
        };
        let instrumentation_lib = InstrumentationLibrary::new(
            component_name,
            version.map(Into::into),
            schema_url.map(Into::into),
        );

        RealtimeTracer::new(
            Arc::downgrade(&self.otel_config),
            Arc::downgrade(&self.event_exporter),
            instrumentation_lib,
        )
    }
}
