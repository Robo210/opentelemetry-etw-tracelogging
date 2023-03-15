//! # ETW Span Exporter
//!
//! The ETW Span Exporter logs spans as ETW events.
//! Spans are logged as activity start and stop events,
//! using auto-generated activity IDs.
//! Events in a span are logged as ETW events using the
//! span's activity ID.
//!
//! This crate is a no-op when running on Linux.
//!
//! The ETW provider ID is generated from a hash of the
//! specified provider name.
//!
//! The ETW provider is joined to the group
//! `{e60ec51a-8e54-5a4f-2fb260a4f9213b3a}`. Events in this
//! group should be interpreted according to the event and
//! field tags on each event.
//!
//! By default, span start and stop events are logged with
//! keyword 1 and Level::Informational. Events attached
//! to the span are logged with keyword 2 and ['Level::Verbose`].
//!
//! # ETW Timestamps
//!
//! ## Batch Exporter
//!
//! Spans are exported asynchronously and in batches.
//! Because of this, the timestamps on the ETW events
//! do not represent the time the span was originally
//! started or ended.
//!
//! When an ETW event has the EVENT_TAG_IGNORE_EVENT_TIME tag,
//! the timestamp on the EVENT_RECORD should be ignored when
//! processing the event. To get the real time of the event,
//! look for a field tagged with FIELD_TAG_IS_REAL_EVENT_TIME.
//!
//! ## Realtime Exporter
//!
//! ETW events for span start and stop will be logged in near-realtime.
//! Events attached to the span are logged as part of the span end,
//! and their ETW timestamps should be processed as described in the
//! Batch Exporter section.
//!
//! Span start events may be incomplete compared to those from the
//! batch exporter. Attributes and other data is only guaranteed
//! to be present on span end events.
//!
//! # Batching
//!
//! Every span that is exported is logged synchronously as an ETW event.
//! Batching or asynchronous logging is not implemented by the exporter.
//!
//! # Examples
//!
//! ## Batch Exporter
//! ```no_run
//! use opentelemetry_api::global::shutdown_tracer_provider;
//! use opentelemetry_api::trace::Tracer;
//!
//! let tracer = opentelemetry_etw::span_exporter::new_etw_exporter("MyEtwProviderName")
//!     .install_simple();
//!
//! tracer.in_span("doing_work", |cx| {
//!     // Traced app logic here...
//! });
//!
//! shutdown_tracer_provider(); // sending remaining spans
//! ```
//!
//! ## Realtime Exporter
//! ```no_run
//! use opentelemetry_api::global::shutdown_tracer_provider;
//! use opentelemetry_api::trace::Tracer;
//!
//! let tracer = opentelemetry_etw::span_exporter::new_etw_exporter("MyEtwProviderName")
//!     .install_realtime();
//!
//! tracer.in_span("doing_work", |cx| {
//!     // Traced app logic here...
//! });
//!
//! shutdown_tracer_provider(); // sending remaining spans
//! ```
mod batch_exporter;
mod builder;
mod constants;
mod error;
mod etw_exporter;
mod realtime_exporter;

pub mod span_exporter {
    pub use crate::batch_exporter::*;
    pub use crate::builder::*;
    pub use crate::constants::*;
    pub use crate::realtime_exporter::*;
}
