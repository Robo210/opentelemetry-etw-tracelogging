#[cfg(test)]
mod functional {
    use std::ops::{Deref, DerefMut};

    use windows::{Win32::{System::Diagnostics::Etw::*, Foundation::WIN32_ERROR}, s, core::{PCSTR, GUID}};
    use tracelogging;

    const test_session_name: &str = "OpenTelemetry-Rust-ETW-Exporter-Tests";
    const sz_test_session_name: PCSTR = s!("OpenTelemetry-Rust-ETW-Exporter-Tests");
    const test_provider_name: &str = "OpenTelemetry-Rust-ETW-Exporter-Test-Provider";
    const sz_test_provider_name: PCSTR = s!("OpenTelemetry-Rust-ETW-Exporter-Test-Provider");
    // 7930b11f-5f82-5871-bee2-c9bb0ad0711c
    const test_provider_id: GUID = GUID::from_u128(161089410211316030591454656377708900636u128);

    #[repr(C)]
    struct EventTraceProperties {
        props: EVENT_TRACE_PROPERTIES,
        file_name: [u8; 1024],
        session_name: [u8; 1024]
    }

    impl EventTraceProperties {
        fn new(for_query: bool) -> EventTraceProperties {
            unsafe {
                let mut props: EventTraceProperties = core::mem::zeroed();
                props.props.Wnode.BufferSize = core::mem::size_of::<Self>() as u32;
                props.props.Wnode.Flags = WNODE_FLAG_TRACED_GUID;

                if for_query {
                    props.props.LoggerNameOffset = core::mem::size_of::<EVENT_TRACE_PROPERTIES>() as u32;
                    props.props.LogFileNameOffset = core::mem::size_of::<EVENT_TRACE_PROPERTIES>() as u32 + 1024;
                }

                props
            }
        }

        fn set_session_name(&mut self, session_name: &str) -> &mut Self {
            if session_name.len() > 1024 {
                panic!()
            }

            if !session_name.is_empty() {
                unsafe {
                    core::ptr::copy_nonoverlapping(session_name.as_ptr(), self.session_name.as_mut_ptr(), session_name.as_bytes().len());
                    self.props.LoggerNameOffset = core::mem::size_of::<EVENT_TRACE_PROPERTIES>() as u32;
                }
            }

            self
        }

        fn set_file_name(&mut self, file_name: &str) -> &mut Self {
            if file_name.len() > 1024 {
                panic!()
            }
            
            if !file_name.is_empty() {
                unsafe {
                    core::ptr::copy_nonoverlapping(file_name.as_ptr(), self.file_name.as_mut_ptr(), file_name.as_bytes().len());
                    self.props.LogFileNameOffset = core::mem::size_of::<EVENT_TRACE_PROPERTIES>() as u32 + 1024;
                }
            }

            self
        }
    }

    struct ControlTraceHandle(CONTROLTRACE_HANDLE);

    impl Drop for ControlTraceHandle {
        fn drop(&mut self) {
            let mut props = EventTraceProperties::new(true);
            unsafe {
                let ptr = &mut props.props as *mut EVENT_TRACE_PROPERTIES;
                let _ = StopTraceA(self.0, PCSTR::null(), ptr);
            }
        }
    }

    fn start_session() -> Result<ControlTraceHandle, windows::core::Error> {
        let mut session_handle: CONTROLTRACE_HANDLE = Default::default();
        let mut properties = EventTraceProperties::new(false);
        properties.set_session_name(test_session_name);
        properties.props = EVENT_TRACE_PROPERTIES {
            Wnode: WNODE_HEADER {
                ClientContext: 1,
                Guid: test_provider_id,

                .. properties.props.Wnode
            },
            BufferSize: 64,
            MinimumBuffers: 4,
            MaximumBuffers: 4,
            LogFileMode: EVENT_TRACE_BUFFERING_MODE | EVENT_TRACE_REAL_TIME_MODE,
            NumberOfBuffers: 4,

            .. properties.props
        };

        unsafe {
            let ptr = &mut properties.props as *mut EVENT_TRACE_PROPERTIES;
            let win32 = StartTraceA(&mut session_handle, sz_test_session_name, ptr);

            if win32.is_err() {
                Err(win32.into())
            } else {
                Ok(ControlTraceHandle(session_handle))
            }
        }
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn log_event() -> Result<(), windows::core::Error> {
        let h = start_session();
        match &h {
            Ok(hndl) => println!("Session started"),
            Err(e) => return Err(e.clone())
        }

        Ok(())
    }
}
