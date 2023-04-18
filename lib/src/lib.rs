//! # TraceLogging Span Exporter
//!
//! ## Overview
//!
//! The TraceLogging Span Exporter exports Spans and Traces (soon) as
//! Windows ETW events or Linux user-mode tracepoints (user_events; requires a Linux 6.4+
//! kernel) events with TraceLogging-like encoding.
//! *Note*: Linux kernels without user_events support will not log any events.
//! 
//! ### ETW
//!
//! ETW is a Windows-specific system wide, high performance, lossy tracing API built into the
//! Windows kernel. Turning Spans into ETW events (activities) can allow a user to
//! the correlate the Span to other system activity, such as disk IO, memory allocations,
//! sample profiling, network activity, or any other event logged by the thousands of
//! ETW providers built into Windows and 3rd party software and drivers.
//!
//! ETW is not designed to be a transport mechanism or message passing interface for
//! forwarding data. These scenarios are better covered by other technologies
//! such as RPC or socket-based transports.
//!
//! Users unfamiliar with the basics of ETW may find the following links helpful.
//! The rest of the documentation for this exporter will assume familiarity
//! with ETW and trace processing tools such as WPA, PerfView, or TraceView.
//! - <https://learn.microsoft.com/windows/win32/etw/about-event-tracing>
//! - <https://learn.microsoft.com/windows-hardware/test/weg/instrumenting-your-code-with-etw>
//! 
//! This Span exporter uses [TraceLogging](https://learn.microsoft.com/windows/win32/tracelogging/trace-logging-about)
//! to log events. The ETW provider ID is generated from a hash of the specified provider name.
//! 
//! ### Linux user_events
//! 
//! User-mode event tracing [(user_events)](https://docs.kernel.org/trace/user_events.html)
//! is new to the Linux kernel starting with version 6.4. For the purposes of this exporter,
//! its functionality is nearly identical to ETW. Any differences between the two will be explicitly
//! called out in these docs.
//!
//! ## Realtime Events
//!
//! The real-time events should be used for almost all scenarios.
//! ETW events for span start and stop, as well as events added to the span,
//! will be logged in near-realtime. The timestamps on the ETW events will
//! be roughly within a few microseconds of the timestamp recorded by OpenTelemetry.
//!
//! Span start events may appear to be incomplete compared to those from the batch
//! exporter. Data such as the span's status (which corresponds to the ETW event's level)
//! is not available at the start of a span. Attributes that are available at the span
//! start will be added to the ETW event, but they may not match the ordering of the
//! full set of attributes on the span end ETW event.
//!
//! ## Common Schema 4.0 Events
//!
//! Common Schema 4.0 events are for advanced scenarios, when the event consumer
//! requires events in this schema. Spans are exported asynchronously
//! and in batches. Because of this, the timestamps on the ETW events do not
//! represent the time the span was originally started or ended.
//!
//! ## Span Links
//!
//! For non-Common Schema 4.0 events, each span link is exported as a separate ETW event.
//! The ETW event's name will match the span start event's name, and the link event's activity ID
//! will match the span's activity ID. A `Link` field in the payload contains the linked
//! span's ID, and any attributes for the link will be logged as additional payload fields.
//! Links are not (currently) supported by the JSON exporter option (described below).
//!
//! ## Example
//!
//! ```no_run
//! use opentelemetry_api::global::shutdown_tracer_provider;
//! use opentelemetry_api::trace::Tracer;
//!
//! let tracer = opentelemetry_etw::span_exporter::new_exporter("MyEtwProviderName")
//!     .install();
//!
//! tracer.in_span("doing_work", |cx| {
//!     // Traced app logic here...
//! });
//!
//! shutdown_tracer_provider(); // sending remaining spans
//! ```
//!
//! ## Differences with OpenTelemetry-C++ ETW Exporter
//!
//! ETW events not logged in the Common Schema 4.0 format will be different from how the
//! [OpenTelemetry-C++ ETW Exporter](https://github.com/open-telemetry/opentelemetry-cpp/tree/main/exporters/etw)
//! would log them. Some of these differences can controlled, as described below.
//!
//! - Spans are represented as ETW events differently.
//!   - The C++ exporter emits one ETW event for each span, when the span is completed. This event contains a
//!   start time and duration in the ETW event payload.
//!     - The `enableActivityTracking` option can be used to enable individual start and stop events from the C++ exporter.
//!   - The Rust exporter emits two ETW events for each span, one for the span start and one for the span end.
//!   This allows for tools such as WPA to match the two events and generate a Region of Interest for that period of time.
//! - The C++ exporter emits `bool` field types as InType `xs:byte`, OutType `xs:boolean`.
//! The Rust exporter emits, `bool` field types as InType `win:Boolean`, OutType `xs:boolean`.
//!   - The C++ representation is more space efficient but is non-standard.
//!   - Rust applications can use the `xs:byte` representation by calling [`span_exporter::ExporterBuilder::with_byte_sized_bools`]
//!   when building the exporter.
//! - The C++ exporter converts the span Kind and Status to numeric values. The Rust exporter logs the string values.
//! - The C++ exporter converts span Links into a single comma-separated string of span IDs, and does not include
//! link attributes. The Rust exporter uses individual events for each link, as described in the section [Span Links].
//! - The OpenTelemetry-C++ SDK supports non-standard value types such as 32-bit and unsigned values, as well as
//! optionally GUIDs, which are emitted as their corresponding InTypes.
//! The OpenTelemetry-Rust crate only supports the types listed in the OpenTelemetry standard, and the exporter will not
//! attempt to coerce values into other types in the ETW event.
//! - The C++ exporter does not support arrays and instead emits strings containing comma-separated
//! values. The Rust exporter emits arrays of the corresponding type.
//! - The C++ exporter can combine all attributes into a single JSON string that is then encoded with MsgPack,
//! and adds it to the ETW event as a field named "Payload".
//!   - Rust applications can emit a JSON string containing all the attributes by enabling the optional feature
//!   `json` on the crate and calling [`span_exporter::ExporterBuilder::with_json_payload`] when building
//!   the exporter. MsgPack encoding is not supported.
//! - The C++ exporter supports logs from the the OpenTelemetry Logging API proposal.
//! This is not (yet) supported by OpenTelemetry-Rust.
//! - The C++ exporter does not (currently) use opcodes or levels on its ETW events.
//! - The C++ exporter and Rust exporter use different algorithms to generate activity IDs from span IDs.
//! This should not be noticeable as span IDs and activity IDs should always be unique.
//!   - The C++ exporter (currently) copies the 8-byte span ID into the 16-byte activity ID GUID, leaving the
//!   remaining 8 bytes empty. This makes the GUID non-compliant, and the uniqueness guarantees of a GUID/LUID
//!   cannot be applied to activities IDs generated in this way.
mod batch_exporter;
mod builder;
mod constants;
mod error;
mod etw_exporter;
mod exporter_traits;
mod json;
mod realtime_exporter;
mod user_events_exporter;

pub mod span_exporter {
    pub use crate::batch_exporter::*;
    pub use crate::builder::*;
    pub use crate::constants::*;
    pub use crate::error::*;
    pub use crate::realtime_exporter::*;
    pub use crate::exporter_traits::*;
}
