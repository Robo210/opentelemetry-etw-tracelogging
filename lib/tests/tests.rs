mod etw_helpers;

#[cfg(test)]
mod functional {
    use crate::etw_helpers::*;
    use futures::TryFutureExt;
    use tracelogging;
    use windows::{
        core::{GUID, PCSTR},
        s,
        Win32::System::Diagnostics::Etw::*,
    };

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
        let h = ControlTraceHandle::start_session(sz_test_session_name)?;
        h.enable_provider(&test_provider_id)?;

        //let h2 = ControlTraceHandle::from_session(sz_test_session_name)?.manual_stop();

        let consumer = EtwEventConsumer::new();

        let trace = ProcessTraceHandle::from_session(sz_test_session_name, &consumer)?;

        unsafe {
            TEST_PROVIDER.register();
        }

        tracelogging::write_event!(TEST_PROVIDER, "test event", level(5));

        let fut = consumer.expect_event(|evt: &EVENT_RECORD| {
            if evt.EventHeader.ProviderId == test_provider_id {
                println!("Found event from provider!");
                true
            } else {
                false
            }
        });

        let thread = trace.process_trace()?;

        let fut2 = consumer.expect_event(|_evt| false);

        let fut3 = fut.and_then(|_| fut2);

        let result = futures::executor::block_on(fut3);
        if result.is_err() {
            thread.stop_and_wait();
            return Ok(()); // TODO: Remove this once fut2 actually does something
        } else {
            thread.wait();
        }

        println!("done");

        result
    }
}
