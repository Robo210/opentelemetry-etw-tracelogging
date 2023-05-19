#![allow(unused_imports, dead_code)]

#[path = "../src/exporter_traits.rs"]
mod exporter_traits;
#[path = "../src/common/mod.rs"]
mod common;
#[path = "../src/user_events.rs"]
mod user_events;

use crate::exporter_traits::*;
use criterion::{criterion_group, criterion_main, Criterion};
use opentelemetry::trace::{SpanContext, SpanId, SpanKind, TraceFlags, TraceState};
use opentelemetry::InstrumentationLibrary;
use opentelemetry_sdk::{
    export::trace::SpanData,
    trace::{EvictedHashMap, EvictedQueue},
};
use std::borrow::Cow;
use std::sync::Arc;
use std::time::SystemTime;
use user_events::UserEventsExporter;

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

#[cfg(all(target_os = "linux"))]
pub fn user_events_benchmark(c: &mut Criterion) {
    let mut provider = eventheader_dynamic::Provider::new(
        "otel_bench",
        &eventheader_dynamic::ProviderOptions::default(),
    );

    // Standard real-time level/keyword pairs
    provider.create_unregistered(true, eventheader::Level::Informational, 1);
    provider.create_unregistered(true, eventheader::Level::Verbose, 2);
    provider.create_unregistered(true, eventheader::Level::Verbose, 4);

    // Common Schema events use a level based on a span's Status
    provider.create_unregistered(true, eventheader::Level::Error, 1);
    provider.create_unregistered(true, eventheader::Level::Verbose, 1);

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

    let provider = Arc::new(provider);

    group.bench_function("provider disabled", |b| {
        let config = ExporterConfig {
            kwl: BenchExporterConfig,
            json: false,
            common_schema: false,
            etw_activities: false,
        };
        let exporter = UserEventsExporter::new(provider.clone(), config);
        b.iter(|| (exporter.log_span_data(&span_data)))
    });

    group.bench_function("provider enabled/cs4", |b| {
        let config = ExporterConfig {
            kwl: BenchExporterConfig,
            json: false,
            common_schema: true,
            etw_activities: false,
        };
        let exporter = UserEventsExporter::new(provider.clone(), config);
        b.iter(|| (exporter.log_span_data(&span_data)))
    });

    group.bench_function("provider enabled/span", |b| {
        let config = ExporterConfig {
            kwl: BenchExporterConfig,
            json: false,
            common_schema: false,
            etw_activities: true,
        };
        let exporter = UserEventsExporter::new(provider.clone(), config);
        b.iter(|| (exporter.log_span_data(&span_data)))
    });

    group.bench_function("provider enabled/cs4+span", |b| {
        let config = ExporterConfig {
            kwl: BenchExporterConfig,
            json: false,
            common_schema: true,
            etw_activities: true,
        };
        let exporter = UserEventsExporter::new(provider.clone(), config);
        b.iter(|| (exporter.log_span_data(&span_data)))
    });
}

#[cfg(all(target_os = "windows"))]
pub fn user_events_benchmark(_c: &mut Criterion) {}

criterion_group!(benches, user_events_benchmark);
criterion_main!(benches);
