//! Narrow, versioned Rust boundary for the in-process TSDuck C++ bridge.
//! No TSDuck command-line process is started.

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct Snapshot {
    pub packets: u64,
    pub continuity_errors: u64,
    pub valid_sections: u64,
    pub pat_sections: u64,
    pub pmt_sections: u64,
    pub standards: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct VideoSps {
    pub width: u32,
    pub height: u32,
    pub frame_rate_numerator: u32,
    pub frame_rate_denominator: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidInput,
    NativeFailure,
    UnalignedPacket,
}

#[cfg(feature = "native")]
mod native {
    use super::{Error, Snapshot, VideoSps};
    use std::marker::PhantomData;
    use std::ptr::NonNull;
    use std::rc::Rc;

    #[repr(C)]
    struct OpaqueSession {
        _private: [u8; 0],
    }

    unsafe extern "C" {
        fn tsan_tsduck_parse_sps(
            stream_type: u8,
            nal: *const u8,
            length: usize,
            out: *mut VideoSps,
        ) -> i32;
        fn tsan_tsduck_create(session: *mut *mut OpaqueSession) -> i32;
        fn tsan_tsduck_feed(session: *mut OpaqueSession, data: *const u8, length: usize) -> i32;
        fn tsan_tsduck_snapshot(session: *const OpaqueSession, snapshot: *mut Snapshot) -> i32;
        fn tsan_tsduck_destroy(session: *mut OpaqueSession);
    }

    fn status(code: i32) -> Result<(), Error> {
        match code {
            0 => Ok(()),
            -1 => Err(Error::InvalidInput),
            -3 => Err(Error::UnalignedPacket),
            _ => Err(Error::NativeFailure),
        }
    }

    pub fn parse_sps(stream_type: u8, nal: &[u8]) -> Option<VideoSps> {
        let mut result = VideoSps::default();
        // SAFETY: The input slice and output struct remain valid during this call.
        let code =
            unsafe { tsan_tsduck_parse_sps(stream_type, nal.as_ptr(), nal.len(), &mut result) };
        (code == 0).then_some(result)
    }

    pub struct Session {
        handle: NonNull<OpaqueSession>,
        pending: Vec<u8>,
        locked: bool,
        // DuckContext and its demuxers are single-threaded.
        _not_send_or_sync: PhantomData<Rc<()>>,
    }

    impl Session {
        pub fn new() -> Result<Self, Error> {
            let mut handle = std::ptr::null_mut();
            // SAFETY: The bridge initializes the out pointer or returns an error.
            status(unsafe { tsan_tsduck_create(&mut handle) })?;
            let handle = NonNull::new(handle).ok_or(Error::NativeFailure)?;
            Ok(Self {
                handle,
                pending: Vec::new(),
                locked: false,
                _not_send_or_sync: PhantomData,
            })
        }

        pub fn feed_aligned(&mut self, packets: &[u8]) -> Result<(), Error> {
            if packets.len() % 188 != 0 || packets.chunks_exact(188).any(|packet| packet[0] != 0x47)
            {
                return Err(Error::UnalignedPacket);
            }
            // SAFETY: The session is live, and the immutable slice remains valid for the call.
            status(unsafe {
                tsan_tsduck_feed(self.handle.as_ptr(), packets.as_ptr(), packets.len())
            })
        }

        /// Accepts arbitrarily split 188-byte TS data, including split GstBuffer boundaries.
        /// Callers handling M2TS or RS-204 first remove their 4/16 wrapper bytes.
        pub fn feed_bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
            self.pending.extend_from_slice(bytes);
            loop {
                if !self.locked {
                    if self.pending.len() < 376 {
                        return Ok(());
                    }
                    let sync = (0..=self.pending.len() - 376).find(|&index| {
                        self.pending[index] == 0x47 && self.pending[index + 188] == 0x47
                    });
                    match sync {
                        Some(index) => {
                            self.pending.drain(..index);
                            self.locked = true;
                        }
                        None => {
                            let keep = self.pending.len() - 375;
                            self.pending.drain(..keep);
                            return Ok(());
                        }
                    }
                }
                let complete = self.pending.len() / 188;
                if complete == 0 {
                    return Ok(());
                }
                let valid = self
                    .pending
                    .chunks_exact(188)
                    .take(complete)
                    .take_while(|packet| packet[0] == 0x47)
                    .count();
                if valid == 0 {
                    self.pending.drain(..1);
                    self.locked = false;
                    continue;
                }
                let length = valid * 188;
                // SAFETY: The bridge borrows this contiguous buffer only for the call.
                status(unsafe {
                    tsan_tsduck_feed(self.handle.as_ptr(), self.pending.as_ptr(), length)
                })?;
                self.pending.drain(..length);
                if valid < complete {
                    self.locked = false;
                    continue;
                }
                return Ok(());
            }
        }

        pub fn snapshot(&self) -> Result<Snapshot, Error> {
            let mut snapshot = Snapshot::default();
            // SAFETY: Both pointers are valid for the duration of the call.
            status(unsafe { tsan_tsduck_snapshot(self.handle.as_ptr(), &mut snapshot) })?;
            Ok(snapshot)
        }
    }

    impl Drop for Session {
        fn drop(&mut self) {
            // SAFETY: This handle came from create and is destroyed exactly once.
            unsafe { tsan_tsduck_destroy(self.handle.as_ptr()) };
        }
    }
}

#[cfg(feature = "native")]
pub use native::{Session, parse_sps};

#[cfg(all(test, feature = "native"))]
mod tests {
    use super::{Session, parse_sps};

    #[test]
    fn malformed_sps_returns_error_without_crossing_ffi() {
        assert!(parse_sps(0x24, &[0x42, 0x01]).is_none());
    }

    #[test]
    fn accepts_split_ts_packet_buffers() {
        let mut session = Session::new().expect("create native TSDuck session");
        let mut packet = [0xff; 188];
        packet[..4].copy_from_slice(&[0x47, 0x1f, 0xff, 0x10]);
        let mut stream = Vec::from([0x00, 0x01, 0x02]);
        stream.extend_from_slice(&packet);
        stream.extend_from_slice(&packet);
        stream.extend_from_slice(&packet);
        session.feed_bytes(&stream[..400]).expect("first buffer");
        session
            .feed_bytes(&stream[400..451])
            .expect("second buffer");
        session.feed_bytes(&stream[451..]).expect("third buffer");
        assert_eq!(session.snapshot().expect("snapshot").packets, 3);
    }
}
