//! Streaming Annex-B picture inspection. Stores headers, not compressed pictures.
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PictureType {
    Idr,
    Cra,
    Bla,
    I,
    P,
    B,
    Unknown,
}
impl PictureType {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Idr => "IDR",
            Self::Cra => "CRA",
            Self::Bla => "BLA",
            Self::I => "I",
            Self::P => "P",
            Self::B => "B",
            Self::Unknown => "?",
        }
    }
    pub const fn random_access(self) -> bool {
        matches!(self, Self::Idr | Self::Cra | Self::Bla)
    }
    pub const fn intra(self) -> bool {
        self.random_access() || matches!(self, Self::I)
    }
}

#[derive(Clone, Debug)]
pub struct VideoFrame {
    pub packet: u64,
    pub pes_packet: u64,
    pub pts: Option<u64>,
    pub picture_type: PictureType,
    pub damaged: bool,
    /// Compressed VCL NAL bytes, including NAL headers; excludes start codes and TS/PES overhead.
    pub vcl_bytes: u64,
}

#[derive(Clone, Copy, Default)]
struct Origin {
    packet: u64,
    pes_packet: u64,
    pts: Option<u64>,
}

pub struct VideoFrameProbe {
    codec: u8,
    prefix: Vec<u8>,
    nal_bytes: u64,
    zeros: usize,
    origin: Origin,
    zero_origin: Origin,
    pes_origin: Origin,
    pes_header: Vec<u8>,
    header_pending: bool,
    active: bool,
    hevc_pps: BTreeMap<u32, u32>,
    frames: Vec<VideoFrame>,
    damaged: bool,
}

impl VideoFrameProbe {
    pub fn new(codec: u8) -> Self {
        Self {
            codec,
            prefix: Vec::with_capacity(128),
            nal_bytes: 0,
            zeros: 0,
            origin: Origin::default(),
            zero_origin: Origin::default(),
            pes_origin: Origin::default(),
            pes_header: Vec::new(),
            header_pending: false,
            active: false,
            hevc_pps: BTreeMap::new(),
            frames: Vec::new(),
            damaged: false,
        }
    }
    pub fn codec(&self) -> u8 {
        self.codec
    }
    pub fn push(&mut self, mut payload: &[u8], start: bool, packet: u64) {
        if start {
            self.pes_header.clear();
            self.header_pending = true;
            self.pes_origin = Origin {
                packet,
                pes_packet: packet,
                pts: None,
            };
        }
        while self.header_pending && !payload.is_empty() {
            self.pes_header.push(payload[0]);
            payload = &payload[1..];
            if self.pes_header.len() == 9 && !self.pes_header.starts_with(&[0, 0, 1]) {
                self.header_pending = false;
                return;
            }
            if self.pes_header.len() >= 9
                && self.pes_header.len() == 9 + usize::from(self.pes_header[8])
            {
                if self.pes_header[7] & 0x80 != 0 {
                    self.pes_origin.pts = self.pes_header.get(9..14).and_then(pts);
                }
                self.header_pending = false;
            }
        }
        for &byte in payload {
            let origin = Origin {
                packet,
                ..self.pes_origin
            };
            if byte == 0 {
                if self.zeros == 0 {
                    self.zero_origin = origin;
                }
                self.zeros += 1;
                continue;
            }
            if byte == 1 && self.zeros >= 2 {
                self.finish_nal();
                self.active = true;
                self.origin = self.zero_origin;
                self.zeros = 0;
                continue;
            }
            if self.active {
                self.nal_bytes += self.zeros as u64 + 1;
                for _ in 0..self.zeros.min(128_usize.saturating_sub(self.prefix.len())) {
                    self.prefix.push(0);
                }
                if self.prefix.len() < 128 {
                    self.prefix.push(byte);
                }
            }
            self.zeros = 0;
        }
    }
    /// Discard a damaged partial NAL after a transport continuity error.
    pub fn discontinuity(&mut self) {
        self.damaged = true;
        if let Some(frame) = self.frames.last_mut() {
            frame.damaged = true;
        }
        self.prefix.clear();
        self.nal_bytes = 0;
        self.zeros = 0;
        self.active = false;
        self.header_pending = false;
    }
    pub fn finish(mut self) -> Vec<VideoFrame> {
        self.finish_nal();
        self.frames
    }
    fn finish_nal(&mut self) {
        let nal = &self.prefix;
        let mut picture = None;
        let mut vcl = false;
        if self.codec == 0x1b && !nal.is_empty() {
            let kind = nal[0] & 31;
            vcl = matches!(kind, 1 | 2 | 5);
            if vcl {
                let data = rbsp(&nal[1..]);
                let mut bits = Bits::new(&data);
                if bits.ue() == Some(0) {
                    picture = Some(if kind == 5 {
                        PictureType::Idr
                    } else {
                        match bits.ue().map(|v| v % 5) {
                            Some(0 | 3) => PictureType::P,
                            Some(1) => PictureType::B,
                            Some(2 | 4) => PictureType::I,
                            _ => PictureType::Unknown,
                        }
                    });
                }
            }
        } else if self.codec == 0x24 && nal.len() >= 3 {
            let kind = (nal[0] >> 1) & 63;
            // Only base-layer pictures. A multilayer stream must not count layers as frames.
            let layer = ((nal[0] & 1) << 5) | (nal[1] >> 3);
            let data = rbsp(&nal[2..]);
            let mut bits = Bits::new(&data);
            if kind == 34
                && let (Some(id), Some(_sps), Some(_flags), Some(extra)) =
                    (bits.ue(), bits.ue(), bits.read(2), bits.read(3))
            {
                self.hevc_pps.insert(id, extra);
            }
            vcl = kind <= 31 && layer == 0;
            if vcl && bits.read(1) == Some(1) {
                if (16..=23).contains(&kind) {
                    bits.read(1);
                }
                let slice = bits
                    .ue()
                    .and_then(|pps| self.hevc_pps.get(&pps).copied())
                    .and_then(|extra| {
                        bits.read(extra as usize)?;
                        bits.ue()
                    });
                picture = Some(match kind {
                    19 | 20 => PictureType::Idr,
                    21 => PictureType::Cra,
                    16..=18 => PictureType::Bla,
                    _ => match slice {
                        Some(0) => PictureType::B,
                        Some(1) => PictureType::P,
                        Some(2) => PictureType::I,
                        _ => PictureType::Unknown,
                    },
                });
            }
        }
        if let Some(picture_type) = picture {
            self.frames.push(VideoFrame {
                packet: self.origin.packet,
                pes_packet: self.origin.pes_packet,
                pts: self.origin.pts,
                picture_type,
                damaged: self.damaged,
                vcl_bytes: 0,
            });
        }
        if picture.is_some() {
            self.damaged = false;
        }
        if vcl && let Some(frame) = self.frames.last_mut() {
            frame.vcl_bytes += self.nal_bytes;
        }
        self.prefix.clear();
        self.nal_bytes = 0;
    }
}
fn pts(p: &[u8]) -> Option<u64> {
    if p.len() != 5 || p[0] & 1 == 0 || p[2] & 1 == 0 || p[4] & 1 == 0 {
        return None;
    }
    Some(
        (u64::from((p[0] >> 1) & 7) << 30)
            | (u64::from(p[1]) << 22)
            | (u64::from(p[2] >> 1) << 15)
            | (u64::from(p[3]) << 7)
            | u64::from(p[4] >> 1),
    )
}
fn rbsp(nal: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    let mut zeros = 0;
    for &byte in nal {
        if zeros >= 2 && byte == 3 {
            zeros = 0;
            continue;
        }
        result.push(byte);
        zeros = if byte == 0 { zeros + 1 } else { 0 };
    }
    result
}
struct Bits<'a> {
    data: &'a [u8],
    offset: usize,
}
impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }
    fn read(&mut self, count: usize) -> Option<u32> {
        if count > 32 || self.offset + count > self.data.len() * 8 {
            return None;
        }
        let mut value = 0;
        for _ in 0..count {
            value =
                (value << 1) | u32::from((self.data[self.offset / 8] >> (7 - self.offset % 8)) & 1);
            self.offset += 1;
        }
        Some(value)
    }
    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.read(1)? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        Some(((1_u32 << zeros) - 1) + self.read(zeros)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_start_codes_and_multiple_slices_are_one_picture() {
        let bytes = [
            0, 0, 1, 0x65, 0xb8, 0x11, 0, 0, 1, 0x65, 0x4e, 0x22, 0, 0, 1, 0x41, 0xc0, 0x33,
        ];
        for chunk in 1..bytes.len() {
            let mut p = VideoFrameProbe::new(0x1b);
            for (i, data) in bytes.chunks(chunk).enumerate() {
                p.push(data, false, i as u64);
            }
            let f = p.finish();
            assert_eq!(f.len(), 2);
            assert_eq!(f[0].picture_type, PictureType::Idr);
            assert_eq!(f[0].vcl_bytes, 6);
            assert_eq!(f[1].picture_type, PictureType::P);
        }
    }
    #[test]
    fn hevc_first_slice_counts_one_picture_per_access_unit() {
        let mut p = VideoFrameProbe::new(0x24);
        // PPS id 0, SPS id 0, two flags 0, extra bits 0; IDR first and dependent slices.
        p.push(
            &[
                0, 0, 1, 68, 1, 0xc1, 0, 0, 1, 38, 1, 0xaf, 0, 0, 1, 38, 1, 0x20, 0, 0, 1, 2, 1,
                0xd0,
            ],
            false,
            0,
        );
        let f = p.finish();
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].picture_type, PictureType::Idr);
        assert_eq!(f[1].picture_type, PictureType::P);
    }
}
