use tsan_core::{PictureType, VideoFrame};

#[derive(Clone, Debug)]
pub struct Gop {
    pub first_packet: u64,
    pub start_pts: Option<u64>,
    pub pictures: Vec<PictureType>,
    pub vcl_bytes: u64,
    /// Start of the following GOP, used as the completed program-byte measurement position.
    pub next_packet: Option<u64>,
    /// Program TS bytes, including both boundary packets.
    /// This is not an additive byte total: adjacent GOPs share a boundary packet.
    pub program_ts_bytes: Option<u64>,
    /// False for a recording that starts between intra pictures, and for the last GOP at EOF.
    pub complete: bool,
    pub damaged: bool,
}
impl Gop {
    pub fn structure(&self) -> String {
        self.pictures
            .iter()
            .map(|p| p.label())
            .collect::<Vec<_>>()
            .join(" ")
    }
}
#[derive(Clone, Debug)]
pub struct VideoGops {
    pub stream_type: u8,
    pub program_number: Option<u16>,
    pub program_pids: Vec<u16>,
    pub frame_count: usize,
    pub gops: Vec<Gop>,
}
impl VideoGops {
    pub(crate) fn from_frames(stream_type: u8, frames: Vec<VideoFrame>) -> Self {
        let frame_count = frames.len();
        let mut gops: Vec<Gop> = Vec::new();
        for frame in frames {
            if frame.picture_type.intra() || gops.is_empty() {
                if let Some(previous) = gops.last_mut() {
                    previous.next_packet = Some(frame.packet);
                    previous.complete = !previous.damaged
                        && previous.pictures.first().is_some_and(|p| p.intra())
                        && !previous.pictures.contains(&PictureType::Unknown);
                }
                gops.push(Gop {
                    first_packet: frame.packet,
                    start_pts: frame.pts,
                    pictures: Vec::new(),
                    vcl_bytes: 0,
                    next_packet: None,
                    program_ts_bytes: None,
                    complete: false,
                    damaged: false,
                });
            }
            if let Some(gop) = gops.last_mut() {
                gop.damaged |= frame.damaged;
                gop.pictures.push(frame.picture_type);
                gop.vcl_bytes += frame.vcl_bytes;
            }
        }
        Self {
            stream_type,
            program_number: None,
            program_pids: Vec::new(),
            frame_count,
            gops,
        }
    }

    pub(crate) fn measure_program_bytes(
        &mut self,
        number: u16,
        pids: Vec<u16>,
        packet_pids: &[u16],
        packet_size: usize,
    ) {
        self.program_number = Some(number);
        self.program_pids = pids;
        let mut included = [false; 8192];
        for &pid in &self.program_pids {
            if let Some(value) = included.get_mut(usize::from(pid)) {
                *value = true;
            }
        }
        for gop in &mut self.gops {
            let Some(end) = gop.next_packet else { continue };
            let (Ok(start), Ok(end)) = (usize::try_from(gop.first_packet), usize::try_from(end))
            else {
                continue;
            };
            if let Some(packets) = packet_pids.get(start..=end) {
                let count = packets
                    .iter()
                    .filter(|&&pid| included.get(usize::from(pid)).copied().unwrap_or(false))
                    .count();
                gop.program_ts_bytes = Some(count as u64 * packet_size as u64);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(picture_type: PictureType, damaged: bool) -> VideoFrame {
        VideoFrame {
            packet: 0,
            pes_packet: 0,
            pts: None,
            picture_type,
            damaged,
            vcl_bytes: 100,
        }
    }
    #[test]
    fn boundary_unknown_and_damaged_groups_are_not_complete() {
        use PictureType::*;
        let video = VideoGops::from_frames(
            0x24,
            vec![
                frame(P, false),
                frame(Idr, false),
                frame(P, false),
                frame(Idr, false),
                frame(Unknown, false),
                frame(Idr, false),
                frame(P, true),
                frame(Idr, false),
            ],
        );
        assert_eq!(
            video.gops.iter().map(|g| g.complete).collect::<Vec<_>>(),
            [false, true, false, false, false]
        );
        assert_eq!(video.gops[1].structure(), "IDR P");
        assert_eq!(video.gops[1].vcl_bytes, 200);
    }

    #[test]
    fn program_bytes_include_both_boundaries() {
        let frames = [0, 4, 7].map(|packet| VideoFrame {
            packet,
            ..frame(PictureType::Idr, false)
        });
        let mut video = VideoGops::from_frames(0x1b, frames.to_vec());
        let pids = [102, 8191, 103, 16, 102, 100, u16::MAX, 102];
        for packet_size in [188, 192, 204] {
            video.measure_program_bytes(1, vec![0, 100, 101, 102, 103], &pids, packet_size);
            assert_eq!(video.gops[0].next_packet, Some(4));
            assert_eq!(video.gops[0].program_ts_bytes, Some(3 * packet_size as u64));
            assert_eq!(video.gops[1].program_ts_bytes, Some(3 * packet_size as u64));
            assert_eq!(video.gops[2].program_ts_bytes, None);
            assert_eq!(video.gops[0].vcl_bytes, 100);
        }
    }
}
