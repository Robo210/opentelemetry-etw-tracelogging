mod etw_helpers;

#[cfg(test)]
#[allow(dead_code, non_upper_case_globals)]
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
        Ok(())
    }
}
