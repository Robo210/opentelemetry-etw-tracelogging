use tracelogging_dynamic::*;

/// {e60ec51a-8e54-5a4f-2fb260a4f9213b3a}
/// Events in this group were (re)logged from OpenTelemetry.
/// Use the event tags and field tags to properly interpret these events.
pub const GROUP_ID: Guid = Guid::from_fields(
    0xe60ec51a,
    0x8e54,
    0x5a4f,
    [0x2f, 0xb2, 0x60, 0xa4, 0xf9, 0x21, 0x3b, 0x3a],
);

pub const GROUP_NAME: &str = "asdf";

/// The ETW event's timestamp is not meaningful.
/// Use the field tags to find the timestamp value to use.
pub const EVENT_TAG_IGNORE_EVENT_TIME: u32 = 12345;
/// This field contains the actual timestamp of the event.
pub const FIELD_TAG_IS_REAL_EVENT_TIME: u32 = 98765;
