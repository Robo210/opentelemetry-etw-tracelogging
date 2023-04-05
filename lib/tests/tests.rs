mod etw_helpers;

#[cfg(test)]
#[allow(dead_code, non_upper_case_globals)]
mod functional {
    use crate::etw_helpers::*;
    use futures::*;
    use opentelemetry::{Key, global::shutdown_tracer_provider, trace::{Span, SpanContext, Tracer, TraceContextExt, Link}};
    use tracelogging;
    use windows::{
        core::{GUID, PCSTR},
        s,
        Win32::System::Diagnostics::Etw::*,
    };

    const SAMPLE_KEY_STR: Key = Key::from_static_str("str");
    const SAMPLE_KEY_BOOL: Key = Key::from_static_str("bool");
    const SAMPLE_KEY_INT: Key = Key::from_static_str("int");
    const SAMPLE_KEY_FLOAT: Key = Key::from_static_str("float");

    const sz_test_session_name: PCSTR = s!("OpenTelemetry-Rust-ETW-Exporter-Tests");
    const test_provider_name: &str = "OpenTelemetry-Rust-ETW-Exporter-Test-Provider";
    const sz_test_provider_name: PCSTR = s!("OpenTelemetry-Rust-ETW-Exporter-Test-Provider");
    // 7930b11f-5f82-5871-bee2-c9bb0ad0711c
    const test_provider_id: windows::core::GUID =
        GUID::from_u128(161089410211316030591454656377708900636u128);

    // //////////////
    tracelogging::define_provider!(
        TEST_PROVIDER,
        "OpenTelemetry-Rust-ETW-Exporter-Test-Provider"
    );

    #[test]
    #[cfg(target_os = "windows")]
    fn log_event() -> Result<(), windows::core::Error> {
        let span_context: SpanContext = SpanContext::empty_context();

        let tracer = opentelemetry_etw::span_exporter::new_etw_exporter(test_provider_name)
            .with_common_schema_events()
            .without_normal_events()
            .install_realtime();

        let h = ControlTraceHandle::start_session(sz_test_session_name)?;
        h.enable_provider(&test_provider_id)?;

        let mut consumer = EtwEventConsumer2::new();
        let event_consumer = consumer.get_consumer();

        let trace = ProcessTraceHandle::from_session(sz_test_session_name, &event_consumer)?;

        // Logging events needs to be delayed enough from the call to enable_provider for the enable callback to arrive

        // Log 2 spans with 1 event on each.
        // Since we are only emitting Common Schema events, this should
        // turn into 2 ETW events.
        tracer.in_span("TestOuterSpan", |cx| {
            std::thread::sleep(std::time::Duration::from_millis(1000));

            let span = cx.span();
            span.add_event(
                "TestEventName",
                vec![SAMPLE_KEY_INT.i64(5), SAMPLE_KEY_FLOAT.f64(7.1)],
            );

            let link = Link::new(span_context, vec![SAMPLE_KEY_STR.string("link_attribute")]);

            let span_builder = tracer
                .span_builder("TestInnerSpan")
                .with_kind(opentelemetry::trace::SpanKind::Server)
                .with_links(vec![link])
                .with_status(opentelemetry::trace::Status::Ok);

            let mut span = tracer.build(span_builder);

            std::thread::sleep(std::time::Duration::from_millis(1000));
            span.add_event(
                "TestEvent2",
                vec![SAMPLE_KEY_BOOL.array(vec![false, true, false])],
            );
            std::thread::sleep(std::time::Duration::from_millis(1000));
        });

        shutdown_tracer_provider(); // sending remaining spans

        let fut = consumer.expect_event(|evt: &EVENT_RECORD| {
            if evt.EventHeader.ProviderId == test_provider_id {
                println!(
                    "Found event from provider! {}",
                    evt.EventHeader.EventDescriptor.Keyword
                );
                true
            } else {
                false
            }
        });
        let fut2 = consumer.expect_event(|evt: &EVENT_RECORD| {
            if evt.EventHeader.ProviderId == test_provider_id {
                println!(
                    "Found event from provider! {}",
                    evt.EventHeader.EventDescriptor.Keyword
                );
                true
            } else {
                false
            }
        });

        let mut thread = trace.process_trace()?;

        let result = futures::executor::block_on(fut.and_then(|_| fut2));

        let _ = thread.stop_and_wait(); // We don't care about what ProcessTrace returned

        result
    }
}
