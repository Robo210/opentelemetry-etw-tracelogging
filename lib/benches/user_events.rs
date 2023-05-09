#![allow(unused_imports, dead_code)]

#[path = "../src/constants.rs"]
mod constants;
#[path = "../src/error.rs"]
mod error;
#[path = "../src/user_events_exporter.rs"]
mod user_events_exporter;
#[path = "../src/exporter_traits.rs"]
mod exporter_traits;
#[path = "../src/json.rs"]
mod json;

use crate::exporter_traits::*;
use criterion::{criterion_group, criterion_main, Criterion};
use user_events_exporter::UserEventsExporter;
use opentelemetry::trace::{SpanContext, SpanId, SpanKind, TraceFlags, TraceState};
use opentelemetry::InstrumentationLibrary;
use opentelemetry_sdk::{
    export::trace::SpanData,
    trace::{EvictedHashMap, EvictedQueue},
};
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
}

#[cfg(all(target_os = "linux"))]
pub fn user_events_benchmark(c: &mut Criterion) {
    let mut provider = linux_tld::Provider::new(
        "otel_bench",
        &linux_tld::ProviderOptions::default(),
    );

    // Standard real-time level/keyword pairs
    provider.create_unregistered(true, linux_tlg::Level::Informational, 1);
    provider.create_unregistered(true, linux_tlg::Level::Verbose, 2);
    provider.create_unregistered(true, linux_tlg::Level::Verbose, 4);

    // Common Schema events use a level based on a span's Status
    provider.create_unregistered(true, linux_tlg::Level::Error, 1);
    provider.create_unregistered(true, linux_tlg::Level::Verbose, 1);

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

    let exporter = UserEventsExporter::new(Arc::new(provider));
    let mut config = ExporterConfig {
        kwl: BenchExporterConfig,
        json: false,
        common_schema: false,
        etw_activities: false,
    };

    let mut group = c.benchmark_group("export span_data");

    group.bench_function("provider disabled", |b| {
        b.iter(|| (exporter.log_span_data(&config, &span_data)))
    });

    config.common_schema = true;

    group.bench_function("provider enabled/cs4", |b| {
        b.iter(|| (exporter.log_span_data(&config, &span_data)))
    });

    config.common_schema = false;
    config.etw_activities = true;

    group.bench_function("provider enabled/span", |b| {
        b.iter(|| (exporter.log_span_data(&config, &span_data)))
    });

    config.common_schema = true;

    group.bench_function("provider enabled/cs4+span", |b| {
        b.iter(|| (exporter.log_span_data(&config, &span_data)))
    });
}

#[cfg(all(target_os = "windows"))]
pub fn user_events_benchmark(_c: &mut Criterion) {

}

criterion_group!(benches, user_events_benchmark);
criterion_main!(benches);
