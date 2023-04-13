use std::{sync::Arc, time::SystemTime};
use linux_tlg::{Opcode, Level, FieldFormat};
use linux_tld::{EventSet, EventBuilder};
use opentelemetry::{trace::{TraceError, Link, Status, Event, SpanKind, SpanContext, SpanId}, Key, Value, Array};
use opentelemetry_sdk::export::trace::{ExportResult, SpanData};

use crate::{json, exporter_traits::*, span_exporter::*};

#[derive(Clone)]
pub(crate) struct UserEventsExporterConfig {
    pub(crate) provider: Arc<linux_tld::Provider>,
    pub(crate) span_keywords: u64,
    pub(crate) event_keywords: u64,
    pub(crate) links_keywords: u64,
    pub(crate) json: bool,
    pub(crate) common_schema: bool,
    pub(crate) etw_activities: bool,
}

impl ExporterConfig for UserEventsExporterConfig {
    fn get_provider(&self) -> ProviderWrapper {
        ProviderWrapper::UserEvents(self.provider.clone())
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

    fn get_export_as_json(&self) -> bool {
        self.json
    }

    fn get_export_common_schema_event(&self) -> bool {
        self.common_schema
    }

    fn get_export_span_events(&self) -> bool {
        self.etw_activities
    }
}

pub(crate) struct UserEventsExporter {}

impl UserEventsExporter {
    #[allow(dead_code)]
    pub(crate) fn new() -> UserEventsExporter {
        // Unfortunately we can't safely share a cached EventBuilder without adding undesirable locking
        UserEventsExporter {}
    }

    fn add_attributes_to_event(
        &self,
        eb: &mut EventBuilder,
        attribs: &mut dyn Iterator<Item = (&Key, &Value)>,
    ) {
        for attrib in attribs {
            let field_name = &attrib.0.to_string();
            match attrib.1 {
                Value::Bool(b) => {
                    eb.add_value(field_name, *b, FieldFormat::Boolean, 0);
                }
                Value::I64(i) => {
                    eb.add_value(field_name, *i, FieldFormat::SignedInt, 0);
                }
                Value::F64(f) => {
                    eb.add_value(field_name, *f, FieldFormat::Float, 0);
                }
                Value::String(s) => {
                    eb.add_str(field_name, &s.to_string(), FieldFormat::Default, 0);
                }
                Value::Array(array) => match array {
                    Array::Bool(v) => {
                        eb.add_value_sequence(
                            field_name,
                            v.iter(),
                            FieldFormat::Boolean,
                            0,
                        );
                    }
                    Array::I64(v) => {
                        eb.add_value_sequence(field_name, v.iter(), FieldFormat::SignedInt, 0);
                    }
                    Array::F64(v) => {
                        eb.add_value_sequence(field_name, v.iter(), FieldFormat::Float, 0);
                    }
                    Array::String(v) => {
                        eb.add_str_sequence(
                            field_name,
                            v.iter().map(|s| s.to_string()),
                            FieldFormat::Default,
                            0,
                        );
                    }
                },
            }
        }
    }

    fn write_span_links(
        &self,
        tlg_provider: &EventSet,
        eb: &mut EventBuilder,
        activities: &Activities,
        event_name: &str,
        span_timestamp: &SystemTime,
        links: &mut dyn Iterator<Item = &Link>,
    ) -> ExportResult {
        for link in links {
            eb.reset(
                event_name,
                EVENT_TAG_IGNORE_EVENT_TIME as u16,
            );
            eb.opcode(Opcode::Info);

            eb.add_value(
                "time",
                span_timestamp.duration_since(std::time::SystemTime::UNIX_EPOCH).unwrap().as_secs(),
                FieldFormat::Time,
                FIELD_TAG_IS_REAL_EVENT_TIME as u16,
            );

            eb.add_str(
                "Link",
                std::fmt::format(format_args!("{:16x}", link.span_context.span_id())),
                FieldFormat::Default,
                0,
            );

            self.add_attributes_to_event(
                eb,
                &mut link.attributes.iter().map(|kv| (&kv.key, &kv.value)),
            );

            let err = eb.write(
                &tlg_provider,
                Some(&activities.activity_id),
                activities.parent_activity_id.as_ref(),
            );
    
            if err != 0 {
                return Err(TraceError::ExportFailed(Box::new(LinuxError { err })));
            }
        }

        Ok(())
    }

    fn write_span_events(
        &self,
        tlg_provider: &EventSet,
        eb: &mut EventBuilder,
        activities: &Activities,
        events: &mut dyn Iterator<Item = &Event>,
        export_payload_as_json: bool,
    ) -> ExportResult {
        for event in events {
            eb.reset(
                &event.name,
                EVENT_TAG_IGNORE_EVENT_TIME as u16,
            );
            eb.opcode(Opcode::Info);

            eb.add_value(
                "time",
                event.timestamp.duration_since(std::time::SystemTime::UNIX_EPOCH).unwrap().as_secs(),
                FieldFormat::Time,
                FIELD_TAG_IS_REAL_EVENT_TIME as u16,
            );

            eb.add_str("SpanId", &activities.span_id, FieldFormat::Default, 0);

            if !activities.parent_span_id.is_empty() {
                eb.add_str("ParentId", &activities.parent_span_id, FieldFormat::Default, 0);
            }

            eb.add_str("TraceId", &activities.trace_id_name, FieldFormat::Default, 0);

            let mut added = false;

            #[cfg(feature = "json")]
            if export_payload_as_json {
                let json_string = json::get_attributes_as_json(&mut event.attributes.iter().map(|kv| (&kv.key, &kv.value)));
                eb.add_str("Payload", &json_string, FieldFormat::StringJson, 0);
                added = true;
            }

            if !added {
                self.add_attributes_to_event(
                    eb,
                    &mut event.attributes.iter().map(|kv| (&kv.key, &kv.value)),
                );
            }

            let err = eb.write(
                &tlg_provider,
                Some(&activities.activity_id),
                activities.parent_activity_id.as_ref(),
            );
    
            if err != 0 {
                return Err(TraceError::ExportFailed(Box::new(LinuxError { err })));
            }
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn write_span_event(
        &self,
        tlg_provider: &EventSet,
        eb: &mut EventBuilder,
        name: &str,
        activities: &Activities,
        event_time: &SystemTime,
        span_kind: Option<&SpanKind>,
        status: &Status,
        attributes: &mut dyn Iterator<Item = (&Key, &Value)>,
        is_start: bool,
        add_tags: bool,
        export_payload_as_json: bool,
    ) -> ExportResult {
        let event_tags = if add_tags {
            EVENT_TAG_IGNORE_EVENT_TIME
        } else {
            0
        };
        let (opcode, time_field_name) = if is_start {
            (Opcode::Start, "StartTime")
        } else {
            (Opcode::Stop, "EndTime")
        };

        eb.reset(name, event_tags as u16);
        eb.opcode(opcode);

        eb.add_value(
            time_field_name,
            event_time.duration_since(std::time::SystemTime::UNIX_EPOCH).unwrap().as_secs(),
            FieldFormat::Time,
            FIELD_TAG_IS_REAL_EVENT_TIME as u16,
        );

        if let Some(sk) = span_kind {
            eb.add_str(
                "Kind",
                match sk {
                    SpanKind::Client => "Client",
                    SpanKind::Server => "Server",
                    SpanKind::Producer => "Producer",
                    SpanKind::Consumer => "Consumer",
                    SpanKind::Internal => "Internal",
                },
                FieldFormat::Default,
                0,
            );
        }

        if let Status::Error { description } = &status {
            eb.add_str("StatusMessage", description.to_string(), FieldFormat::Default, 0);
        };

        eb.add_str("SpanId", &activities.span_id, FieldFormat::Default, 0);

        if !activities.parent_span_id.is_empty() {
            eb.add_str("ParentId", &activities.parent_span_id, FieldFormat::Default, 0);
        }

        eb.add_str("TraceId", &activities.trace_id_name, FieldFormat::Default, 0);

        let mut added = false;

        #[cfg(feature = "json")]
        if export_payload_as_json {
            let json_string = json::get_attributes_as_json(attributes);
            eb.add_str("Payload", &json_string, FieldFormat::StringJson, 0);
            added = true;
        }

        if !added {
            self.add_attributes_to_event(eb, attributes);
        }

        let err = eb.write(
            &tlg_provider,
            Some(&activities.activity_id),
            activities.parent_activity_id.as_ref(),
        );

        if err != 0 {
            return Err(TraceError::ExportFailed(Box::new(LinuxError { err })));
        }

        Ok(())
    }

    fn write_common_schema_span<'a, C>(
        &self,
        tlg_provider: &EventSet,
        eb: &mut EventBuilder,
        name: &str,
        span_data: &SpanData,
        span_context: &SpanContext,
        export_payload_as_json: bool,
        _attributes: C,
    ) -> ExportResult
    where
        C: IntoIterator<Item = (&'a Key, &'a Value)>,
    {
        let trace_id = span_context.trace_id().to_string();
        let span_id = span_context.span_id().to_string();

        let event_tags: u32 = 0; // TODO
        eb.reset(name, event_tags as u16);
        eb.opcode(Opcode::Info);

        // Promoting values from PartC to PartA extensions is apparently just a draft spec
        // and not necessary / supported by consumers.
        // let exts = json::extract_common_schema_parta_exts(attributes);

        eb.add_value("__csver__", 0x0401u16, FieldFormat::HexInt, 0);
        eb.add_struct("PartA", 2 /* + exts.len() as u8*/, 0);
        {
            let time: String = chrono::DateTime::to_rfc3339(
                &chrono::DateTime::<chrono::Utc>::from(span_data.end_time),
            );
            eb.add_str("time", time, FieldFormat::Default, 0);

            eb.add_struct("ext_dt", 2, 0);
            {
                eb.add_str("traceId", &trace_id, FieldFormat::Default, 0);
                eb.add_str("spanId", &span_id, FieldFormat::Default, 0);
            }

            // for ext in exts {
            //     eb.add_struct(ext.0, ext.1.len() as u8, 0);

            //     for field in ext.1 {
            //         eb.add_str(field.0, field.1.as_ref(), FieldFormat::Default, 0);
            //     }
            // }
        }

        // if !span_data.links.is_empty() {
        //     eb.add_struct("PartB", 5, 0);
        //     {
        //         eb.add_str("_typeName", "SpanLink", FieldFormat::Default, 0);
        //         eb.add_str("fromTraceId", &traceId, FieldFormat::Default, 0);
        //         eb.add_str("fromSpanId", &spanId, FieldFormat::Default, 0);
        //         eb.add_str("toTraceId", "SpanLink", FieldFormat::Default, 0);
        //         eb.add_str("toSpanId", "SpanLink", FieldFormat::Default, 0);
        //     }
        // }

        let mut status_message: String = String::default();
        let mut partb_field_count = 5u8;
        if span_data.parent_span_id != SpanId::INVALID {
            partb_field_count += 1;
        }
        if let Status::Error { description } = &span_data.status {
            partb_field_count += 1;
            status_message = description.to_string();
        }
        // TODO: azureResourceProvider: string
        if !span_data.links.is_empty() {
            partb_field_count += 1; // Type is an "array", but really it's just a string with a JSON array
        }

        eb.add_struct("PartB", partb_field_count, 0);
        {
            eb.add_str("_typeName", "Span", FieldFormat::Default, 0);
            if span_data.parent_span_id != SpanId::INVALID {
                eb.add_str(
                    "parentId",
                    &span_data.parent_span_id.to_string(),
                    FieldFormat::Default,
                    0,
                );
            }
            eb.add_str("name", name, FieldFormat::Default, 0);
            eb.add_value(
                "kind",
                match span_data.span_kind {
                    SpanKind::Internal => 0u8,
                    SpanKind::Server => 1,
                    SpanKind::Client => 2,
                    SpanKind::Producer => 3,
                    SpanKind::Consumer => 4,
                },
                FieldFormat::UnsignedInt,
                0,
            );
            eb.add_str(
                "startTime",
                &chrono::DateTime::to_rfc3339(&chrono::DateTime::<chrono::Utc>::from(
                    span_data.end_time,
                )),
                FieldFormat::Default,
                0,
            );
            eb.add_value(
                "success",
                match span_data.status {
                    Status::Ok => 1u8,
                    _ => 0u8,
                },
                FieldFormat::Boolean,
                0,
            );
            if !status_message.is_empty() {
                eb.add_str("statusMessage", &status_message, FieldFormat::Default, 0);
            }
            // TODO: azureResourceProvider: string
            if !span_data.links.is_empty() {
                let mut links = String::with_capacity(2 + (78 * span_data.links.len()));
                links += "[";
                for link in span_data.links.iter() {
                    links += "{\"toTraceId\":\"";
                    links += &link.span_context.trace_id().to_string();
                    links += "\",\"toSpanId\":\"";
                    links += &link.span_context.span_id().to_string();
                    links += "\"}";
                }
                links += "]";

                eb.add_str("links", &links, FieldFormat::StringJson, 0);
            }
            // TODO: promote HTTP, Database and Messaging fields
        }

        if !span_data.attributes.is_empty() {
            let partc_field_count = if export_payload_as_json {
                1u8
            } else {
                span_data.attributes.len() as u8
            };

            eb.add_struct("PartC", partc_field_count, 0);
            {
                let mut added = false;

                #[cfg(feature = "json")]
                if export_payload_as_json {
                    let json_string = json::get_attributes_as_json(&mut span_data.attributes.iter());
                    eb.add_str("Payload", &json_string, FieldFormat::StringJson, 0);
                    added = true;
                }

                if !added {
                    self.add_attributes_to_event(eb, &mut span_data.attributes.iter());
                }
            }
        }

        let err = eb.write(
            &tlg_provider,
            None,
            None,
        );

        if err != 0 {
            return Err(TraceError::ExportFailed(Box::new(LinuxError { err })));
        }

        Ok(())
    }
}

impl EventExporter for UserEventsExporter {
    // Called by the real-time exporter when a span is started
    fn log_span_start<C, S>(
        &self,
        provider: &C,
        span: &S,
    ) -> ExportResult
    where
        C: ExporterConfig,
        S: opentelemetry_api::trace::Span + EtwSpan,
    {
        if !provider.get_export_span_events() {
            // Common schema events are logged at span end
            return Ok(());
        }

        let tlg_provider = match provider.get_provider() {
            ProviderWrapper::UserEvents(p) => p,
            _ => panic!()
        };

        let span_es = if let Some(es) = tlg_provider.find_set(Level::Informational, provider.get_span_keywords()) {
            es
        } else {
            return Ok(());
        };

        if !span_es.enabled() {
            return Ok(());
        }

        let export_payload_as_json = provider.get_export_as_json();

        let span_context = opentelemetry_api::trace::Span::span_context(span);

        let span_data = span.get_span_data();

        let activities = Activities::generate(
            &span_context.span_id(),
            &span_data.parent_span_id,
            &span_context.trace_id(),
        );

        let mut eb = EventBuilder::new();

        self.write_span_event(
            &span_es,
            &mut eb,
            &span_data.name,
            &activities,
            &span_data.start_time,
            Some(&span_data.span_kind),
            &Status::Unset,
            &mut std::iter::empty(),
            true,
            false,
            export_payload_as_json,
        )?;

        let links_es = if let Some(es) = tlg_provider.find_set(Level::Informational, provider.get_links_keywords()) {
            es
        } else {
            return Ok(());
        };

        if !span_es.enabled() {
            return Ok(());
        }

        self.write_span_links(
            &links_es,
            &mut eb,
            &activities,
            &span_data.name,
            &span_data.start_time,
            &mut span_data.links.iter(),
        )?;

        Ok(())
    }

    // Called by the real-time exporter when a span is ended
    fn log_span_end<C, S>(&self, provider: &C, span: &S) -> ExportResult
    where
        C: ExporterConfig,
        S: opentelemetry_api::trace::Span + EtwSpan,
    {
        //let event_keywords = provider.get_event_keywords();
        let export_payload_as_json = provider.get_export_as_json();
        let tlg_provider = match provider.get_provider() {
            ProviderWrapper::UserEvents(p) => p,
            _ => panic!()
        };

        let span_es = if let Some(es) = tlg_provider.find_set(Level::Informational, provider.get_span_keywords()) {
            es
        } else {
            return Ok(());
        };

        if !span_es.enabled() {
            return Ok(());
        }

        let span_data = span.get_span_data();

        let mut eb = EventBuilder::new();

        if provider.get_export_span_events()
        {
            let activities = Activities::generate(
                &span_data.span_context.span_id(),
                &span_data.parent_span_id,
                &span_data.span_context.trace_id(),
            );

            self.write_span_event(
                &span_es,
                &mut eb,
                &span_data.name,
                &activities,
                &span_data.end_time,
                Some(&span_data.span_kind),
                &span_data.status,
                &mut span_data.attributes.iter(),
                false,
                false,
                export_payload_as_json,
            )?;
        }

        if provider.get_export_common_schema_event()
        {
            let attributes = span_data.resource.iter().chain(span_data.attributes.iter());
            self.write_common_schema_span(
                &span_es,
                &mut eb,
                &span_data.name,
                span_data,
                span.span_context(),
                export_payload_as_json,
                attributes,
            )?;
        }

        Ok(())
    }

    // Called by the real-time exporter when an event is added to a span
    fn log_span_event<C, S>(
        &self,
        provider: &C,
        event: opentelemetry_api::trace::Event,
        span: &S,
    ) -> ExportResult
    where
        C: ExporterConfig,
        S: opentelemetry_api::trace::Span + EtwSpan,
    {
        let tlg_provider = match provider.get_provider() {
            ProviderWrapper::UserEvents(p) => p,
            _ => panic!()
        };

        let span_es = if let Some(es) = tlg_provider.find_set(Level::Informational, provider.get_span_keywords()) {
            es
        } else {
            return Ok(());
        };

        if !span_es.enabled() {
            return Ok(());
        }

        if !provider.get_export_span_events()
        {
            // TODO: Common Schema PartB SpanEvent events
            return Ok(());
        }

        let export_payload_as_json = provider.get_export_as_json();
        let span_data = span.get_span_data();

        let activities = Activities::generate(
            &span_data.span_context.span_id(),
            &span_data.parent_span_id,
            &span_data.span_context.trace_id(),
        );

        let mut eb = EventBuilder::new();

        eb.reset(
            &event.name,
            EVENT_TAG_IGNORE_EVENT_TIME as u16,
        );
        eb.opcode(Opcode::Info);

        eb.add_value(
            "time",
            event.timestamp.duration_since(std::time::SystemTime::UNIX_EPOCH).unwrap().as_secs(),
            FieldFormat::Time,
            FIELD_TAG_IS_REAL_EVENT_TIME as u16,
        );

        eb.add_str("SpanId", &activities.span_id, FieldFormat::Default, 0);

        if !activities.parent_span_id.is_empty() {
            eb.add_str("ParentId", &activities.parent_span_id, FieldFormat::Default, 0);
        }

        eb.add_str("TraceId", &activities.trace_id_name, FieldFormat::Default, 0);

        let mut added = false;

        #[cfg(feature = "json")]
        if export_payload_as_json {
            let json_string = json::get_attributes_as_json(&mut event.attributes.iter().map(|kv| (&kv.key, &kv.value)));
            eb.add_str("Payload", &json_string, FieldFormat::StringJson, 0);
            added = true;
        }

        if !added {
            self.add_attributes_to_event(
                &mut eb,
                &mut event.attributes.iter().map(|kv| (&kv.key, &kv.value)),
            );
        }

        let err = eb.write(
            &span_es,
            Some(&activities.activity_id),
            activities.parent_activity_id.as_ref(),
        );

        if err != 0 {
            return Err(TraceError::ExportFailed(Box::new(LinuxError { err })));
        }

        Ok(())
    }

    // Called by the batch exporter sometime after span is completed
    fn log_span_data<C>(
        &self,
        provider: &C,
        span_data: &SpanData,
    ) -> ExportResult
    where
        C: ExporterConfig,
    {
        let export_payload_as_json = provider.get_export_as_json();
        let tlg_provider = match provider.get_provider() {
            ProviderWrapper::UserEvents(p) => p,
            _ => panic!()
        };

        let level = match span_data.status {
            Status::Ok => Level::Informational,
            Status::Error { .. } => Level::Error,
            Status::Unset => Level::Verbose,
        };

        let span_es = if let Some(es) = tlg_provider.find_set(level, provider.get_span_keywords()) {
            es
        } else {
            return Ok(());
        };

        if !span_es.enabled() {
            return Ok(());
        }

        // TODO: We should be caching this and reusing it for the entire batch export
        let mut eb = EventBuilder::new();

        let mut err = Ok(());
        if provider.get_export_span_events() {
            let activities = Activities::generate(
                &span_data.span_context.span_id(),
                &span_data.parent_span_id,
                &span_data.span_context.trace_id(),
            );

            err = self
                .write_span_event(
                    &span_es,
                    &mut eb,
                    &span_data.name,
                    &activities,
                    &span_data.start_time,
                    Some(&span_data.span_kind),
                    &span_data.status,
                    &mut std::iter::empty(),
                    true,
                    true,
                    export_payload_as_json,
                )
                .and_then(|_| {
                    let events_es = if let Some(es) = tlg_provider.find_set(Level::Verbose, provider.get_event_keywords()) {
                        es
                    } else {
                        return Ok(());
                    };
            
                    if !events_es.enabled() {
                        return Ok(());
                    }

                    self.write_span_events(
                        &events_es,
                        &mut eb,
                        &activities,
                        &mut span_data.events.iter(),
                        export_payload_as_json,
                    )
                })
                .and_then(|_| {
                    let links_es = if let Some(es) = tlg_provider.find_set(Level::Verbose, provider.get_links_keywords()) {
                        es
                    } else {
                        return Ok(());
                    };
            
                    if !links_es.enabled() {
                        return Ok(());
                    }

                    self.write_span_links(
                        &links_es,
                        &mut eb,
                        &activities,
                        &span_data.name,
                        &span_data.start_time,
                        &mut span_data.links.iter(),
                    )
                })
                .and_then(|_| {
                    self.write_span_event(
                        &span_es,
                        &mut eb,
                        &span_data.name,
                        &activities,
                        &span_data.end_time,
                        Some(&span_data.span_kind),
                        &span_data.status,
                        &mut span_data.attributes.iter(),
                        false,
                        true,
                        export_payload_as_json,
                    )
                });
        }

        if provider.get_export_common_schema_event()
        {
            let span_es = if let Some(es) = tlg_provider.find_set(Level::Informational, provider.get_span_keywords()) {
                es
            } else {
                return Ok(());
            };
    
            if span_es.enabled() {
                let attributes = span_data.resource.iter().chain(span_data.attributes.iter());

                let err2 = self.write_common_schema_span(
                    &span_es,
                    &mut eb,
                    &span_data.name,
                    span_data,
                    &span_data.span_context,
                    export_payload_as_json,
                    attributes,
                );

                err = err.and(err2);
            }
        }

        err
    }
}