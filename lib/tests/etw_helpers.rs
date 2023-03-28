use std::{
    ffi::c_void,
    mem::ManuallyDrop,
    pin::Pin,
    sync::{atomic, Condvar, Mutex},
    thread::JoinHandle,
    time::Duration,
};

use windows::{
    core::{HRESULT, PCSTR, PSTR},
    Win32::{Foundation::GetLastError, System::Diagnostics::Etw::*},
};

#[repr(C)]
pub struct EventTraceProperties {
    props: EVENT_TRACE_PROPERTIES,
    file_name: [u8; 1024],
    session_name: [u8; 1024],
}

impl EventTraceProperties {
    pub fn new(for_query: bool) -> EventTraceProperties {
        unsafe {
            let mut props: EventTraceProperties = core::mem::zeroed();
            props.props.Wnode.BufferSize = core::mem::size_of::<Self>() as u32;
            props.props.Wnode.Flags = WNODE_FLAG_TRACED_GUID;

            if for_query {
                props.props.LoggerNameOffset =
                    core::mem::size_of::<EVENT_TRACE_PROPERTIES>() as u32;
                props.props.LogFileNameOffset =
                    core::mem::size_of::<EVENT_TRACE_PROPERTIES>() as u32 + 1024;
            }

            props
        }
    }

    pub fn set_session_name(&mut self, session_name: PCSTR) -> &mut Self {
        if !session_name.is_null() {
            unsafe {
                let len = windows::core::strlen(session_name) + 1;
                if len > 1024 {
                    panic!()
                }

                core::ptr::copy_nonoverlapping(
                    session_name.as_ptr(),
                    self.session_name.as_mut_ptr(),
                    len,
                );
                self.props.LoggerNameOffset = core::mem::size_of::<EVENT_TRACE_PROPERTIES>() as u32;
            }
        }

        self
    }

    #[allow(dead_code)]
    pub fn set_file_name(&mut self, file_name: PCSTR) -> &mut Self {
        if !file_name.is_null() {
            unsafe {
                let len = windows::core::strlen(file_name) + 1;
                if len > 1024 {
                    panic!()
                }

                core::ptr::copy_nonoverlapping(
                    file_name.as_ptr(),
                    self.file_name.as_mut_ptr(),
                    len,
                );
                self.props.LogFileNameOffset =
                    core::mem::size_of::<EVENT_TRACE_PROPERTIES>() as u32 + 1024;
            }
        }

        self
    }
}

#[repr(C)]
pub struct EventTraceLogFile {
    props: EVENT_TRACE_LOGFILEA,
    name: [u8; 1024],
}

impl EventTraceLogFile {
    pub fn from_session(
        session_name: PCSTR,
        callback: PEVENT_RECORD_CALLBACK,
    ) -> EventTraceLogFile {
        unsafe {
            if session_name.is_null() {
                panic!()
            }

            let mut props: EventTraceLogFile = core::mem::zeroed();
            props.props.Anonymous1.ProcessTraceMode =
                PROCESS_TRACE_MODE_EVENT_RECORD | PROCESS_TRACE_MODE_REAL_TIME;
            props.props.Anonymous2.EventRecordCallback = callback;

            let len = windows::core::strlen(session_name) + 1;
            if len > 1024 {
                panic!()
            }

            core::ptr::copy_nonoverlapping(session_name.as_ptr(), props.name.as_mut_ptr(), len);
            props.props.LoggerName = PSTR::from_raw(props.name.as_mut_ptr());

            props
        }
    }

    #[allow(dead_code)]
    pub fn from_file(file_name: PCSTR, callback: PEVENT_RECORD_CALLBACK) -> EventTraceLogFile {
        unsafe {
            if file_name.is_null() {
                panic!()
            }

            let mut props: EventTraceLogFile = core::mem::zeroed();
            props.props.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_EVENT_RECORD;
            props.props.Anonymous2.EventRecordCallback = callback;

            let len = windows::core::strlen(file_name) + 1;
            if len > 1024 {
                panic!()
            }

            core::ptr::copy_nonoverlapping(file_name.as_ptr(), props.name.as_mut_ptr(), len);
            props.props.LogFileName = PSTR::from_raw(props.name.as_mut_ptr());

            props
        }
    }

    fn set_user_context(mut self, ctx: *mut c_void) -> Self {
        self.props.Context = ctx;

        self
    }
}

pub struct ControlTraceHandle(CONTROLTRACE_HANDLE);

impl Drop for ControlTraceHandle {
    fn drop(&mut self) {
        let mut props = EventTraceProperties::new(true);
        unsafe {
            let ptr = &mut props.props as *mut EVENT_TRACE_PROPERTIES;
            let _ = StopTraceA(self.0, PCSTR::null(), ptr);
        }
    }
}

impl ControlTraceHandle {
    pub fn start_session(
        sz_session_name: PCSTR,
    ) -> Result<ControlTraceHandle, windows::core::Error> {
        let mut session_handle: CONTROLTRACE_HANDLE = Default::default();
        let mut properties = EventTraceProperties::new(false);
        properties.set_session_name(sz_session_name);
        properties.props = EVENT_TRACE_PROPERTIES {
            Wnode: WNODE_HEADER {
                ClientContext: 1,

                ..properties.props.Wnode
            },
            BufferSize: 64,
            MinimumBuffers: 4,
            MaximumBuffers: 4,
            LogFileMode: EVENT_TRACE_FILE_MODE_NONE | EVENT_TRACE_REAL_TIME_MODE,
            NumberOfBuffers: 4,

            ..properties.props
        };

        unsafe {
            let ptr = &mut properties.props as *mut EVENT_TRACE_PROPERTIES;
            let err = StartTraceA(&mut session_handle, sz_session_name, ptr);

            if err.is_err() {
                Err(err.into())
            } else {
                Ok(ControlTraceHandle(session_handle))
            }
        }
    }

    #[allow(dead_code)]
    pub fn from_session(
        sz_session_name: PCSTR,
    ) -> Result<ControlTraceHandle, windows::core::Error> {
        unsafe {
            let mut properties = EventTraceProperties::new(true);
            let err = ControlTraceA(
                CONTROLTRACE_HANDLE::default(),
                sz_session_name,
                &mut properties.props,
                EVENT_TRACE_CONTROL_QUERY,
            );
            if err.is_err() {
                Err(err.into())
            } else {
                Ok(ControlTraceHandle(CONTROLTRACE_HANDLE(
                    properties.props.Wnode.Anonymous1.HistoricalContext,
                )))
            }
        }
    }

    #[allow(dead_code)]
    pub fn manual_stop(self) -> ManuallyDrop<Self> {
        ManuallyDrop::new(self)
    }

    pub fn enable_provider(
        &self,
        provider_id: &windows::core::GUID,
    ) -> Result<(), windows::core::Error> {
        unsafe {
            let err = EnableTraceEx2(
                self.0,
                provider_id,
                EVENT_CONTROL_CODE_ENABLE_PROVIDER.0,
                0xFF,
                0,
                0,
                0,
                None,
            );
            if err.is_err() {
                Err(err.into())
            } else {
                Ok(())
            }
        }
    }

    #[allow(dead_code)]
    pub fn disable_provider(
        &self,
        provider_id: &windows::core::GUID,
    ) -> Result<(), windows::core::Error> {
        unsafe {
            let err = EnableTraceEx2(
                self.0,
                provider_id,
                EVENT_CONTROL_CODE_DISABLE_PROVIDER.0,
                0xFF,
                0,
                0,
                0,
                None,
            );
            if err.is_err() {
                Err(err.into())
            } else {
                Ok(())
            }
        }
    }
}

struct InnerProcessTraceHandle<'a, C>
where
    C: EventConsumer + Unpin + 'a,
{
    consumer: Option<&'a C>,
    hndl: Option<PROCESSTRACE_HANDLE>,
}

impl<'a, C> InnerProcessTraceHandle<'a, C>
where
    C: EventConsumer + Unpin + 'a,
{
    fn inner_callback(&self, event_record: &EVENT_RECORD) -> Result<(), windows::core::Error> {
        println!("Inner!");
        <C as EventConsumer>::on_event(&self.consumer.as_ref().unwrap(), event_record)
    }

    fn close_trace(&mut self) {
        unsafe {
            if self.hndl.is_some() {
                CloseTrace(self.hndl.unwrap());
            }
            self.hndl = None;
        }
    }
}

impl<'a, C> Drop for InnerProcessTraceHandle<'a, C>
where
    C: EventConsumer + Unpin + 'a,
{
    fn drop(&mut self) {
        self.close_trace()
    }
}

pub struct ProcessTraceHandle<'a, C>
where
    C: EventConsumer + Unpin + 'a,
{
    inner: Pin<Box<InnerProcessTraceHandle<'a, C>>>,
}

impl<'a, C> ProcessTraceHandle<'a, C>
where
    C: EventConsumer + Unpin + 'a,
{
    unsafe extern "system" fn event_record_callback(event_record: *mut EVENT_RECORD) {
        println!("Event!");

        let ctx = (*event_record).UserContext as *mut InnerProcessTraceHandle<'a, C>;
        if ctx != core::ptr::null_mut() {
            let result = (*ctx).inner_callback(&(*event_record));
            if result.is_err() {
                (*ctx).close_trace();
            }
        }
    }

    pub fn from_session(
        session_name: PCSTR,
        consumer: &'a C,
    ) -> Result<ProcessTraceHandle<'a, C>, windows::core::Error> {
        unsafe {
            let mut log = EventTraceLogFile::from_session(
                session_name,
                Some(ProcessTraceHandle::<C>::event_record_callback),
            );
            let mut inner = Box::pin(InnerProcessTraceHandle {
                consumer: Some(consumer),
                hndl: None,
            });
            let ptr = &mut (*inner.as_mut());
            log = log.set_user_context(ptr as *mut InnerProcessTraceHandle<'a, C> as *mut c_void);

            let hndl = OpenTraceA(&mut log.props);
            if hndl.0 == u64::MAX {
                let err = GetLastError();
                Err(err.into())
            } else {
                inner.as_mut().hndl = Some(hndl);
                Ok(ProcessTraceHandle { inner })
            }
        }
    }

    // pub fn from_file(file_name: &str) -> Result<ProcessTraceHandle, windows::core::Error> {
    //     unsafe {
    //         let name = PCSTR::from_raw(file_name.as_ptr());
    //         let mut log = EventTraceLogFile::from_file(name, Some(ProcessTraceHandle::event_record_callback));

    //         let hndl = OpenTraceA(&mut log.props);
    //         if hndl.0 == 0 {
    //             let err = GetLastError();
    //             Err(err.into())
    //         } else {
    //             Ok(ProcessTraceHandle{hndl: Box::pin(hndl)})
    //         }
    //     }
    // }

    pub fn process_trace(self) -> Result<ProcessTraceThread<'a, C>, windows::core::Error> {
        unsafe {
            if self.inner.hndl.is_none() {
                panic!();
            }

            let handles = [self.inner.hndl.unwrap()];
            let thread = std::thread::spawn(move || {
                let err = ProcessTrace(&handles, None, None);
                if err.is_err() {
                    println!("Error {}", err.0);
                }
            });

            Ok(ProcessTraceThread {
                thread,
                inner: self.inner,
            })
        }
    }
}

pub struct ProcessTraceThread<'a, C>
where
    C: EventConsumer + Unpin + 'a,
{
    thread: JoinHandle<()>,
    inner: Pin<Box<InnerProcessTraceHandle<'a, C>>>,
}

impl<'a, C> ProcessTraceThread<'a, C>
where
    C: EventConsumer + Unpin + 'a,
{
    pub fn stop(self) {
        unsafe {
            CloseTrace(self.inner.hndl.unwrap());
        }
    }

    pub fn wait(self) {
        let _ = self.thread.join();
    }

    pub fn stop_and_wait(self) {
        unsafe {
            CloseTrace(self.inner.hndl.unwrap());
        }
        let _ = self.thread.join();
    }
}

pub trait EventConsumer {
    fn on_event(&self, evt: &EVENT_RECORD) -> Result<(), windows::core::Error>;
}

pub struct EtwEventConsumer<'a> {
    ready_for_next_event: atomic::AtomicBool,
    next_event_consumer_set: Condvar,
    waiter: Mutex<Option<Box<dyn Fn(&EVENT_RECORD) -> bool + 'a>>>,

    event_callback_completed: Condvar,
    waiter2: Mutex<bool>,
}

impl<'a> EventConsumer for EtwEventConsumer<'a> {
    fn on_event(&self, evt: &EVENT_RECORD) -> Result<(), windows::core::Error> {
        println!("Yay!");

        let mut guard;
        loop {
            let event_consumer_ready = self.ready_for_next_event.compare_exchange(
                true,
                false,
                atomic::Ordering::Acquire,
                atomic::Ordering::Relaxed,
            );
            if event_consumer_ready.is_err() {
                guard = self.waiter.lock().unwrap();
                let result = self
                    .next_event_consumer_set
                    .wait_timeout(guard, Duration::new(10, 0))
                    .unwrap();
                if result.1.timed_out() {
                    println!("timed out");
                    return Err(windows::core::HRESULT(-2147023436i32).into()); // HRESULT_FROM_WIN32(ERROR_TIMEOUT)
                } else {
                    guard = result.0;
                    break;
                }
            } else {
                guard = self.waiter.lock().unwrap();
                break;
            }
        }

        if let Some(f) = &*guard {
            let should_continue = f(evt);
            self.event_callback_completed.notify_one();
            if !should_continue {
                return Err(windows::core::HRESULT(-2147023673).into()); // HRESULT_FROM_WIN32(ERROR_CANCELLED)
            } else {
                return Ok(());
            }
        }

        Ok(())
    }
}

impl<'a> EtwEventConsumer<'a> {
    pub fn new() -> EtwEventConsumer<'a> {
        EtwEventConsumer {
            ready_for_next_event: atomic::AtomicBool::new(false),
            next_event_consumer_set: Condvar::new(),
            waiter: Mutex::new(None),
            event_callback_completed: Condvar::new(),
            waiter2: Mutex::new(false),
        }
    }

    pub async fn expect_event<F>(&self, f: F) -> Result<(), windows::core::Error>
    where
        F: Fn(&EVENT_RECORD) -> bool + 'a,
    {
        {
            let mut guard = self.waiter.lock().unwrap();
            *guard = Some(Box::new(f));
        }

        let ready = self.ready_for_next_event.compare_exchange(
            false,
            true,
            atomic::Ordering::Acquire,
            atomic::Ordering::Relaxed,
        );
        if ready.is_err() {
            panic!("Cannot call expect event twice");
        } else {
        }
        self.next_event_consumer_set.notify_one();

        {
            let guard = self.waiter2.lock().unwrap();
            let result = self
                .event_callback_completed
                .wait_timeout(guard, Duration::new(10, 0))
                .unwrap();
            if result.1.timed_out() {
                println!("timed out 2");
                return Err::<(), windows::core::Error>(
                    windows::core::HRESULT(-2147023436i32).into(),
                ); // HRESULT_FROM_WIN32(ERROR_TIMEOUT)
            }
        }
        return Ok::<(), windows::core::Error>(());
    }
}
