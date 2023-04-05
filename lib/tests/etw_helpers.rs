use std::{
    cell::RefCell,
    ffi::c_void,
    marker::PhantomData,
    mem::ManuallyDrop,
    ops::Deref,
    panic::{catch_unwind, AssertUnwindSafe},
    pin::Pin,
    sync::{
        atomic::{self, AtomicPtr, Ordering},
        Arc, Condvar, Mutex, RwLock, Weak,
    },
    thread::JoinHandle,
    time::Duration,
};

use windows::{
    core::{PCSTR, PSTR},
    Win32::{Foundation::GetLastError, System::Diagnostics::Etw::*},
};

use futures::*;

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
            FlushTimer: 1,

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
    C: EventConsumer + Unpin + Send + Sync + 'a,
{
    consumer: Option<&'a C>,
    hndl: Option<PROCESSTRACE_HANDLE>,
}

impl<'a, C> InnerProcessTraceHandle<'a, C>
where
    C: EventConsumer + Unpin + Send + Sync + 'a,
{
    unsafe fn inner_callback(
        &self,
        event_record: *mut EVENT_RECORD,
    ) -> Result<(), windows::core::Error> {
        <C as EventConsumer>::on_event_raw(&self.consumer.as_ref().unwrap(), event_record)
    }

    fn process_trace_complete(&self, err: windows::core::Error) {
        <C as EventConsumer>::complete(&self.consumer.as_ref().unwrap(), err)
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
    C: EventConsumer + Unpin + Send + Sync + 'a,
{
    fn drop(&mut self) {
        self.close_trace()
    }
}

pub struct ProcessTraceHandle<'a, C>
where
    C: EventConsumer + Unpin + Send + Sync + 'a,
{
    inner: Box<InnerProcessTraceHandle<'a, C>>,
}

impl<'a, C> ProcessTraceHandle<'a, C>
where
    C: EventConsumer + Unpin + Send + Sync + 'a,
{
    pub fn from_session(
        session_name: PCSTR,
        consumer: &'a C,
    ) -> Result<ProcessTraceHandle<'a, C>, windows::core::Error> {
        unsafe {
            let mut log = EventTraceLogFile::from_session(
                session_name,
                Some(ProcessTraceHandle::<C>::event_record_callback),
            );
            let mut inner = Box::new(InnerProcessTraceHandle {
                consumer: Some(consumer),
                hndl: None,
            });
            let ptr = &mut *inner;
            log = log.set_user_context(ptr as *mut InnerProcessTraceHandle<'a, C> as *mut c_void);

            let hndl = OpenTraceA(&mut log.props);
            if hndl.0 == u64::MAX {
                let err = GetLastError();
                Err(err.into())
            } else {
                inner.hndl = Some(hndl);
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

    unsafe extern "system" fn event_record_callback(event_record: *mut EVENT_RECORD) {
        let ctx = (*event_record).UserContext as *mut InnerProcessTraceHandle<'a, C>;
        if ctx != core::ptr::null_mut() {
            // It's not safe to let a panic cross back into C code.
            // Use AssertUnwindSafe because we will always abort in the event of a panic.
            let err = catch_unwind(AssertUnwindSafe(|| {
                let result = (*ctx).inner_callback(event_record);
                if result.is_err() {
                    (*ctx).close_trace();
                }
            }));
            if err.is_err() {
                std::process::abort();
            }
        }
    }

    pub fn process_trace(mut self) -> Result<ProcessTraceThread<'a, C>, windows::core::Error> {
        let inner =
            self.inner.as_mut() as *const InnerProcessTraceHandle<'a, C> as *const c_void as usize;
        let thread = spawn_process_trace_thread(&self, inner);

        Ok(ProcessTraceThread {
            thread: Some(thread),
            inner: RwLock::new(self.inner),
        })
    }
}

fn spawn_process_trace_thread<'a, C>(
    real_inner: &ProcessTraceHandle<'a, C>,
    inner: usize,
) -> JoinHandle<Result<(), windows::core::Error>>
where
    C: EventConsumer + Unpin + Send + Sync + 'a,
{
    let handles = [real_inner.inner.hndl.expect("y")];
    unsafe {
        std::thread::spawn(move || {
            let err = ProcessTrace(&handles, None, None);
            let ctx = inner as *mut InnerProcessTraceHandle<'a, C>;
            if ctx != core::ptr::null_mut() {
                (*ctx).process_trace_complete(err.into());
            }
            if err.is_err() {
                Err(windows::core::Error::from(err))
            } else {
                Ok(())
            }
        })
    }
}

pub struct ProcessTraceThread<'a, C>
where
    C: EventConsumer + Unpin + Send + Sync + 'a,
{
    thread: Option<JoinHandle<Result<(), windows::core::Error>>>,
    inner: RwLock<Box<InnerProcessTraceHandle<'a, C>>>,
}

impl<'a, C> ProcessTraceThread<'a, C>
where
    C: EventConsumer + Unpin + Send + Sync + 'a,
{
    pub fn stop_and_wait(&mut self) -> Result<(), windows::core::Error> {
        let thread = self.stop_and_get_thread();
        thread.join().unwrap()
    }

    pub fn stop_and_get_thread(&mut self) -> JoinHandle<Result<(), windows::core::Error>> {
        let mut guard = self.inner.write().unwrap();
        if let Some(tracehandle) = guard.hndl.take() {
            unsafe {
                CloseTrace(tracehandle);
            }
        }
        self.thread.take().unwrap()
    }
}

pub trait EventConsumer {
    unsafe fn on_event_raw(&self, evt: *mut EVENT_RECORD) -> Result<(), windows::core::Error> {
        self.on_event(&(*evt))
    }

    fn on_event(&self, evt: &EVENT_RECORD) -> Result<(), windows::core::Error>;

    fn complete(&self, _err: windows::core::Error) {}
}

pub struct EtwEventConsumer<'a> {
    ready_for_next_event: atomic::AtomicBool,
    next_event_consumer_set: Condvar,
    waiter: Mutex<Option<Box<dyn Fn(&EVENT_RECORD) -> bool + Send + Sync + 'a>>>,

    event_callback_completed: Condvar,
    waiter2: Mutex<bool>,
}

impl<'a> EventConsumer for EtwEventConsumer<'a> {
    fn on_event(&self, evt: &EVENT_RECORD) -> Result<(), windows::core::Error> {
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
                if guard.is_none() {
                    let result = self
                        .next_event_consumer_set
                        .wait_timeout(guard, Duration::new(10, 0))
                        .unwrap();
                    if result.1.timed_out() {
                        println!("waiting for consume to release current event timed out");
                        return Err(windows::core::HRESULT(-2147023436i32).into());
                    // HRESULT_FROM_WIN32(ERROR_TIMEOUT)
                    } else {
                        guard = result.0;
                        break;
                    }
                }
            } else {
                guard = self.waiter.lock().unwrap();
                break;
            }
        }

        if let Some(f) = guard.take() {
            let should_continue = f(evt);
            *self.waiter2.lock().unwrap() = true;
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
        F: Fn(&EVENT_RECORD) -> bool + Send + Sync + 'a,
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
            panic!("Cannot await more than one call to expect_event at once");
        } else {
        }
        self.next_event_consumer_set.notify_one();

        {
            let guard = self.waiter2.lock().unwrap();
            // If *guard == true then the condition variable was notified before we were ready to wait on it,
            // so skip the wait so we don't deadlock.
            if *guard == false {
                let mut result = self
                    .event_callback_completed
                    .wait_timeout(guard, Duration::new(10, 0))
                    .unwrap();
                if result.1.timed_out() {
                    println!("waiting for next event timed out");
                    return Err::<(), windows::core::Error>(
                        windows::core::HRESULT(-2147023436i32).into(),
                    ); // HRESULT_FROM_WIN32(ERROR_TIMEOUT)
                }
                *result.0 = false;
            }
        }
        return Ok::<(), windows::core::Error>(());
    }
}

struct EtwEventStreamInner<'a> {
    waker: Mutex<Option<task::Waker>>,
    next_event: Mutex<AtomicPtr<EVENT_RECORD>>,
    consumer_complete: Arc<Condvar>,
    _x: PhantomData<&'a bool>,
}

impl<'a> EventConsumer for EtwEventStreamConsumer<'a> {
    unsafe fn on_event_raw(&self, evt: *mut EVENT_RECORD) -> Result<(), windows::core::Error> {
        let mut guard = self.inner.next_event.lock().unwrap();
        guard.store(evt, Ordering::Release);
        if let Some(w) = &*self.inner.waker.lock().unwrap() {
            w.wake_by_ref();
        }

        if evt != (12345 as *mut EVENT_RECORD) {
            loop {
                guard = self.inner.consumer_complete.wait(guard).unwrap();

                if guard.load(Ordering::Acquire) == core::ptr::null_mut() {
                    break;
                }
            }
        }

        Ok(())
    }

    fn on_event(&self, _evt: &EVENT_RECORD) -> Result<(), windows::core::Error> {
        Ok(())
    }

    fn complete(&self, _err: windows::core::Error) {
        unsafe {
            let _ = self.on_event_raw(12345 as *mut EVENT_RECORD);
        }
    }
}

impl<'a> Stream for EtwEventStreamExt<'a> {
    type Item = EventRecord<'a>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
    ) -> task::Poll<Option<Self::Item>> {
        *self.inner.waker.lock().unwrap() = Some(cx.waker().clone());
        let guard = self.inner.next_event.lock().unwrap();
        let ptr = guard.load(Ordering::Acquire);
        if ptr == core::ptr::null_mut() {
            task::Poll::Pending
        } else if ptr == (12345 as *mut EVENT_RECORD) {
            task::Poll::Ready(None)
        } else {
            unsafe {
                let evt = &*(guard.load(Ordering::Acquire));
                task::Poll::Ready(Some(EventRecord {
                    evt,
                    inner: Arc::downgrade(&self.inner),
                }))
            }
        }
    }
}

pub struct EtwEventStreamConsumer<'a> {
    inner: Arc<EtwEventStreamInner<'a>>,
}

pub struct EtwEventStreamExt<'a> {
    inner: Arc<EtwEventStreamInner<'a>>,
}

pub struct EtwEventStream<'a> {
    inner: Arc<EtwEventStreamInner<'a>>,
}

impl<'a> EtwEventStream<'a> {
    pub fn new() -> EtwEventStream<'a> {
        EtwEventStream {
            inner: Arc::new(EtwEventStreamInner {
                waker: Mutex::new(None),
                next_event: Mutex::new(AtomicPtr::new(core::ptr::null_mut())),
                consumer_complete: Arc::default(),
                _x: PhantomData,
            }),
        }
    }

    pub fn get_consumer(&self) -> impl EventConsumer + 'a {
        EtwEventStreamConsumer {
            inner: self.inner.clone(),
        }
    }

    pub fn get_stream(&self) -> impl Stream + 'a {
        EtwEventStreamExt {
            inner: self.inner.clone(),
        }
    }
}

pub struct EventRecord<'a> {
    evt: &'static EVENT_RECORD,
    inner: Weak<EtwEventStreamInner<'a>>,
}

impl<'a> Drop for EventRecord<'a> {
    fn drop(&mut self) {
        let strong = self.inner.upgrade();
        if let Some(strong) = strong {
            strong
                .next_event
                .lock()
                .unwrap()
                .store(core::ptr::null_mut(), Ordering::Release);
            strong.consumer_complete.notify_one();
        }
    }
}

impl<'a> Deref for EventRecord<'a> {
    type Target = EVENT_RECORD;

    fn deref(&self) -> &Self::Target {
        self.evt
    }
}

struct EtwEventConsumer2Consumer {
    tx: Mutex<futures::channel::mpsc::Sender<AtomicPtr<EVENT_RECORD>>>,
}

impl EventConsumer for EtwEventConsumer2Consumer {
    unsafe fn on_event_raw(&self, evt: *mut EVENT_RECORD) -> Result<(), windows::core::Error> {
        futures::executor::block_on(async {
            let send_result = self.tx.lock().unwrap().send(AtomicPtr::new(evt)).await;
            if send_result.is_err() {
                panic!();
            }
            Ok(())
        })
    }

    fn on_event(&self, _evt: &EVENT_RECORD) -> Result<(), windows::core::Error> {
        Ok(())
    }

    fn complete(&self, _err: windows::core::Error) {
        futures::executor::block_on(async {
            let _ = self.tx.lock().unwrap().close();
        });
    }
}

pub struct EtwEventConsumer2 {
    tx: Option<futures::channel::mpsc::Sender<AtomicPtr<EVENT_RECORD>>>,
    rx: RefCell<futures::channel::mpsc::Receiver<AtomicPtr<EVENT_RECORD>>>,
}

impl EtwEventConsumer2 {
    pub fn new() -> EtwEventConsumer2 {
        let (tx, rx) = futures::channel::mpsc::channel::<AtomicPtr<EVENT_RECORD>>(0);
        EtwEventConsumer2 {
            tx: Some(tx),
            rx: RefCell::new(rx),
        }
    }

    pub fn get_consumer(&mut self) -> impl EventConsumer {
        EtwEventConsumer2Consumer {
            tx: Mutex::new(self.tx.take().unwrap()),
        }
    }

    pub async fn expect_event<F>(&self, f: F) -> Result<(), windows::core::Error>
    where
        F: Fn(&EVENT_RECORD) -> bool + Send + Sync,
    {
        let next_event = self.rx.borrow_mut().next().await;
        let next_event_record = unsafe { &*(next_event.unwrap().load(Ordering::Acquire)) };

        let should_continue = f(next_event_record);
        if !should_continue {
            self.rx.borrow_mut().close();
            //return Err(windows::core::HRESULT(-2147023673).into()); // HRESULT_FROM_WIN32(ERROR_CANCELLED)
        }

        return Ok::<(), windows::core::Error>(());
    }
}

#[cfg(test)]
#[allow(dead_code, non_upper_case_globals)]
mod tests {
    use rsevents::Awaitable;
    use tracelogging::{Guid, Level};

    use super::*;

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

    static consume_event_enabled_event: rsevents::ManualResetEvent =
        rsevents::ManualResetEvent::new(rsevents::EventState::Unset);

    #[test]
    fn consume_event() -> Result<(), windows::core::Error> {
        const sz_test_name: PCSTR = windows::s!("EtwConsumer-Rust-Tests-ConsumeEvent");

        let mut options = tracelogging_dynamic::Provider::options();
        let options = options.callback(
            provider_enabled_callback,
            &consume_event_enabled_event as *const rsevents::ManualResetEvent as usize,
        );

        let provider = Box::pin(tracelogging_dynamic::Provider::new(
            "consume_event_test",
            &options,
        ));
        unsafe {
            provider.as_ref().register();
        }
        let provider_guid = windows::core::GUID::from_u128(provider.id().to_u128());
        let mut eb = tracelogging_dynamic::EventBuilder::new();

        let h = ControlTraceHandle::start_session(sz_test_name)?;
        h.enable_provider(&provider_guid)?;

        //let h2 = ControlTraceHandle::from_session(sz_test_name)?.manual_stop();

        let consumer = EtwEventConsumer::new();

        let trace = ProcessTraceHandle::from_session(sz_test_name, &consumer)?;

        consume_event_enabled_event.wait();

        eb.reset("test event", tracelogging::Level::LogAlways, 1, 0);
        eb.write(&provider, None, None);

        let fut = consumer.expect_event(|evt: &EVENT_RECORD| {
            if evt.EventHeader.ProviderId == provider_guid {
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

        let fut2 = consumer.expect_event(|_evt| {
            println!("yay second event");
            false
        });

        eb.reset("test event", tracelogging::Level::LogAlways, 2, 0);
        eb.write(&provider, None, None);

        let fut3 = fut.and_then(|_| fut2);

        let result = futures::executor::block_on(fut3);

        let _ = thread.stop_and_wait(); // We don't care about what ProcessTrace returned

        result
    }

    #[tokio::test]
    async fn stream_events() -> Result<(), windows::core::Error> {
        const sz_test_name: PCSTR = windows::s!("EtwConsumer-Rust-Tests-StreamEvent");

        let provider = Box::pin(tracelogging_dynamic::Provider::new(
            "stream_event_test",
            &tracelogging_dynamic::Provider::options(),
        ));
        unsafe {
            provider.as_ref().register();
        }
        let provider_guid = windows::core::GUID::from_u128(provider.id().to_u128());
        let mut eb = tracelogging_dynamic::EventBuilder::new();

        let h = ControlTraceHandle::start_session(sz_test_name)?;
        h.enable_provider(&provider_guid)?;

        //let h2 = ControlTraceHandle::from_session(sz_test_session_name)?.manual_stop();

        let etw_event_stream = EtwEventStream::new();
        let event_consumer = etw_event_stream.get_consumer();
        let event_stream = etw_event_stream.get_stream();

        let trace = ProcessTraceHandle::from_session(sz_test_name, &event_consumer)?;

        let mut thread = trace.process_trace()?;

        let mut events = event_stream.enumerate().fuse();

        eb.write(&provider, None, None);
        eb.write(&provider, None, None);
        eb.write(&provider, None, None);

        let mut process_trace_thread = None;
        let mut count = 0;
        while let Some(_evt) = events.next().await {
            count += 1;
            println!("Yay! {count}");

            if count == 3 {
                process_trace_thread = Some(thread.stop_and_get_thread());
            }
        }

        let _ = process_trace_thread.expect("x").join(); // We don't care about what ProcessTrace returned

        Ok(())
    }

    static consume_event2_enabled_event: rsevents::ManualResetEvent =
        rsevents::ManualResetEvent::new(rsevents::EventState::Unset);

    #[test]
    fn consume_event2() -> Result<(), windows::core::Error> {
        const sz_test_name: PCSTR = windows::s!("EtwConsumer-Rust-Tests-ConsumeEvent2");

        let mut options = tracelogging_dynamic::Provider::options();
        let options = options.callback(
            provider_enabled_callback,
            &consume_event2_enabled_event as *const rsevents::ManualResetEvent as usize,
        );

        let provider = Box::pin(tracelogging_dynamic::Provider::new(
            "consume_event_test2",
            &options,
        ));
        unsafe {
            provider.as_ref().register();
        }
        let provider_guid = windows::core::GUID::from_u128(provider.id().to_u128());
        let mut eb = tracelogging_dynamic::EventBuilder::new();

        let h = ControlTraceHandle::start_session(sz_test_name)?;
        //let h = ControlTraceHandle::from_session(sz_test_name)?.manual_stop();
        h.enable_provider(&provider_guid)?;

        let mut consumer = EtwEventConsumer2::new();
        let event_consumer = consumer.get_consumer();

        let trace = ProcessTraceHandle::from_session(sz_test_name, &event_consumer)?;

        consume_event2_enabled_event.wait();

        eb.reset("test event", tracelogging::Level::LogAlways, 1, 0);
        eb.write(&provider, None, None);

        let fut = consumer.expect_event(|evt: &EVENT_RECORD| {
            if evt.EventHeader.ProviderId == provider_guid {
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

        let fut2 = consumer.expect_event(|_evt| {
            println!("yay second event");
            false
        });

        eb.reset("test event", tracelogging::Level::LogAlways, 2, 0);
        eb.write(&provider, None, None);

        let fut3 = fut.and_then(|_| fut2);

        let result = futures::executor::block_on(fut3);

        let _ = thread.stop_and_wait(); // We don't care about what ProcessTrace returned

        result
    }
}
