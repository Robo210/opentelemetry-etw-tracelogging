use std::{mem::MaybeUninit, io::Cursor};
use std::io::Write;

use opentelemetry_api::trace::{SpanId, TraceId};

pub(crate) struct Activities {
    pub(crate) span_id: [u8; 16],                    // Hex string
    pub(crate) activity_id: [u8; 16],                // Guid
    pub(crate) parent_activity_id: Option<[u8; 16]>, // Guid
    pub(crate) parent_span_id: [u8; 16],             // Hex string
    pub(crate) trace_id_name: [u8; 32],              // Hex string
}

impl Activities {
    #[allow(invalid_value)]
    pub(crate) fn generate(
        span_id: &SpanId,
        parent_span_id: &SpanId,
        trace_id: &TraceId,
    ) -> Activities {
        let mut activity_id: [u8; 16] = [0; 16];
        let (_, half) = activity_id.split_at_mut(8);
        half.copy_from_slice(&span_id.to_bytes());

        let (parent_activity_id, parent_span_name) = if *parent_span_id == SpanId::INVALID {
            (None, [0; 16])
        } else {
            let mut buf: [u8; 16] = unsafe { MaybeUninit::uninit().assume_init() };
            let mut cur = Cursor::new(&mut buf[..]);
            write!(&mut cur, "{:16x}", span_id).expect("!write");

            let mut activity_id: [u8; 16] = [0; 16];
            let (_, half) = activity_id.split_at_mut(8);
            half.copy_from_slice(&parent_span_id.to_bytes());
            (Some(activity_id), buf)
        };

        let mut buf: [u8; 16] = unsafe { MaybeUninit::uninit().assume_init() };
        let mut cur = Cursor::new(&mut buf[..]);
        write!(&mut cur, "{:16x}", span_id).expect("!write");

        let mut buf2: [u8; 32] = unsafe { MaybeUninit::uninit().assume_init() };
        let mut cur2 = Cursor::new(&mut buf2[..]);
        write!(&mut cur2, "{:32x}", trace_id).expect("!write");

        Activities {
            span_id: buf,
            activity_id,
            parent_activity_id,
            parent_span_id: parent_span_name,
            trace_id_name: buf2,
        }
    }
}
