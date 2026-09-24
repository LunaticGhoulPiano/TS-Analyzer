use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const PACKET_SIZE_BYTES: usize = 188;
const PACKET_COUNT: usize = 16;
const SYNC_BYTE: u8 = 0x47;
const NULL_PID_HIGH: u8 = 0x1f;
const NULL_PID_LOW: u8 = 0xff;
const PAYLOAD_ONLY: u8 = 0x10;
const PAYLOAD_FILL: u8 = 0xff;
const OUTPUT_FILE_NAME: &str = "transport_detect_packet_size_188.ts";

fn workspace_root() -> io::Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "tsan-core is not located under the workspace crates directory",
            )
        })
}

fn make_null_packet(continuity_counter: u8) -> [u8; PACKET_SIZE_BYTES] {
    let mut packet = [PAYLOAD_FILL; PACKET_SIZE_BYTES];
    packet[0] = SYNC_BYTE;
    packet[1] = NULL_PID_HIGH;
    packet[2] = NULL_PID_LOW;
    packet[3] = PAYLOAD_ONLY | (continuity_counter & 0x0f);
    packet
}

fn make_transport_stream() -> Vec<u8> {
    let mut stream = Vec::with_capacity(PACKET_SIZE_BYTES * PACKET_COUNT);

    for packet_index in 0..PACKET_COUNT {
        let packet = make_null_packet(packet_index as u8);
        stream.extend_from_slice(&packet);
    }

    stream
}

fn write_new_or_verify_existing(path: &Path, data: &[u8]) -> io::Result<bool> {
    if path.exists() {
        if fs::read(path)? == data {
            return Ok(false);
        }

        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "refusing to overwrite an existing file with different content: {}",
                path.display()
            ),
        ));
    }

    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(data)?;
    file.flush()?;

    Ok(true)
}

fn main() -> io::Result<()> {
    let output_directory = workspace_root()?
        .join("developmentHelpers")
        .join("test-data")
        .join("inputs")
        .join("synthetic");
    let output_path = output_directory.join(OUTPUT_FILE_NAME);
    let stream = make_transport_stream();

    fs::create_dir_all(&output_directory)?;

    if write_new_or_verify_existing(&output_path, &stream)? {
        println!(
            "generated {} bytes at {}",
            stream.len(),
            output_path.display()
        );
    } else {
        println!(
            "verified existing {}-byte file at {}",
            stream.len(),
            output_path.display()
        );
    }

    Ok(())
}
