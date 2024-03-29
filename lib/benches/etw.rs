#![allow(unused_imports, dead_code)]

#[path = "../src/etw.rs"]
mod etw;
#[path = "../src/exporter_traits.rs"]
mod exporter_traits;
#[path = "../src/common/mod.rs"]
mod common;

use crate::exporter_traits::*;
use criterion::{criterion_group, criterion_main, Criterion};
use etw::EtwEventExporter;
use etw_helpers::*;
use opentelemetry::trace::{SpanContext, SpanId, SpanKind, TraceFlags, TraceState};
use opentelemetry::InstrumentationLibrary;
use opentelemetry_sdk::{
    export::trace::SpanData,
    trace::{EvictedHashMap, EvictedQueue},
};
use rsevents::Awaitable;
use std::borrow::Cow;
use std::sync::Arc;
use std::time::SystemTime;

struct BenchExporterConfig;

impl KeywordLevelProvider for BenchExporterConfig {
    fn get_span_keywords(&self) -> u64 {
        1
    }
    fn get_event_keywords(&self) -> u64 {
        2
    }
    fn get_links_keywords(&self) -> u64 {
        4
    }
    fn get_span_level(&self) -> u8 {
        4 // Level::Informational
    }
    fn get_event_level(&self) -> u8 {
        5 // Level::Verbose
    }
    fn get_links_level(&self) -> u8 {
        5 // Level::Verbose
    }
}

fn provider_enabled_callback(
    _source_id: &tracelogging::Guid,
    _event_control_code: u32,
    _level: tracelogging::Level,
    _match_any_keyword: u64,
    _match_all_keyword: u64,
    _filter_data: usize,
    callback_context: usize,
) {
    unsafe {
        let ctx =
            &*(callback_context as *const std::ffi::c_void as *const rsevents::ManualResetEvent);
        ctx.set();
    }
}

static BENCH_PROVIDER_ENABLED_EVENT: rsevents::ManualResetEvent =
    rsevents::ManualResetEvent::new(rsevents::EventState::Unset);

#[cfg(all(target_os = "windows"))]
pub fn etw_benchmark(c: &mut Criterion) {
    let mut options = tracelogging_dynamic::Provider::options();
    let options = options.callback(
        provider_enabled_callback,
        &BENCH_PROVIDER_ENABLED_EVENT as *const rsevents::ManualResetEvent as usize,
    );

    let provider = Arc::pin(tracelogging_dynamic::Provider::new("otel-bench", &options));
    unsafe {
        provider.as_ref().register();
    }
    let provider_id = provider.id().clone();

    let instrumentation_lib = InstrumentationLibrary::new(Cow::Borrowed("bench"), None, None);

    let otel_config = opentelemetry_sdk::trace::config();

    let span_data = SpanData {
        span_context: SpanContext::new(
            otel_config.id_generator.new_trace_id(),
            otel_config.id_generator.new_span_id(),
            TraceFlags::SAMPLED,
            false,
            TraceState::default(),
        ),
        parent_span_id: SpanId::INVALID,
        span_kind: SpanKind::Internal,
        name: Cow::Borrowed("bench span"),
        start_time: SystemTime::UNIX_EPOCH,
        end_time: SystemTime::UNIX_EPOCH,
        attributes: EvictedHashMap::new(otel_config.span_limits.max_attributes_per_span, 1),
        events: EvictedQueue::new(otel_config.span_limits.max_events_per_span),
        links: EvictedQueue::new(otel_config.span_limits.max_links_per_span),
        status: opentelemetry::trace::Status::Ok,
        resource: otel_config.resource.clone(),
        instrumentation_lib,
    };

    let mut group = c.benchmark_group("export span_data");

    group.bench_function("provider disabled", |b| {
        let config = ExporterConfig {
            //kwl: Box::new(BenchExporterConfig) as Box::<dyn KeywordLevelProvider>, // Using a boxed trait object has a measurable performance impact
            kwl: BenchExporterConfig,
            json: false,
            common_schema: false,
            etw_activities: false,
        };
        let exporter =
            EtwEventExporter::new(provider.clone(), config, tracelogging::InType::Bool32);
        b.iter(|| (exporter.log_span_data(&span_data)))
    });

    let h = EtwSession::get_or_start_etw_session(windows::s!("otel-bench"), true)
        .expect("can't start etw session");

    h.enable_provider(&windows::core::GUID::from_u128(provider_id.to_u128()))
        .unwrap();

    BENCH_PROVIDER_ENABLED_EVENT.wait();

    group.bench_function("provider enabled/cs4", |b| {
        let config = ExporterConfig {
            //kwl: Box::new(BenchExporterConfig) as Box::<dyn KeywordLevelProvider>, // Using a boxed trait object has a measurable performance impact
            kwl: BenchExporterConfig,
            json: false,
            common_schema: true,
            etw_activities: false,
        };
        let exporter =
            EtwEventExporter::new(provider.clone(), config, tracelogging::InType::Bool32);
        b.iter(|| (exporter.log_span_data(&span_data)))
    });

    group.bench_function("provider enabled/span", |b| {
        let config = ExporterConfig {
            //kwl: Box::new(BenchExporterConfig) as Box::<dyn KeywordLevelProvider>, // Using a boxed trait object has a measurable performance impact
            kwl: BenchExporterConfig,
            json: false,
            common_schema: false,
            etw_activities: true,
        };
        let exporter =
            EtwEventExporter::new(provider.clone(), config, tracelogging::InType::Bool32);
        b.iter(|| (exporter.log_span_data(&span_data)))
    });

    group.bench_function("provider enabled/cs4+span", |b| {
        let config = ExporterConfig {
            //kwl: Box::new(BenchExporterConfig) as Box::<dyn KeywordLevelProvider>, // Using a boxed trait object has a measurable performance impact
            kwl: BenchExporterConfig,
            json: false,
            common_schema: true,
            etw_activities: true,
        };
        let exporter =
            EtwEventExporter::new(provider.clone(), config, tracelogging::InType::Bool32);
        b.iter(|| (exporter.log_span_data(&span_data)))
    });
}

#[cfg(all(target_os = "linux"))]
pub fn etw_benchmark(_c: &mut Criterion) {}

criterion_group!(benches, etw_benchmark);
criterion_main!(benches);
