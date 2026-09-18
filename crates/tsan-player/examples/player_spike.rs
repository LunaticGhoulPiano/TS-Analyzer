use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as gst_app;

const TS_PACKET_SIZE: usize = 188;
const FEED_PACKETS_PER_BUFFER: usize = 256;
const APP_SOURCE_MAX_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
enum VideoBackend {
    D3d12,
    D3d11,
}

impl VideoBackend {
    fn parse(value: &str) -> Result<Self, io::Error> {
        match value {
            "d3d12" => Ok(Self::D3d12),
            "d3d11" => Ok(Self::D3d11),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unsupported backend '{value}'; expected d3d12 or d3d11"),
            )),
        }
    }

    fn decoder_factory(self, adapter_index: u32) -> String {
        let prefix = match self {
            Self::D3d12 => "d3d12h265",
            Self::D3d11 => "d3d11h265",
        };
        if adapter_index == 0 {
            format!("{prefix}dec")
        } else {
            format!("{prefix}device{adapter_index}dec")
        }
    }

    fn sink_factory(self) -> &'static str {
        match self {
            Self::D3d12 => "d3d12videosink",
            Self::D3d11 => "d3d11videosink",
        }
    }

    fn memory_caps(self) -> &'static str {
        match self {
            Self::D3d12 => "video/x-raw(memory:D3D12Memory)",
            Self::D3d11 => "video/x-raw(memory:D3D11Memory)",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::D3d12 => "d3d12",
            Self::D3d11 => "d3d11",
        }
    }
}

#[derive(Debug)]
struct Arguments {
    input: PathBuf,
    backend: VideoBackend,
    adapter_index: u32,
    program_number: Option<i32>,
}

#[derive(Debug)]
struct FeedStats {
    buffers: u64,
    bytes: u64,
}

#[derive(Debug)]
struct BusStats {
    warnings: u64,
    qos_messages: u64,
}

fn usage() -> &'static str {
    "Usage: cargo run --release --locked -p tsan-player --example player_spike -- \
<INPUT.ts> [--backend d3d12|d3d11] [--adapter INDEX] [--program NUMBER]"
}

fn next_option_value(
    arguments: &mut impl Iterator<Item = OsString>,
    option: &str,
) -> Result<OsString, io::Error> {
    arguments.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("missing value for {option}"),
        )
    })
}

fn parse_arguments() -> Result<Option<Arguments>, io::Error> {
    let mut arguments = env::args_os().skip(1);
    let mut input = None;
    let mut backend = VideoBackend::D3d12;
    let mut adapter_index = 0;
    let mut program_number = None;

    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--help" | "-h") => {
                println!("{}", usage());
                return Ok(None);
            }
            Some("--backend") => {
                let value = next_option_value(&mut arguments, "--backend")?;
                let value = value.to_str().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "--backend must be valid UTF-8")
                })?;
                backend = VideoBackend::parse(value)?;
            }
            Some("--adapter") => {
                let value = next_option_value(&mut arguments, "--adapter")?;
                let value = value.to_str().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "--adapter must be valid UTF-8")
                })?;
                adapter_index = value.parse::<u32>().map_err(|error| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("invalid adapter index '{value}': {error}"),
                    )
                })?;
                i32::try_from(adapter_index).map_err(|error| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("adapter index '{value}' is too large: {error}"),
                    )
                })?;
            }
            Some("--program") => {
                let value = next_option_value(&mut arguments, "--program")?;
                let value = value.to_str().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "--program must be valid UTF-8")
                })?;
                let parsed = value.parse::<i32>().map_err(|error| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("invalid program number '{value}': {error}"),
                    )
                })?;
                if parsed < 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "--program must be zero or greater",
                    ));
                }
                program_number = Some(parsed);
            }
            Some(value) if value.starts_with('-') => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown option '{value}'"),
                ));
            }
            _ => {
                if input.is_some() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "only one input path may be specified",
                    ));
                }
                input = Some(PathBuf::from(argument));
            }
        }
    }

    let input = input.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("missing input path\n{}", usage()),
        )
    })?;

    Ok(Some(Arguments {
        input,
        backend,
        adapter_index,
        program_number,
    }))
}

fn create_element(factory_name: &str, element_name: &str) -> Result<gst::Element, io::Error> {
    gst::ElementFactory::make(factory_name)
        .name(element_name)
        .build()
        .map_err(|error| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("failed to create GStreamer element '{factory_name}': {error}"),
            )
        })
}

fn create_video_sink(backend: VideoBackend, adapter_index: u32) -> Result<gst::Element, io::Error> {
    let adapter_index = i32::try_from(adapter_index).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("adapter index is too large: {error}"),
        )
    })?;
    let builder = gst::ElementFactory::make(backend.sink_factory())
        .name("video-sink")
        .property("force-aspect-ratio", true)
        .property("adapter", adapter_index);

    let result = match backend {
        VideoBackend::D3d12 => builder.property("fullscreen-on-alt-enter", true).build(),
        VideoBackend::D3d11 => builder
            .property_from_str("fullscreen-toggle-mode", "alt-enter")
            .build(),
    };

    result.map_err(|error| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "failed to create GStreamer element '{}': {error}",
                backend.sink_factory()
            ),
        )
    })
}

fn is_h265_pad(pad: &gst::Pad) -> bool {
    let caps = pad.current_caps().unwrap_or_else(|| pad.query_caps(None));

    println!("demux-pad: name={} caps={caps}", pad.name());

    caps.structure(0)
        .is_some_and(|structure| structure.name() == "video/x-h265")
}

fn connect_video_pad(
    demux: &gst::Element,
    video_queue: &gst::Element,
) -> Result<Arc<AtomicBool>, io::Error> {
    let queue_sink_pad = video_queue.static_pad("sink").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "video queue does not expose a static sink pad",
        )
    })?;
    let video_pad_linked = Arc::new(AtomicBool::new(false));
    let linked_for_callback = Arc::clone(&video_pad_linked);

    demux.connect_pad_added(move |_demux, source_pad| {
        if !is_h265_pad(source_pad) {
            return;
        }

        if linked_for_callback.load(Ordering::Acquire) {
            println!("ignoring additional H.265 pad: {}", source_pad.name());
            return;
        }

        match source_pad.link(&queue_sink_pad) {
            Ok(_) => {
                linked_for_callback.store(true, Ordering::Release);
                println!("linked H.265 pad: {}", source_pad.name());
            }
            Err(error) => {
                eprintln!(
                    "failed to link H.265 pad '{}' to video queue: {error}",
                    source_pad.name()
                );
            }
        }
    });

    Ok(video_pad_linked)
}

fn feed_file(app_src: gst_app::AppSrc, input: PathBuf) -> Result<FeedStats, io::Error> {
    let mut file = File::open(&input)?;
    let mut stats = FeedStats {
        buffers: 0,
        bytes: 0,
    };

    loop {
        let mut bytes = vec![0_u8; TS_PACKET_SIZE * FEED_PACKETS_PER_BUFFER];
        let bytes_read = match file.read(&mut bytes) {
            Ok(bytes_read) => bytes_read,
            Err(error) => {
                let _ = app_src.end_of_stream();
                return Err(error);
            }
        };

        if bytes_read == 0 {
            break;
        }

        bytes.truncate(bytes_read);
        let buffer = gst::Buffer::from_mut_slice(bytes);
        app_src.push_buffer(buffer).map_err(|error| {
            io::Error::other(format!("appsrc failed to push a TS buffer: {error}"))
        })?;

        stats.buffers += 1;
        stats.bytes += bytes_read as u64;
    }

    app_src
        .end_of_stream()
        .map_err(|error| io::Error::other(format!("appsrc failed to send EOS: {error}")))?;

    Ok(stats)
}

fn message_source(message: &gst::MessageRef) -> String {
    message
        .src()
        .map(|source| source.path_string().to_string())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn wait_for_pipeline(bus: &gst::Bus) -> Result<BusStats, io::Error> {
    let mut stats = BusStats {
        warnings: 0,
        qos_messages: 0,
    };

    for message in bus.iter_timed(gst::ClockTime::NONE) {
        match message.view() {
            gst::MessageView::Eos(_) => return Ok(stats),
            gst::MessageView::Error(error) => {
                let debug = error
                    .debug()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "none".to_owned());
                return Err(io::Error::other(format!(
                    "pipeline error from {}: {}; debug={debug}",
                    message_source(message.as_ref()),
                    error.error()
                )));
            }
            gst::MessageView::Warning(warning) => {
                stats.warnings += 1;
                let debug = warning
                    .debug()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "none".to_owned());
                eprintln!(
                    "pipeline warning from {}: {}; debug={debug}",
                    message_source(message.as_ref()),
                    warning.error()
                );
            }
            gst::MessageView::Qos(_) => stats.qos_messages += 1,
            _ => {}
        }
    }

    Err(io::Error::other("pipeline bus closed before EOS"))
}

fn print_decoder_identity(decoder: &gst::Element) {
    println!("decoder-element: {}", decoder.name());

    if decoder.find_property("vendor-id").is_some() {
        println!(
            "decoder-vendor-id: {}",
            decoder.property::<u32>("vendor-id")
        );
    }
    if decoder.find_property("device-id").is_some() {
        println!(
            "decoder-device-id: {}",
            decoder.property::<u32>("device-id")
        );
    }
    if decoder.find_property("adapter-luid").is_some() {
        println!(
            "decoder-adapter-luid: {}",
            decoder.property::<i64>("adapter-luid")
        );
    }
}

fn run(arguments: Arguments) -> Result<(), Box<dyn Error>> {
    if !arguments.input.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("input file does not exist: {}", arguments.input.display()),
        )
        .into());
    }

    gst::init()?;

    let app_src_caps = gst::Caps::builder("video/mpegts")
        .field("systemstream", true)
        .field("packetsize", TS_PACKET_SIZE as i32)
        .build();
    let app_src_element = gst::ElementFactory::make("appsrc")
        .name("source")
        .property("caps", app_src_caps)
        .property("format", gst::Format::Bytes)
        .property("stream-type", gst_app::AppStreamType::Stream)
        .property("block", true)
        .property("max-bytes", APP_SOURCE_MAX_BYTES)
        .build()?;
    let app_src = app_src_element
        .clone()
        .downcast::<gst_app::AppSrc>()
        .map_err(|_| io::Error::other("appsrc element has an unexpected runtime type"))?;

    let ts_parse = create_element("tsparse", "ts-parse")?;
    let demux_builder = gst::ElementFactory::make("tsdemux").name("demux");
    let demux = match arguments.program_number {
        Some(program_number) => demux_builder
            .property("program-number", program_number)
            .build()?,
        None => demux_builder.build()?,
    };
    let video_queue = create_element("queue", "video-queue")?;
    let h265_parse = create_element("h265parse", "h265-parse")?;
    let decoder_factory = arguments.backend.decoder_factory(arguments.adapter_index);
    let decoder = create_element(&decoder_factory, "video-decoder")?;
    let memory_caps = gst::Caps::from_str(arguments.backend.memory_caps())?;
    let memory_filter = gst::ElementFactory::make("capsfilter")
        .name("gpu-memory-filter")
        .property("caps", memory_caps)
        .build()?;
    let video_sink = create_video_sink(arguments.backend, arguments.adapter_index)?;

    let pipeline = gst::Pipeline::builder().name("player-spike").build();
    pipeline.add_many([
        &app_src_element,
        &ts_parse,
        &demux,
        &video_queue,
        &h265_parse,
        &decoder,
        &memory_filter,
        &video_sink,
    ])?;

    gst::Element::link_many([&app_src_element, &ts_parse, &demux])?;
    gst::Element::link_many([
        &video_queue,
        &h265_parse,
        &decoder,
        &memory_filter,
        &video_sink,
    ])?;
    let video_pad_linked = connect_video_pad(&demux, &video_queue)?;

    let bus = pipeline
        .bus()
        .ok_or_else(|| io::Error::other("pipeline does not expose a bus"))?;

    println!("input: {}", arguments.input.display());
    println!("backend: {}", arguments.backend.name());
    println!("adapter-index: {}", arguments.adapter_index);
    println!("decoder-factory: {decoder_factory}");
    println!("sink-factory: {}", arguments.backend.sink_factory());
    println!("required-memory-caps: {}", arguments.backend.memory_caps());
    println!(
        "program-number: {}",
        arguments
            .program_number
            .map(|value| value.to_string())
            .unwrap_or_else(|| "automatic".to_owned())
    );
    println!("fullscreen-toggle: Alt+Enter");

    pipeline.set_state(gst::State::Playing)?;

    let feeder_input = arguments.input.clone();
    let feeder = thread::Builder::new()
        .name("player-spike-feeder".to_owned())
        .spawn(move || feed_file(app_src, feeder_input))?;

    let bus_result = wait_for_pipeline(&bus);

    if let Some(caps) = video_sink
        .static_pad("sink")
        .and_then(|pad| pad.current_caps())
    {
        println!("negotiated-video-caps: {caps}");
    }
    print_decoder_identity(&decoder);

    pipeline.set_state(gst::State::Null)?;

    let feeder_result = feeder
        .join()
        .map_err(|_| io::Error::other("player feeder thread panicked"))?;
    let bus_stats = bus_result?;
    let feed_stats = feeder_result?;

    if !video_pad_linked.load(Ordering::Acquire) {
        return Err(io::Error::other("no H.265 video pad was linked").into());
    }

    println!(
        "feed: buffers={} bytes={}",
        feed_stats.buffers, feed_stats.bytes
    );
    println!(
        "bus: warnings={} qos-messages={}",
        bus_stats.warnings, bus_stats.qos_messages
    );

    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let Some(arguments) = parse_arguments()? else {
        return Ok(());
    };

    run(arguments)
}
