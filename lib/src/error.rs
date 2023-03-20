#[derive(Debug)]
pub struct Error {
    pub win32err: u32,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("Win32 error: {}", self.win32err))
    }
}
impl std::error::Error for Error {}

impl opentelemetry::ExportError for Error {
    fn exporter_name(&self) -> &'static str {
        "TraceLogging"
    }
}
