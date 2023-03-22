//! # ETW Span Exporter
//!
//! The ETW Span Exporter logs spans as ETW events. Spans are logged as activity start
//! and stop events, using auto-generated activity IDs. Events in a span are logged as
//! ETW events using the span's activity ID.
//!
//! This crate is a no-op when running on Linux.
//!
//! The ETW provider ID is generated from a hash of the specified provider name.
//!
//! The ETW provider is joined to the group `{e60ec51a-8e54-5a4f-2fb260a4f9213b3a}`.
//! Events in this group should be interpreted according to the event and field tags
//! on each event.
//!
//! By default, span start and stop events are logged with keyword 1 and Level::Informational.
//! Events attached to the span are logged with keyword 2 and [`tracelogging::Level::Verbose`].
//!
//! # ETW Timestamps
//!
//! ## Batch Exporter
//!
//! Spans are exported asynchronously and in batches. Because of this,
//! the timestamps on the ETW events do not represent the time the span
//! was originally started or ended.
//!
//! When an ETW event has the EVENT_TAG_IGNORE_EVENT_TIME tag, the timestamp
//! on the EVENT_RECORD should be ignored when processing the event.
//! To get the real time of the event, look for a field tagged with
//! FIELD_TAG_IS_REAL_EVENT_TIME.
//!
//! ## Realtime Exporter
//!
//! ETW events for span start and stop will be logged in near-realtime.
//! Events attached to the span are logged as part of the span end, and
//! their ETW timestamps should be processed as described in the Batch Exporter
//! section.
//!
//! Span start events may be incomplete compared to those from the batch exporter.
//! Data such as the span's status (which corresponds to the ETW event's level)
//! is not available for the span start when logging in real-time. Attributes and
//! other data are only guaranteed to be present on span end events.
//!
//! The realtime exporter operates as a span processor rather than a span exporter.
//! It does not use sampling, which is technically required by the span processors spec.
//!
//! # Batching
//!
//! Every span that is exported is logged synchronously as an ETW event.
//! Batching or asynchronous logging is not implemented by the exporter.
//!
//! # Span Links
//! Span links are not (yet) implemented.
//!
//! # Differences with [OpenTelemetry-C++ ETW Exporter](https://github.com/open-telemetry/opentelemetry-cpp/tree/main/exporters/etw)
//!
//! - Spans are represented as ETW events differently.
//!   - The C++ exporter emits one ETW event for each span, when the span is completed. This event contains a
//!   start time and duration in the ETW event payload.
//!   - The Rust exporter emits two ETW events for each span, one for the span start and one for the span end.
//!   This allows for tools such as WPA to match the two events and generate a Region of Interest for that period of time.
//! - In C++, `bool` types are represented in the ETW event as InType `xs:byte`, OutType `xs:boolean`.
//! In Rust, `bool` types are represented in the ETW event as InType `win:Boolean`, OutType `xs:boolean`.
//!   - The C++ representation is more space efficient but is non-standard.
//!   - Rust applications can use the `xs:byte` representation by calling [`span_exporter::EtwExporterBuilder::with_byte_sized_bools`]
//!   when building the exporter.
//! - The C++ exporter converts the span Kind and Status to numeric values. The Rust exporter logs the string values.
//! - The OpenTelemetry-C++ SDK supports non-standard value types such as 32-bit and unsigned values, as well as
//!  optionally GUIDs. The OpenTelemetry-Rust crate does not support any of these, so the values will always be
//! logged as signed, 64-bit integers or strings.
//! - The C++ exporter does not support arrays and instead uses strings containing comma-separated
//! values for various fields. The Rust exporter will use arrays of the proper type.
//! - The C++ exporter can combine all attributes into a single JSON string that is then encoded with MsgPack,
//! and logs it in the ETW event as a field named "Payload".
//!   - Rust applications can emit a JSON string containing all the attributes by enabling the optional feature
//!   `json` on the crate and calling [`span_exporter::EtwExporterBuilder::with_json_payload`] when building
//!   the exporter. MsgPack encoding is not supported.
//! - The C++ exporter supports logs from the the OpenTelemetry Logging API proposal.
//! This is not (yet) supported by OpenTelemetry-Rust.
//! - The C++ exporter does not (currently) use opcodes or levels on its ETW events.
//! - The C++ exporter does not tag its ETW events or fields containing the "real" timestamp for the span/event.
//! - The C++ exporter and Rust exporter use different algorithms to generate activity IDs from span IDs.
//! This should not be noticable as span IDs and activity IDs should always be unique.
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
