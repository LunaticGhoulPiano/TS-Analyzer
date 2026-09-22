//! Bounded, dependency-free probes for H.264/H.265 elementary streams.

const MAX_SAMPLE_BYTES: usize = 256 * 1024;
const SAMPLE_TAIL_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VideoMetadata {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frame_rate: Option<(u32, u32)>,
}

pub(crate) struct VideoProbe {
    stream_type: u8,
    sample: Vec<u8>,
    random_access_tail: Vec<u8>,
    dimensions: Option<(u32, u32)>,
}

impl VideoProbe {
    pub(crate) fn new(stream_type: u8) -> Self {
        Self {
            stream_type,
            sample: Vec::new(),
            random_access_tail: Vec::new(),
            dimensions: None,
        }
    }

    pub(crate) fn stream_type(&self) -> u8 {
        self.stream_type
    }

    pub(crate) fn push_packet(&mut self, payload: &[u8], unit_start: bool) -> bool {
        let mut elementary = payload;
        if unit_start && payload.len() >= 9 && payload.starts_with(&[0, 0, 1]) {
            let header_end = 9 + usize::from(payload[8]);
            let Some(data) = payload.get(header_end..) else {
                return false;
            };
            elementary = data;
        }

        let mut scan = Vec::with_capacity(self.random_access_tail.len() + elementary.len());
        scan.extend_from_slice(&self.random_access_tail);
        scan.extend_from_slice(elementary);
        let random_access = contains_random_access_nal(self.stream_type, &scan);
        let tail_start = scan.len().saturating_sub(4);
        self.random_access_tail.clear();
        self.random_access_tail
            .extend_from_slice(&scan[tail_start..]);

        if self.dimensions.is_none() {
            self.sample.extend_from_slice(elementary);
            if self.sample.len() >= MAX_SAMPLE_BYTES {
                self.dimensions = dimensions_in_annex_b(self.stream_type, &self.sample);
                if self.dimensions.is_none() {
                    let discard = self.sample.len() - SAMPLE_TAIL_BYTES;
                    self.sample.drain(..discard);
                }
            }
        }
        random_access
    }

    pub(crate) fn finish(mut self) -> Option<VideoMetadata> {
        if self.dimensions.is_none() {
            self.dimensions = dimensions_in_annex_b(self.stream_type, &self.sample);
        }
        #[cfg(feature = "native-tsduck")]
        let native = native_sps(self.stream_type, &self.sample);
        #[cfg(feature = "native-tsduck")]
        let frame_rate = native
            .as_ref()
            .and_then(|sps| reduced_rate(sps.frame_rate_numerator, sps.frame_rate_denominator));
        #[cfg(not(feature = "native-tsduck"))]
        let frame_rate = None;
        #[cfg(feature = "native-tsduck")]
        if self.dimensions.is_none() {
            self.dimensions = native.map(|sps| (sps.width, sps.height));
        }
        if self.dimensions.is_none() && frame_rate.is_none() {
            return None;
        }
        Some(VideoMetadata {
            width: self.dimensions.map(|(width, _)| width),
            height: self.dimensions.map(|(_, height)| height),
            frame_rate,
        })
    }
}

#[cfg(feature = "native-tsduck")]
fn reduced_rate(numerator: u32, denominator: u32) -> Option<(u32, u32)> {
    if numerator == 0 || denominator == 0 {
        return None;
    }
    let (mut left, mut right) = (numerator, denominator);
    while right != 0 {
        (left, right) = (right, left % right);
    }
    Some((numerator / left, denominator / left))
}

#[cfg(feature = "native-tsduck")]
fn native_sps(stream_type: u8, data: &[u8]) -> Option<tsan_tsduck_sys::VideoSps> {
    let mut offset = 0;
    while let Some((_, nal_start)) = next_start_code(data, offset) {
        let next = next_start_code(data, nal_start);
        let nal_end = next.map_or(data.len(), |(start, _)| start);
        let nal = &data[nal_start..nal_end];
        let expected = match stream_type {
            0x1b => nal.first().is_some_and(|byte| byte & 0x1f == 7),
            0x24 => nal.first().is_some_and(|byte| (byte >> 1) & 0x3f == 33),
            _ => false,
        };
        if expected && let Some(metadata) = tsan_tsduck_sys::parse_sps(stream_type, nal) {
            return Some(metadata);
        }
        offset = next.map_or(data.len(), |(_, start)| start);
    }
    None
}

fn contains_random_access_nal(stream_type: u8, data: &[u8]) -> bool {
    let mut offset = 0;
    while let Some((_, nal_start)) = next_start_code(data, offset) {
        let Some(&header) = data.get(nal_start) else {
            return false;
        };
        let random_access = match stream_type {
            0x1b => header & 0x1f == 5,
            0x24 => (16..=23).contains(&((header >> 1) & 0x3f)),
            _ => false,
        };
        if random_access {
            return true;
        }
        offset = nal_start.saturating_add(1);
    }
    false
}

fn dimensions_in_annex_b(stream_type: u8, data: &[u8]) -> Option<(u32, u32)> {
    let mut offset = 0;
    while let Some((_, nal_start)) = next_start_code(data, offset) {
        let next = next_start_code(data, nal_start);
        let nal_end = next.map_or(data.len(), |(start, _)| start);
        let nal = &data[nal_start..nal_end];
        let dimensions = match stream_type {
            0x1b if nal.first().is_some_and(|byte| byte & 0x1f == 7) => h264_sps(nal),
            0x24 if nal.first().is_some_and(|byte| (byte >> 1) & 0x3f == 33) => h265_sps(nal),
            _ => None,
        };
        if dimensions.is_some() {
            return dimensions;
        }
        offset = next.map_or(data.len(), |(_, start)| start);
    }
    None
}

fn next_start_code(data: &[u8], from: usize) -> Option<(usize, usize)> {
    let last = data.len().checked_sub(3)?;
    (from..=last)
        .find(|&index| data[index..index + 3] == [0, 0, 1])
        .map(|index| (index, index + 3))
}

struct Bits<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Bits<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn read(&mut self, count: usize) -> Option<u32> {
        if count > 32 || self.position.checked_add(count)? > self.bytes.len() * 8 {
            return None;
        }
        let mut value = 0_u32;
        for _ in 0..count {
            let byte = self.bytes[self.position / 8];
            value = (value << 1) | u32::from((byte >> (7 - self.position % 8)) & 1);
            self.position += 1;
        }
        Some(value)
    }

    fn skip(&mut self, count: usize) -> Option<()> {
        self.position = self.position.checked_add(count)?;
        (self.position <= self.bytes.len() * 8).then_some(())
    }

    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.read(1)? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        ((1_u32 << zeros) - 1).checked_add(self.read(zeros)?)
    }

    fn se(&mut self) -> Option<i64> {
        let code = i64::from(self.ue()?);
        Some(if code & 1 == 0 {
            -(code / 2)
        } else {
            (code + 1) / 2
        })
    }
}

fn rbsp(ebsp: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(ebsp.len());
    let mut zeros = 0;
    for &byte in ebsp {
        if zeros >= 2 && byte == 3 {
            zeros = 0;
            continue;
        }
        bytes.push(byte);
        zeros = if byte == 0 { zeros + 1 } else { 0 };
    }
    bytes
}

fn dimensions(width: u32, height: u32, crop_x: u32, crop_y: u32) -> Option<(u32, u32)> {
    let width = width.checked_sub(crop_x)?;
    let height = height.checked_sub(crop_y)?;
    (width > 0 && height > 0 && width <= 65_535 && height <= 65_535).then_some((width, height))
}

fn skip_hevc_profile_tier_level(bits: &mut Bits<'_>, max_sub_layers: usize) -> Option<()> {
    bits.skip(96)?;
    let mut profile = [false; 7];
    let mut level = [false; 7];
    for index in 0..max_sub_layers {
        profile[index] = bits.read(1)? != 0;
        level[index] = bits.read(1)? != 0;
    }
    if max_sub_layers > 0 {
        bits.skip((8 - max_sub_layers) * 2)?;
    }
    for index in 0..max_sub_layers {
        if profile[index] {
            bits.skip(88)?;
        }
        if level[index] {
            bits.skip(8)?;
        }
    }
    Some(())
}

fn h265_sps(nal: &[u8]) -> Option<(u32, u32)> {
    let rbsp = rbsp(nal.get(2..)?);
    let mut bits = Bits::new(&rbsp);
    bits.skip(4)?;
    let max_sub_layers = bits.read(3)? as usize;
    bits.skip(1)?;
    skip_hevc_profile_tier_level(&mut bits, max_sub_layers)?;
    bits.ue()?;
    let chroma = bits.ue()?;
    if chroma > 3 {
        return None;
    }
    let separate_colour_plane = chroma == 3 && bits.read(1)? != 0;
    let width = bits.ue()?;
    let height = bits.ue()?;
    let (sub_width, sub_height) = match (chroma, separate_colour_plane) {
        (_, true) | (0, _) | (3, _) => (1, 1),
        (1, _) => (2, 2),
        (2, _) => (2, 1),
        _ => return None,
    };
    let (crop_x, crop_y) = if bits.read(1)? != 0 {
        let left = bits.ue()?;
        let right = bits.ue()?;
        let top = bits.ue()?;
        let bottom = bits.ue()?;
        (
            left.checked_add(right)?.checked_mul(sub_width)?,
            top.checked_add(bottom)?.checked_mul(sub_height)?,
        )
    } else {
        (0, 0)
    };
    dimensions(width, height, crop_x, crop_y)
}

fn skip_h264_scaling_list(bits: &mut Bits<'_>, size: usize) -> Option<()> {
    let mut last = 8_i64;
    let mut next = 8_i64;
    for _ in 0..size {
        if next != 0 {
            next = (last + bits.se()?).rem_euclid(256);
        }
        if next != 0 {
            last = next;
        }
    }
    Some(())
}

fn h264_sps(nal: &[u8]) -> Option<(u32, u32)> {
    let rbsp = rbsp(nal.get(1..)?);
    let mut bits = Bits::new(&rbsp);
    let profile = bits.read(8)?;
    bits.skip(16)?;
    bits.ue()?;
    let mut chroma = 1;
    let mut separate_colour_plane = false;
    if matches!(
        profile,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
    ) {
        chroma = bits.ue()?;
        if chroma > 3 {
            return None;
        }
        if chroma == 3 {
            separate_colour_plane = bits.read(1)? != 0;
        }
        bits.ue()?;
        bits.ue()?;
        bits.skip(1)?;
        if bits.read(1)? != 0 {
            for index in 0..if chroma == 3 { 12 } else { 8 } {
                if bits.read(1)? != 0 {
                    skip_h264_scaling_list(&mut bits, if index < 6 { 16 } else { 64 })?;
                }
            }
        }
    }
    bits.ue()?;
    match bits.ue()? {
        0 => {
            bits.ue()?;
        }
        1 => {
            bits.skip(1)?;
            bits.se()?;
            bits.se()?;
            let count = bits.ue()?;
            if count > 256 {
                return None;
            }
            for _ in 0..count {
                bits.se()?;
            }
        }
        2 => {}
        _ => return None,
    }
    bits.ue()?;
    bits.skip(1)?;
    let width_mbs = bits.ue()?.checked_add(1)?;
    let height_map_units = bits.ue()?.checked_add(1)?;
    let frame_mbs_only = bits.read(1)? != 0;
    if !frame_mbs_only {
        bits.skip(1)?;
    }
    bits.skip(1)?;
    let (crop_left, crop_right, crop_top, crop_bottom) = if bits.read(1)? != 0 {
        (bits.ue()?, bits.ue()?, bits.ue()?, bits.ue()?)
    } else {
        (0, 0, 0, 0)
    };
    let frame_factor = if frame_mbs_only { 1 } else { 2 };
    let chroma_array = if separate_colour_plane { 0 } else { chroma };
    let (crop_unit_x, crop_unit_y) = match chroma_array {
        0 => (1, frame_factor),
        1 => (2, 2 * frame_factor),
        2 => (2, frame_factor),
        3 => (1, frame_factor),
        _ => return None,
    };
    dimensions(
        width_mbs.checked_mul(16)?,
        height_map_units
            .checked_mul(16)?
            .checked_mul(frame_factor)?,
        crop_left
            .checked_add(crop_right)?
            .checked_mul(crop_unit_x)?,
        crop_top
            .checked_add(crop_bottom)?
            .checked_mul(crop_unit_y)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h264_idr_and_h265_irap_are_random_access_points() {
        assert!(contains_random_access_nal(0x1b, &[0, 0, 1, 0x65, 0x88]));
        assert!(!contains_random_access_nal(0x1b, &[0, 0, 1, 0x41, 0x88]));
        assert!(contains_random_access_nal(
            0x24,
            &[0, 0, 1, 19 << 1, 0x01, 0x88]
        ));
        assert!(contains_random_access_nal(
            0x24,
            &[0, 0, 1, 21 << 1, 0x01, 0x88]
        ));
        assert!(!contains_random_access_nal(
            0x24,
            &[0, 0, 1, 1 << 1, 0x01, 0x88]
        ));
    }

    #[test]
    fn video_probe_detects_start_code_split_across_packets() {
        let mut probe = VideoProbe::new(0x1b);
        assert!(!probe.push_packet(&[0x11, 0x00, 0x00], false));
        assert!(probe.push_packet(&[0x01, 0x65, 0x88], false));
    }
}
