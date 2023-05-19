#[derive(Debug)]
pub struct Win32Error {
    pub win32err: u32,
}

impl std::fmt::Display for Win32Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("Win32 error: {}", self.win32err))
    }
}
impl std::error::Error for Win32Error {}

impl opentelemetry::ExportError for Win32Error {
    fn exporter_name(&self) -> &'static str {
        "ETW TraceLogging"
    }
}

#[derive(Debug)]
pub struct LinuxError {
    pub err: i32,
}

impl std::fmt::Display for LinuxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("Linux error: {}", self.err))
    }
}
impl std::error::Error for LinuxError {}

impl opentelemetry::ExportError for LinuxError {
    fn exporter_name(&self) -> &'static str {
        "UserEvents TraceLogging"
    }
}
