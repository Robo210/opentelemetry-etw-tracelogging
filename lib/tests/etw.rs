#[cfg(test)]
#[allow(dead_code, non_upper_case_globals, unused_imports)]
mod functional {
    use std::ffi::c_void;

    use etw_helpers::*;
    use futures::future::Either;
    use futures::*;
    use opentelemetry::{
        global::shutdown_tracer_provider,
        trace::{Link, Span, SpanContext, TraceContextExt, Tracer},
        Key,
    };
    use rsevents::Awaitable;
    use tracelogging::{self, Guid, Level};
    use windows::{
        core::{GUID, PCSTR},
        s,
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

    // We don't log events from this, but we need to register it and get an enabled callback for it
    // so we don't start logging events before the test has enabled the provider.
    tracelogging::define_provider!(
        TEST_PROVIDER,
        "OpenTelemetry-Rust-ETW-Exporter-Test-Provider"
    );

    fn provider_enabled_callback(
        _source_id: &Guid,
        _event_control_code: u32,
        _level: Level,
        _match_any_keyword: u64,
        _match_all_keyword: u64,
        _filter_data: usize,
        callback_context: usize,
    ) {
        unsafe {
            let ctx = &*(callback_context as *const c_void as *const rsevents::ManualResetEvent);
            ctx.set();
        }
    }

    static log_common_schema_events_enabled_event: rsevents::ManualResetEvent =
        rsevents::ManualResetEvent::new(rsevents::EventState::Unset);

    #[test]
    #[cfg(target_os = "windows")]
    fn log_common_schema_events() -> Result<(), windows::core::Error> {
        unsafe {
            TEST_PROVIDER.register_with_callback(
                provider_enabled_callback,
                &log_common_schema_events_enabled_event as *const rsevents::ManualResetEvent
                    as usize,
            );
        }

        let span_context: SpanContext = SpanContext::empty_context();

        let tracer = opentelemetry_etw_user_events::new_exporter(test_provider_name)
            .with_common_schema_events()
            .without_realtime_events()
            .tracing()
            .install_span_exporter();

        let h = EtwSession::get_or_start_etw_session(sz_test_session_name, false)?;
        h.enable_provider(&test_provider_id)?;

        let mut consumer = EtwEventAsyncWaiter::new();
        let event_consumer = consumer.get_consumer();

        let trace = ProcessTraceHandle::from_session(sz_test_session_name, event_consumer)?;

        log_common_schema_events_enabled_event.wait();

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

        // Check the ETW events that were collected

        let fut = consumer.expect_event_async(|evt| {
            let event_header = evt.get_event_header();
            if event_header.ProviderId == test_provider_id {
                println!(
                    "Found event from provider! {}",
                    event_header.EventDescriptor.Keyword
                );
                // TODO: Actually verify the event contents
                true
            } else {
                false
            }
        });
        let fut2 = consumer.expect_event_async(|evt| {
            let event_header = evt.get_event_header();
            if event_header.ProviderId == test_provider_id {
                println!(
                    "Found event from provider! {}",
                    event_header.EventDescriptor.Keyword
                );
                // TODO: Actually verify the event contents
                true
            } else {
                false
            }
        });

        // Create a future that will either time out (success) or pick up another event (failure).
        // We only expect 2 ETW events at this point, so a 3rd event showing up is a problem.
        // This will need to be adjusted when Span Events are turned into ETW Events.
        let fut3 = consumer.expect_event_async(|_evt| {
            assert!(false, "Found unexpected third event");
            false
        });
        let fut4 = async {
            std::thread::sleep(std::time::Duration::from_millis(2000));
            Result::<(), windows::core::Error>::Ok(())
        };
        let fut5 = futures::future::select(Box::pin(fut3), Box::pin(fut4));

        // Assemble the final futures:
        // - fut and fut2 need to run and return Ok
        // - fut4 needs to complete with its timeout checking for extra events
        let fut6 = fut.and_then(|_| fut2);
        let fut7 = fut6.and_then(|_| async {
            match fut5.await {
                Either::Left(_) => Err(windows::core::HRESULT(-2147024662).into()), // HRESULT_FROM_WIN32(ERROR_MORE_DATA)
                Either::Right(_) => Ok(()),
            }
        });

        let mut thread = trace.process_trace()?;

        let result = futures::executor::block_on(fut7);

        let _ = thread.stop_and_wait(); // We don't care about what ProcessTrace returned

        result
    }
}
