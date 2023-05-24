#![allow(unused_imports, unused_mut, unused_variables)]

use eventheader::{FieldFormat, Level, Opcode};
use eventheader_dynamic::{EventBuilder, EventSet};
use opentelemetry::{
    trace::{Event, Link, SpanContext, SpanId, SpanKind, Status, TraceError},
    Array, Key, Value,
};
use opentelemetry_sdk::export::{trace::{ExportResult, SpanData}, logs::{LogData, self}};
use std::{cell::RefCell, sync::Arc, time::SystemTime};

use crate::{exporter_traits::*, common::{json, activities::*, EtwSpan, *}};

thread_local! {static EBW: RefCell<EventBuilder> = RefCell::new(EventBuilder::new());}

#[allow(dead_code)]
pub(crate) fn register_eventsets(
    provider: &mut eventheader_dynamic::Provider,
    kwl: &impl KeywordLevelProvider,
) {
    #[cfg(not(test))]
    {
        // Standard real-time level/keyword pairs
        provider.register_set(kwl.get_span_level().into(), kwl.get_span_keywords());
        provider.register_set(kwl.get_span_event_level().into(), kwl.get_span_event_keywords());
        provider.register_set(kwl.get_span_links_level().into(), kwl.get_span_links_keywords());

        // Common Schema events use a level based on a span's Status
        provider.register_set(eventheader::Level::Informational, kwl.get_span_keywords());
        provider.register_set(eventheader::Level::Error, kwl.get_span_keywords());
        provider.register_set(eventheader::Level::Verbose, kwl.get_span_keywords());
    }
    #[cfg(test)]
    {
        // Standard real-time level/keyword pairs
        provider.create_unregistered(true, kwl.get_span_level().into(), kwl.get_span_keywords());
        provider.create_unregistered(true, kwl.get_span_event_level().into(), kwl.get_span_event_keywords());
        provider.create_unregistered(true, kwl.get_span_links_level().into(), kwl.get_span_links_keywords());

        // Common Schema events use a level based on a span's Status
        provider.create_unregistered(
            true,
            eventheader::Level::Informational,
            kwl.get_span_keywords(),
        );
        provider.create_unregistered(true, eventheader::Level::Error, kwl.get_span_keywords());
        provider.create_unregistered(true, eventheader::Level::Verbose, kwl.get_span_keywords());
    }
}

pub(crate) struct UserEventsExporter<C: KeywordLevelProvider> {
    provider: Arc<eventheader_dynamic::Provider>,
    exporter_config: ExporterConfig<C>,
}

impl<C: KeywordLevelProvider> UserEventsExporter<C> {
    #[allow(dead_code)]
    pub(crate) fn new(
        provider: Arc<eventheader_dynamic::Provider>,
        exporter_config: ExporterConfig<C>,
    ) -> Self {
        // Unfortunately we can't safely share a cached EventBuilder without adding undesirable locking
        UserEventsExporter {
            provider,
            exporter_config,
        }
    }

    fn add_attributes_to_event<'a, I>(
        &self,
        eb: &mut EventBuilder,
        attribs: I,
    ) where I: Iterator<Item = (&'a Key, &'a Value)> {
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
                        eb.add_value_sequence(field_name, v.iter(), FieldFormat::Boolean, 0);
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
            eb.reset(event_name, EVENT_TAG_IGNORE_EVENT_TIME as u16);
            eb.opcode(Opcode::Info);

            eb.add_value(
                "time",
                span_timestamp
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
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
            eb.reset(&event.name, EVENT_TAG_IGNORE_EVENT_TIME as u16);
            eb.opcode(Opcode::Info);

            eb.add_value(
                "time",
                event
                    .timestamp
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                FieldFormat::Time,
                FIELD_TAG_IS_REAL_EVENT_TIME as u16,
            );

            eb.add_str("SpanId", &activities.span_id, FieldFormat::Default, 0);

            if !activities.parent_span_id.is_empty() {
                eb.add_str(
                    "ParentId",
                    &activities.parent_span_id,
                    FieldFormat::Default,
                    0,
                );
            }

            eb.add_str(
                "TraceId",
                &activities.trace_id_name,
                FieldFormat::Default,
                0,
            );

            let mut added = false;

            #[cfg(feature = "json")]
            if export_payload_as_json {
                let json_string = json::get_attributes_as_json(
                    &mut event.attributes.iter().map(|kv| (&kv.key, &kv.value)),
                );
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
    fn write_span_event<'a, I>(
        &self,
        tlg_provider: &EventSet,
        eb: &mut EventBuilder,
        name: &str,
        activities: &Activities,
        event_time: &SystemTime,
        span_kind: Option<&SpanKind>,
        status: &Status,
        attributes: I,
        is_start: bool,
        add_tags: bool,
        export_payload_as_json: bool,
    ) -> ExportResult
    where I: Iterator<Item = (&'a Key, &'a Value)> + Clone
    {
        let event_tags = if add_tags {
            EVENT_TAG_IGNORE_EVENT_TIME
        } else {
            0
        };
        let (opcode, time_field_name) = if is_start {
            (Opcode::ActivityStart, "StartTime")
        } else {
            (Opcode::ActivityStop, "EndTime")
        };

        eb.reset(name, event_tags as u16);
        eb.opcode(opcode);

        eb.add_value(
            time_field_name,
            event_time
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
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
            eb.add_str(
                "StatusMessage",
                description.to_string(),
                FieldFormat::Default,
                0,
            );
        };

        eb.add_str("SpanId", &activities.span_id, FieldFormat::Default, 0);

        if !activities.parent_span_id.is_empty() {
            eb.add_str(
                "ParentId",
                &activities.parent_span_id,
                FieldFormat::Default,
                0,
            );
        }

        eb.add_str(
            "TraceId",
            &activities.trace_id_name,
            FieldFormat::Default,
            0,
        );

        let mut added = false;

        #[cfg(feature = "json")]
        if export_payload_as_json {
            let json_string = json::get_attributes_as_json(attributes.clone());
            eb.add_str("Payload", &json_string, FieldFormat::StringJson, 0);
            added = true;
        }

        if !added {
            self.add_attributes_to_event(eb, attributes.clone());
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

    fn write_common_schema_span<'a, A>(
        &self,
        tlg_provider: &EventSet,
        eb: &mut EventBuilder,
        name: &str,
        span_data: &SpanData,
        span_context: &SpanContext,
        export_payload_as_json: bool,
        _attributes: A,
    ) -> ExportResult
    where
        A: IntoIterator<Item = (&'a Key, &'a Value)>,
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
                    let json_string =
                        json::get_attributes_as_json(&mut span_data.attributes.iter());
                    eb.add_str("Payload", &json_string, FieldFormat::StringJson, 0);
                    added = true;
                }

                if !added {
                    self.add_attributes_to_event(eb, &mut span_data.attributes.iter());
                }
            }
        }

        let err = eb.write(&tlg_provider, None, None);

        if err != 0 {
            return Err(TraceError::ExportFailed(Box::new(LinuxError { err })));
        }

        Ok(())
    }
}

impl<C: KeywordLevelProvider> EventExporter for UserEventsExporter<C> {
    fn enabled(&self, level: u8, keyword: u64) -> bool {
        let es = self.provider.find_set(level.into(), keyword);
        if es.is_some() {
            es.unwrap().enabled()
        } else {
            false
        }
    }

    // Called by the real-time exporter when a span is started
    fn log_span_start<S>(&self, span: &S) -> ExportResult
    where
        S: opentelemetry_api::trace::Span + EtwSpan,
    {
        if !self.exporter_config.get_export_etw_activity_events() {
            // Common schema events are logged at span end
            return Ok(());
        }

        let span_es = if let Some(es) = self.provider.find_set(
            self.exporter_config.get_span_level().into(),
            self.exporter_config.get_span_keywords(),
        ) {
            es
        } else {
            return Ok(());
        };

        if !span_es.enabled() {
            return Ok(());
        }

        let export_payload_as_json = self.exporter_config.get_export_as_json();

        let span_context = opentelemetry_api::trace::Span::span_context(span);

        let span_data = span.get_span_data();

        let activities = Activities::generate(
            &span_context.span_id(),
            &span_data.parent_span_id,
            &span_context.trace_id(),
        );

        EBW.with(|eb| {
            let mut eb = eb.borrow_mut();

            self.write_span_event(
                &span_es,
                &mut eb,
                &span_data.name,
                &activities,
                &span_data.start_time,
                Some(&span_data.span_kind),
                &Status::Unset,
                std::iter::empty(),
                true,
                false,
                export_payload_as_json,
            )?;

            let links_es = if let Some(es) = self.provider.find_set(
                self.exporter_config.get_span_links_level().into(),
                self.exporter_config.get_span_links_keywords(),
            ) {
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
    }

    // Called by the real-time exporter when a span is ended
    fn log_span_end<S>(&self, span: &S) -> ExportResult
    where
        S: opentelemetry_api::trace::Span + EtwSpan,
    {
        //let event_keywords = provider.get_event_keywords();
        let export_payload_as_json = self.exporter_config.get_export_as_json();

        let span_es = if let Some(es) = self.provider.find_set(
            self.exporter_config.get_span_level().into(),
            self.exporter_config.get_span_keywords(),
        ) {
            es
        } else {
            return Ok(());
        };

        if !span_es.enabled() {
            return Ok(());
        }

        let span_data = span.get_span_data();

        EBW.with(|eb| {
            let mut eb = eb.borrow_mut();

            if self.exporter_config.get_export_etw_activity_events() {
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
                    span_data.attributes.iter(),
                    false,
                    false,
                    export_payload_as_json,
                )?;
            }

            if self.exporter_config.get_export_common_schema_events() {
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
        })
    }

    // Called by the real-time exporter when an event is added to a span
    fn log_span_event<S>(&self, event: opentelemetry_api::trace::Event, span: &S) -> ExportResult
    where
        S: opentelemetry_api::trace::Span + EtwSpan,
    {
        let span_es = if let Some(es) = self.provider.find_set(
            self.exporter_config.get_span_level().into(),
            self.exporter_config.get_span_keywords(),
        ) {
            es
        } else {
            return Ok(());
        };

        if !span_es.enabled() {
            return Ok(());
        }

        if !self.exporter_config.get_export_etw_activity_events() {
            // TODO: Common Schema PartB SpanEvent events
            return Ok(());
        }

        let export_payload_as_json = self.exporter_config.get_export_as_json();
        let span_data = span.get_span_data();

        let activities = Activities::generate(
            &span_data.span_context.span_id(),
            &span_data.parent_span_id,
            &span_data.span_context.trace_id(),
        );

        EBW.with(|eb| {
            let mut eb = eb.borrow_mut();

            eb.reset(&event.name, EVENT_TAG_IGNORE_EVENT_TIME as u16);
            eb.opcode(Opcode::Info);

            eb.add_value(
                "time",
                event
                    .timestamp
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                FieldFormat::Time,
                FIELD_TAG_IS_REAL_EVENT_TIME as u16,
            );

            eb.add_str("SpanId", &activities.span_id, FieldFormat::Default, 0);

            if !activities.parent_span_id.is_empty() {
                eb.add_str(
                    "ParentId",
                    &activities.parent_span_id,
                    FieldFormat::Default,
                    0,
                );
            }

            eb.add_str(
                "TraceId",
                &activities.trace_id_name,
                FieldFormat::Default,
                0,
            );

            let mut added = false;

            #[cfg(feature = "json")]
            if export_payload_as_json {
                let json_string = json::get_attributes_as_json(
                    &mut event.attributes.iter().map(|kv| (&kv.key, &kv.value)),
                );
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
        })
    }

    // Called by the batch exporter sometime after span is completed
    fn log_span_data(&self, span_data: &SpanData) -> ExportResult {
        let export_payload_as_json = self.exporter_config.get_export_as_json();

        let level = match span_data.status {
            Status::Ok => Level::Informational,
            Status::Error { .. } => Level::Error,
            Status::Unset => Level::Verbose,
        };

        let span_es = if let Some(es) = self
            .provider
            .find_set(level, self.exporter_config.get_span_keywords())
        {
            es
        } else {
            return Ok(());
        };

        if !span_es.enabled() {
            return Ok(());
        }

        EBW.with(|eb| {
            let mut eb = eb.borrow_mut();
            let mut err = Ok(());

            if self.exporter_config.get_export_etw_activity_events() {
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
                        std::iter::empty(),
                        true,
                        true,
                        export_payload_as_json,
                    )
                    .and_then(|_| {
                        let events_es = if let Some(es) = self.provider.find_set(
                            self.exporter_config.get_span_event_level().into(),
                            self.exporter_config.get_span_event_keywords(),
                        ) {
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
                        let links_es = if let Some(es) = self.provider.find_set(
                            self.exporter_config.get_span_links_level().into(),
                            self.exporter_config.get_span_links_keywords(),
                        ) {
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
                            span_data.attributes.iter(),
                            false,
                            true,
                            export_payload_as_json,
                        )
                    });
            }

            if self.exporter_config.get_export_common_schema_events() {
                let span_es = if let Some(es) = self.provider.find_set(
                    Level::Informational,
                    self.exporter_config.get_span_keywords(),
                ) {
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
        })
    }

    fn log_log_data(&self, log_data: &LogData) -> logs::ExportResult {
        Ok(())
    }
}
