use std::env;
use std::error::Error;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as gst_app;
use tsan_core::{TransportStreamTimeline, index_transport_stream, probe_transport_stream};

const FRAME_TIMEOUT: Duration = Duration::from_secs(10);
const TARGETS: [u64; 6] = [35, 70, 20, 100, 45, 15];

fn main() -> Result<(), Box<dyn Error>> {
    let input = env::args_os().nth(1).map(PathBuf::from).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "usage: seek_probe <INPUT.ts>")
    })?;
    if !input.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("input file does not exist: {}", input.display()),
        )
        .into());
    }

    let mut sample = Vec::new();
    File::open(&input)?
        .take(64 * 1024)
        .read_to_end(&mut sample)?;
    let probe = probe_transport_stream(&sample)?;
    let timeline = index_transport_stream(File::open(&input)?, probe)?
        .ok_or_else(|| io::Error::other("input has no stable indexed video timeline"))?;
    println!(
        "index: duration={:.3}s seek-points={}",
        timeline.duration().as_secs_f64(),
        timeline.seek_point_count(),
    );

    gst::init()?;
    run_mode(&input, "indexed-byte", &timeline)?;
    run_d3d_sink_mode(&input, &timeline)?;
    Ok(())
}

fn run_mode(
    input: &Path,
    mode: &str,
    timeline: &TransportStreamTimeline,
) -> Result<(), Box<dyn Error>> {
    let video_sink = gst::ElementFactory::make("appsink")
        .name(format!("video-sink-{mode}"))
        .property("sync", true)
        .property("max-buffers", 1_u32)
        .property("drop", true)
        .build()?;
    let app_sink = video_sink
        .clone()
        .downcast::<gst_app::AppSink>()
        .map_err(|_| io::Error::other("appsink has an unexpected runtime type"))?;
    let input_file = File::open(input)?;
    let input_size = i64::try_from(input_file.metadata()?.len())?;
    let source_caps = gst::Caps::builder("video/mpegts")
        .field("systemstream", true)
        .field("packetsize", 188_i32)
        .build();
    let source_element = gst::ElementFactory::make("appsrc")
        .name("source")
        .property("caps", source_caps)
        .property("format", gst::Format::Bytes)
        .property("stream-type", gst_app::AppStreamType::RandomAccess)
        .property("size", input_size)
        .property("block", true)
        .property("max-bytes", 32_u64 * 1024 * 1024)
        .build()?;
    let source = source_element
        .clone()
        .downcast::<gst_app::AppSrc>()
        .map_err(|_| io::Error::other("appsrc has an unexpected runtime type"))?;
    configure_source(&source, input_file);
    let source_queue = make_element("queue", "source-queue")?;
    let ts_parse = gst::ElementFactory::make("tsparse")
        .name("ts-parse")
        .property("split-on-rai", true)
        .property("ignore-pcr", true)
        .property("skew-corrections", false)
        .build()?;
    let demux = gst::ElementFactory::make("tsdemux")
        .name("demux")
        .property("latency", 0_i32)
        .property("skew-corrections", false)
        .build()?;
    let video_queue = make_element("queue", "video-queue")?;
    let parser = make_element("h265parse", "video-parser")?;
    let decoder = make_element("d3d12h265dec", "video-decoder")?;
    let pipeline = gst::Pipeline::builder()
        .name(format!("seek-probe-{mode}"))
        .build();
    pipeline.add_many([
        &source_element,
        &source_queue,
        &ts_parse,
        &demux,
        &video_queue,
        &parser,
        &decoder,
        &video_sink,
    ])?;
    gst::Element::link_many([&source_element, &source_queue, &ts_parse, &demux])?;
    gst::Element::link_many([&video_queue, &parser, &decoder, &video_sink])?;

    let queue_sink = video_queue
        .static_pad("sink")
        .ok_or_else(|| io::Error::other("video queue has no sink pad"))?;
    let linked = Arc::new(AtomicBool::new(false));
    let linked_for_callback = Arc::clone(&linked);
    demux.connect_pad_added(move |_demux, source_pad| {
        let caps = source_pad
            .current_caps()
            .unwrap_or_else(|| source_pad.query_caps(None));
        let is_h265 = caps
            .structure(0)
            .is_some_and(|structure| structure.name() == "video/x-h265");
        if is_h265
            && linked_for_callback
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            && source_pad.link(&queue_sink).is_err()
        {
            linked_for_callback.store(false, Ordering::Release);
        }
    });

    pipeline.set_state(gst::State::Paused)?;
    let (state_result, current, pending) =
        pipeline.state(gst::ClockTime::from_seconds(FRAME_TIMEOUT.as_secs()));
    state_result.map_err(|error| {
        io::Error::other(format!(
            "{mode}: failed to preroll: {error} (current={current:?}, pending={pending:?})"
        ))
    })?;
    pipeline.set_state(gst::State::Playing)?;

    let first = wait_for_sample(&app_sink)?;
    let origin = sample_pts(&first)?;
    println!("{mode}: origin={} ns", origin.nseconds());

    for target_seconds in TARGETS {
        while app_sink.try_pull_sample(gst::ClockTime::ZERO).is_some() {}
        let target = Duration::from_secs(target_seconds);
        let seek_point = timeline
            .seek_point(target)
            .ok_or_else(|| io::Error::other(format!("no seek point for {target_seconds}s")))?;
        let seqnum = gst::Seqnum::next();
        let event = gst::event::Seek::builder(
            1.0,
            gst::SeekFlags::FLUSH,
            gst::SeekType::Set,
            gst::format::Bytes::from_bytes(seek_point.byte_offset()),
            gst::SeekType::None,
            gst::format::Bytes::NONE,
        )
        .seqnum(seqnum)
        .build();
        let started = Instant::now();
        if !pipeline.send_event(event) {
            return Err(io::Error::other(format!(
                "{mode}: pipeline rejected seek to {target_seconds}s"
            ))
            .into());
        }
        let sample = wait_for_sample(&app_sink)?;
        let first_pts = sample_pts(&sample)?.saturating_sub(origin);
        let frame_latency = started.elapsed();
        std::thread::sleep(Duration::from_millis(250));
        println!(
            "{mode}: requested={target_seconds:>3}s indexed={:.3}s offset={} first-pts={:.3}s latency={:.0}ms",
            seek_point.position().as_secs_f64(),
            seek_point.byte_offset(),
            first_pts.seconds_f64(),
            frame_latency.as_secs_f64() * 1000.0,
        );
    }

    pipeline.set_state(gst::State::Null)?;
    Ok(())
}

fn run_d3d_sink_mode(
    input: &Path,
    timeline: &TransportStreamTimeline,
) -> Result<(), Box<dyn Error>> {
    let mode = "d3d12-sink";
    let video_sink = gst::ElementFactory::make("d3d12videosink")
        .name("d3d12-video-sink")
        .property("sync", true)
        .property("adapter", -1_i32)
        .property("force-aspect-ratio", true)
        .property("external-window-only", false)
        .property("direct-swapchain", false)
        .property("enable-last-sample", true)
        .property("error-on-closed", false)
        .build()?;
    let frame_count = Arc::new(AtomicU64::new(0));
    let observed_frame_count = Arc::clone(&frame_count);
    let sink_pad = video_sink
        .static_pad("sink")
        .ok_or_else(|| io::Error::other("D3D12 video sink has no sink pad"))?;
    let _probe = sink_pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, _info| {
        observed_frame_count.fetch_add(1, Ordering::Release);
        gst::PadProbeReturn::Ok
    });

    let input_file = File::open(input)?;
    let input_size = i64::try_from(input_file.metadata()?.len())?;
    let source_caps = gst::Caps::builder("video/mpegts")
        .field("systemstream", true)
        .field("packetsize", 188_i32)
        .build();
    let source_element = gst::ElementFactory::make("appsrc")
        .name("d3d12-source")
        .property("caps", source_caps)
        .property("format", gst::Format::Bytes)
        .property("stream-type", gst_app::AppStreamType::RandomAccess)
        .property("size", input_size)
        .property("block", true)
        .property("max-bytes", 32_u64 * 1024 * 1024)
        .build()?;
    let source = source_element
        .clone()
        .downcast::<gst_app::AppSrc>()
        .map_err(|_| io::Error::other("appsrc has an unexpected runtime type"))?;
    configure_source(&source, input_file);

    let source_queue = make_element("queue", "d3d12-source-queue")?;
    let ts_parse = gst::ElementFactory::make("tsparse")
        .name("d3d12-ts-parse")
        .property("split-on-rai", true)
        .property("ignore-pcr", true)
        .property("skew-corrections", false)
        .build()?;
    let demux = gst::ElementFactory::make("tsdemux")
        .name("d3d12-demux")
        .property("latency", 0_i32)
        .property("skew-corrections", false)
        .build()?;
    let video_queue = make_element("queue", "d3d12-video-queue")?;
    let parser = make_element("h265parse", "d3d12-video-parser")?;
    let decoder = make_element("d3d12h265dec", "d3d12-video-decoder")?;
    let pipeline = gst::Pipeline::builder().name("d3d12-seek-probe").build();
    pipeline.add_many([
        &source_element,
        &source_queue,
        &ts_parse,
        &demux,
        &video_queue,
        &parser,
        &decoder,
        &video_sink,
    ])?;
    gst::Element::link_many([&source_element, &source_queue, &ts_parse, &demux])?;
    gst::Element::link_many([&video_queue, &parser, &decoder, &video_sink])?;

    let queue_sink = video_queue
        .static_pad("sink")
        .ok_or_else(|| io::Error::other("D3D12 video queue has no sink pad"))?;
    let linked = Arc::new(AtomicBool::new(false));
    let linked_for_callback = Arc::clone(&linked);
    demux.connect_pad_added(move |_demux, source_pad| {
        let caps = source_pad
            .current_caps()
            .unwrap_or_else(|| source_pad.query_caps(None));
        let is_h265 = caps
            .structure(0)
            .is_some_and(|structure| structure.name() == "video/x-h265");
        if is_h265
            && linked_for_callback
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            && source_pad.link(&queue_sink).is_err()
        {
            linked_for_callback.store(false, Ordering::Release);
        }
    });

    pipeline.set_state(gst::State::Paused)?;
    let (state_result, current, pending) =
        pipeline.state(gst::ClockTime::from_seconds(FRAME_TIMEOUT.as_secs()));
    state_result.map_err(|error| {
        io::Error::other(format!(
            "{mode}: failed to preroll: {error} (current={current:?}, pending={pending:?})"
        ))
    })?;
    pipeline.set_state(gst::State::Playing)?;
    let initial_latency = wait_for_frame_after(&pipeline, &frame_count, 0)?;
    println!(
        "{mode}: initial frame latency={:.0}ms",
        initial_latency.as_secs_f64() * 1000.0
    );

    for target_seconds in TARGETS {
        let target = Duration::from_secs(target_seconds);
        let seek_point = timeline
            .seek_point(target)
            .ok_or_else(|| io::Error::other(format!("no seek point for {target_seconds}s")))?;
        let event = gst::event::Seek::builder(
            1.0,
            gst::SeekFlags::FLUSH,
            gst::SeekType::Set,
            gst::format::Bytes::from_bytes(seek_point.byte_offset()),
            gst::SeekType::None,
            gst::format::Bytes::NONE,
        )
        .seqnum(gst::Seqnum::next())
        .build();
        let before_seek = frame_count.load(Ordering::Acquire);
        let started = Instant::now();
        if !pipeline.send_event(event) {
            return Err(io::Error::other(format!(
                "{mode}: pipeline rejected seek to {target_seconds}s"
            ))
            .into());
        }
        let first_latency = wait_for_frame_after(&pipeline, &frame_count, before_seek)?;
        let first_frame = frame_count.load(Ordering::Acquire);
        let continued_latency = wait_for_frame_after(&pipeline, &frame_count, first_frame)?;
        if video_sink
            .property::<Option<gst::Sample>>("last-sample")
            .is_none()
        {
            return Err(io::Error::other(format!(
                "{mode}: sink did not render a sample after seek to {target_seconds}s"
            ))
            .into());
        }
        std::thread::sleep(Duration::from_millis(250));
        println!(
            "{mode}: requested={target_seconds:>3}s indexed={:.3}s offset={} first={:.0}ms continued={:.0}ms total={:.0}ms",
            seek_point.position().as_secs_f64(),
            seek_point.byte_offset(),
            first_latency.as_secs_f64() * 1000.0,
            continued_latency.as_secs_f64() * 1000.0,
            started.elapsed().as_secs_f64() * 1000.0,
        );
    }

    pipeline.set_state(gst::State::Null)?;
    Ok(())
}

fn wait_for_frame_after(
    pipeline: &gst::Pipeline,
    frame_count: &AtomicU64,
    previous_count: u64,
) -> Result<Duration, io::Error> {
    let bus = pipeline
        .bus()
        .ok_or_else(|| io::Error::other("seek probe pipeline has no bus"))?;
    let started = Instant::now();
    loop {
        if frame_count.load(Ordering::Acquire) > previous_count {
            return Ok(started.elapsed());
        }
        while let Some(message) = bus.pop() {
            match message.view() {
                gst::MessageView::Error(error) => {
                    return Err(io::Error::other(format!(
                        "D3D12 seek probe error from {}: {}; {}",
                        message
                            .src()
                            .map(|source| source.path_string().to_string())
                            .unwrap_or_else(|| "unknown source".to_owned()),
                        error.error(),
                        error
                            .debug()
                            .map(|debug| debug.to_string())
                            .unwrap_or_default(),
                    )));
                }
                gst::MessageView::Eos(_) => {
                    return Err(io::Error::other(
                        "D3D12 seek probe reached end of stream before another frame",
                    ));
                }
                _ => {}
            }
        }
        if started.elapsed() >= FRAME_TIMEOUT {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("D3D12 video sink did not receive another frame after {previous_count}"),
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn configure_source(source: &gst_app::AppSrc, file: File) {
    let file = Arc::new(Mutex::new(file));
    let read_file = Arc::clone(&file);
    source.set_callbacks(
        gst_app::AppSrcCallbacks::builder()
            .need_data(move |source, requested| {
                let maximum = 188 * 256;
                let requested = usize::try_from(requested)
                    .unwrap_or(maximum)
                    .clamp(188, maximum);
                let requested = requested - requested % 188;
                let mut bytes = vec![0_u8; requested];
                let result = read_file.lock().ok().and_then(|mut file| {
                    let offset = file.stream_position().ok()?;
                    let count = file.read(&mut bytes).ok()?;
                    Some((offset, count))
                });
                let Some((offset, count)) = result else {
                    let _result = source.end_of_stream();
                    return;
                };
                if count == 0 {
                    let _result = source.end_of_stream();
                    return;
                }
                bytes.truncate(count);
                let mut buffer = gst::Buffer::from_mut_slice(bytes);
                if let Some(buffer) = buffer.get_mut() {
                    buffer.set_offset(offset);
                    buffer.set_offset_end(offset + count as u64);
                }
                let _result = source.push_buffer(buffer);
            })
            .seek_data(move |_source, offset| {
                let aligned = offset / 188 * 188;
                file.lock()
                    .ok()
                    .and_then(|mut file| file.seek(SeekFrom::Start(aligned)).ok())
                    .is_some()
            })
            .build(),
    );
}

fn make_element(factory: &str, name: &str) -> Result<gst::Element, io::Error> {
    gst::ElementFactory::make(factory)
        .name(name)
        .build()
        .map_err(|error| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("cannot create {factory}: {error}"),
            )
        })
}

fn wait_for_sample(app_sink: &gst_app::AppSink) -> Result<gst::Sample, io::Error> {
    app_sink
        .try_pull_sample(gst::ClockTime::from_seconds(FRAME_TIMEOUT.as_secs()))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "timed out waiting for decoded frame",
            )
        })
}

fn sample_pts(sample: &gst::Sample) -> Result<gst::ClockTime, io::Error> {
    sample
        .buffer()
        .and_then(|buffer| buffer.pts())
        .ok_or_else(|| io::Error::other("decoded frame has no PTS"))
}
