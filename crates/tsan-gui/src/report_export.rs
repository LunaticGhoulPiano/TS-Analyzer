use std::fmt::Write as _;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use tsan_analyzer::{AnalysisReport, ClockKind};
use windows::Win32::System::SystemInformation::GetLocalTime;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReportFormat {
    Cbor,
    Xlsx,
    Latex,
    Pdf,
}

impl ReportFormat {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Cbor => "Raw data (.cbor)",
            Self::Xlsx => "Excel workbook (.xlsx)",
            Self::Latex => "LaTeX source (.tex + sections)",
            Self::Pdf => "Management PDF (.pdf)",
        }
    }

    pub const fn extension(self) -> &'static str {
        match self {
            Self::Cbor => "cbor",
            Self::Xlsx => "xlsx",
            Self::Latex => "tex",
            Self::Pdf => "pdf",
        }
    }
}

pub struct ExportInput {
    pub path: PathBuf,
    pub report: AnalysisReport,
}

#[allow(unsafe_code)]
fn local_date() -> String {
    // SAFETY: GetLocalTime returns a value and does not retain caller memory.
    let now = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} local time",
        now.wYear, now.wMonth, now.wDay, now.wHour, now.wMinute, now.wSecond
    )
}

pub fn export(
    format: ReportFormat,
    destination: &Path,
    inputs: &[ExportInput],
) -> Result<(), String> {
    if inputs.is_empty() {
        return Err("Select at least one analyzed TS file.".to_owned());
    }
    match format {
        ReportFormat::Cbor => fs::write(destination, encode_cbor(inputs))
            .map_err(|error| format!("Could not write {}: {error}", destination.display())),
        ReportFormat::Xlsx => crate::xlsx::export(destination, &encode_cbor(inputs)),
        ReportFormat::Latex => write_latex_bundle(destination, inputs, &sections_name(destination)),
        ReportFormat::Pdf => export_pdf(destination, inputs),
    }
}

fn sections_name(main: &Path) -> String {
    let stem = main
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("ts-analyzer_report");
    format!(
        "{}_sections",
        stem.chars()
            .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            })
            .collect::<String>()
    )
}

fn xelatex_command() -> Result<Command, String> {
    let Some(root) = std::env::var_os("TSAN_TEX_ROOT").map(PathBuf::from) else {
        return Ok(crate::os_integration::background_command("xelatex"));
    };
    let executable = root.join("bin/windows/xelatex.exe");
    if !executable.is_file() {
        return Err(format!(
            "Bundled XeLaTeX is missing: {}",
            executable.display()
        ));
    }
    let mut runtime_id = std::hash::DefaultHasher::new();
    root.hash(&mut runtime_id);
    let cache = std::env::var_os("TSAN_CACHE_ROOT")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| tsan_platform::paths::cache_directory().ok())
        .ok_or("Application cache location is unavailable")?
        .join("tex")
        .join(format!("{:016x}", runtime_id.finish()));
    fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
    let mut command = crate::os_integration::background_command(executable);
    command.env_clear();
    for name in [
        "SystemRoot",
        "WINDIR",
        "TEMP",
        "TMP",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "COMSPEC",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let windows = tsan_platform::paths::windows_directory()?;
    command
        .env("SystemRoot", &windows)
        .env("WINDIR", &windows)
        .env(
            "PATH",
            std::env::join_paths([root.join("bin/windows"), windows.join("System32")])
                .map_err(|e| e.to_string())?,
        )
        .env("TEXMFROOT", &root)
        .env("TEXMFCNF", root.join("texmf-dist/web2c"))
        .env("TEXMFDIST", root.join("texmf-dist"))
        .env("TEXMFSYSVAR", root.join("texmf-var"))
        .env("TEXMFSYSCONFIG", root.join("texmf-config"))
        .env("TEXMFLOCAL", root.join("empty"))
        .env("TEXMFHOME", root.join("empty"))
        .env("TEXMFVAR", &cache)
        .env("TEXMFCONFIG", &cache)
        .env(
            "TEXMF",
            format!(
                "{{{}, {}}}",
                root.join("texmf-dist").display(),
                root.join("texmf-var").display()
            )
            .replace('\\', "/")
            .replace(", ", ","),
        )
        .env(
            "TEXFORMATS",
            format!("{}//", root.join("texmf-var/web2c").display()).replace('\\', "/"),
        );
    let font_config = cache.join("fonts.conf");
    let xml = |path: &Path| {
        path.to_string_lossy()
            .replace('\\', "/")
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('"', "&quot;")
    };
    fs::write(&font_config, format!(
        "<?xml version=\"1.0\"?><!DOCTYPE fontconfig SYSTEM \"fonts.dtd\"><fontconfig><dir>{}</dir><dir>{}</dir><cachedir>{}</cachedir></fontconfig>",
        xml(&windows.join("Fonts")), xml(&root.join("texmf-dist/fonts/opentype")), xml(&cache.join("fonts"))
    )).map_err(|e| e.to_string())?;
    command.env("FONTCONFIG_FILE", font_config);
    Ok(command)
}

fn export_pdf(destination: &Path, inputs: &[ExportInput]) -> Result<(), String> {
    let main = destination.with_extension("tex");
    let section_dir = sections_name(&main);
    write_latex_bundle(&main, inputs, &section_dir)?;
    let parent = main.parent().ok_or("Report path has no parent")?;
    let build = parent.join(&section_dir).join("build");
    fs::create_dir_all(&build).map_err(|e| e.to_string())?;
    for _ in 0..2 {
        let output = xelatex_command()?
            .current_dir(parent)
            .args([
                "-no-shell-escape",
                "-recorder",
                "-interaction=nonstopmode",
                "-halt-on-error",
                "-jobname=report",
            ])
            .arg(format!("-output-directory={}", build.display()))
            .arg(main.file_name().ok_or("Missing report filename")?)
            .output()
            .map_err(|e| format!("XeLaTeX is required for PDF export: {e}"))?;
        if !output.status.success() {
            let log = String::from_utf8_lossy(&output.stdout);
            return Err(format!(
                "XeLaTeX failed; source and build log retained in {}:\n{}",
                build.display(),
                log.lines()
                    .rev()
                    .take(16)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
    }
    fs::copy(build.join("report.pdf"), destination).map_err(|e| e.to_string())?;
    Ok(())
}

fn write_latex_bundle(
    main: &Path,
    inputs: &[ExportInput],
    section_dir: &str,
) -> Result<(), String> {
    let parent = main
        .parent()
        .ok_or("Report path has no parent directory.")?;
    let sections = parent.join(section_dir);
    fs::create_dir_all(&sections).map_err(|error| error.to_string())?;
    let mut preamble = String::from(
        "\\documentclass[11pt,a4paper]{article}\n\\usepackage[margin=18mm]{geometry}\n\\usepackage{fontspec}\n\\IfFontExistsTF{Microsoft JhengHei}{\\setmainfont{Microsoft JhengHei}}{\\setmainfont{FandolSong-Regular.otf}[BoldFont=FandolSong-Bold.otf]}\n\\usepackage{longtable,booktabs,array,hyperref,graphicx}\n\\hypersetup{hidelinks}\n\\setcounter{tocdepth}{1}\n\\raggedbottom\n\\begin{document}\n\\title{Transport Stream Analysis}\n\\author{TS Analyzer}\n",
    );
    let _ = writeln!(preamble, "\\date{{{}}}\\maketitle", local_date());
    preamble.push_str("\\section*{Included files}\\begin{enumerate}\n");
    for input in inputs {
        let name = input.path.file_name().map_or_else(
            || input.path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let _ = writeln!(preamble, "\\item {}", tex(name));
    }
    preamble.push_str("\\end{enumerate}\\bigskip\\tableofcontents\\clearpage\n");
    for title in ["Overview", "PSI_SI_Tree", "TR_101_290", "Graphs"] {
        let _ = writeln!(preamble, "\\input{{{section_dir}/{title}.tex}}\\clearpage");
    }
    preamble.push_str("\\end{document}\n");
    fs::write(main, preamble).map_err(|error| error.to_string())?;
    fs::write(sections.join("Overview.tex"), overview_tex(inputs))
        .map_err(|error| error.to_string())?;
    fs::write(sections.join("PSI_SI_Tree.tex"), psi_tex(inputs))
        .map_err(|error| error.to_string())?;
    fs::write(sections.join("TR_101_290.tex"), tr101290_tex(inputs))
        .map_err(|error| error.to_string())?;
    crate::report_graphics::write_sections(&sections, section_dir, inputs)?;
    Ok(())
}

pub(crate) fn tex(text: impl AsRef<str>) -> String {
    let mut result = String::new();
    for ch in text.as_ref().chars() {
        match ch {
            '\\' => result.push_str("\\textbackslash{}"),
            '{' => result.push_str("\\{"),
            '}' => result.push_str("\\}"),
            '#' => result.push_str("\\#"),
            '$' => result.push_str("\\$"),
            '%' => result.push_str("\\%"),
            '&' => result.push_str("\\&"),
            '_' => result.push_str("\\_"),
            '^' => result.push_str("\\textasciicircum{}"),
            '~' => result.push_str("\\textasciitilde{}"),
            _ => result.push(ch),
        }
    }
    result
}

fn file_heading(output: &mut String, input: &ExportInput) {
    let name = input.path.file_name().map_or_else(
        || input.path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let _ = writeln!(output, "\\subsection{{{}}}", tex(name));
    let source =
        tex(input.path.display().to_string().replace('\\', "/")).replace('/', "/\\allowbreak{}");
    let _ = writeln!(output, "\\noindent Source file: \\texttt{{{source}}}\\par");
}

fn row(output: &mut String, label: &str, value: impl std::fmt::Display) {
    let _ = writeln!(output, "{} & {} \\\\", tex(label), tex(value.to_string()));
}

fn overview_tex(inputs: &[ExportInput]) -> String {
    let mut output = String::from("\\section{Overview}\n");
    for input in inputs {
        file_heading(&mut output, input);
        let report = &input.report;
        let media = report.media_information();
        output.push_str(
            "\\subsubsection{Media Information}\n\\begin{longtable}{p{48mm}p{105mm}}\\toprule\n",
        );
        row(&mut output, "Standard", format!("{:?}", report.standard));
        row(&mut output, "Video", media.video_codecs.join(", "));
        row(
            &mut output,
            "Resolution",
            match (media.video_width, media.video_height) {
                (Some(width), Some(height)) => format!("{width} × {height} pixels"),
                _ => "Not detected".to_owned(),
            },
        );
        row(
            &mut output,
            "Frame rate",
            media.video_frame_rate.map_or_else(
                || "Not detected".to_owned(),
                |(n, d)| format!("{:.3} frames/s", n as f64 / d as f64),
            ),
        );
        row(&mut output, "Audio", media.audio_codecs.join(", "));
        row(&mut output, "Audio tracks", media.audio_tracks);
        output.push_str("\\bottomrule\\end{longtable}\n");
        output.push_str(
            "\\subsubsection{Transport Stream}\n\\begin{longtable}{p{48mm}p{105mm}}\\toprule\n",
        );
        row(&mut output, "Packet format", format!("{:?}", report.format));
        row(
            &mut output,
            "TS packets",
            format!("{} packets", report.packets),
        );
        row(
            &mut output,
            "Malformed packets",
            format!("{} packets", report.malformed_packets),
        );
        row(
            &mut output,
            "Trailing bytes",
            format!("{} bytes", report.trailing_bytes),
        );
        row(
            &mut output,
            "Null packets",
            format!("{} packets", report.null_packets),
        );
        row(&mut output, "Programs", report.programs.len());
        row(&mut output, "PIDs", report.pids.len());
        row(
            &mut output,
            "Valid PSI/SI",
            format!("{} sections", report.valid_sections),
        );
        row(
            &mut output,
            "Section CRC errors",
            format!("{} sections", report.section_crc_errors),
        );
        output.push_str("\\bottomrule\\end{longtable}\n");
    }
    output
}

fn psi_tex(inputs: &[ExportInput]) -> String {
    let mut output = String::from("\\section{PSI / SI Tree}\n");
    for input in inputs {
        file_heading(&mut output, input);
        let report = &input.report;
        output.push_str("\\subsubsection{Observed tables}\n");
        if report.tables.is_empty() {
            output.push_str("No PSI/SI sections observed.\\par\n");
        } else {
            output.push_str("\\begin{longtable}{llll}\\toprule\nPID & Table ID & Sections & CRC errors \\\\ \\midrule\n");
            for ((pid, id), table) in &report.tables {
                let _ = writeln!(
                    output,
                    "0x{pid:04X} & 0x{id:02X} & {} & {} \\\\",
                    table.sections, table.crc_errors
                );
            }
            output.push_str("\\bottomrule\\end{longtable}\n");
        }
        output.push_str("\\subsubsection{Programs and elementary streams}\n");
        if report.programs.is_empty() {
            output.push_str("No PAT/PMT program structure observed.\\par\n");
        }
        for (number, program) in &report.programs {
            let _ = writeln!(
                output,
                "\\paragraph{{Program {number}}} PMT PID 0x{:04X}; PCR PID {}.\\par",
                program.pmt_pid,
                program
                    .pcr_pid
                    .map_or_else(|| "unknown".to_owned(), |pid| format!("0x{pid:04X}"))
            );
            for (pid, stream) in &program.streams {
                let _ = writeln!(
                    output,
                    "ES PID 0x{pid:04X}: {} (stream type 0x{:02X})\\par",
                    tex(stream.name_for_standard(report.standard)),
                    stream.stream_type
                );
            }
        }
        output.push_str("\\subsubsection{Packets: aggregated statistics}\n");
        output.push_str("\\begin{longtable}{lllll}\\toprule\nPID & Packets & Payload & CC errors & TEI \\\\ \\midrule\n");
        for (pid, data) in &report.pids {
            let _ = writeln!(
                output,
                "0x{pid:04X} & {} & {} & {} & {} \\\\",
                data.packets, data.payload_packets, data.continuity_errors, data.transport_errors
            );
        }
        output.push_str("\\bottomrule\\end{longtable}\n");
    }
    output
}

fn tr101290_tex(inputs: &[ExportInput]) -> String {
    let mut output = String::from("\\section{TR 101 290}\n");
    for input in inputs {
        file_heading(&mut output, input);
        let profile = tsan_analyzer::ComplianceProfile::suggested(&input.report);
        let summary = tsan_analyzer::tr101290_report(&input.report, profile);
        let _ = writeln!(
            output,
            "\\noindent Applied profile: {}.\\par",
            tex(summary.profile.label())
        );
        let _ = writeln!(
            output,
            "\\noindent Family: {}; System: {}; Signalling: {}; Delivery: {}.\\par",
            tex(summary.hierarchy.family.label()),
            tex(summary.hierarchy.system.label()),
            tex(summary.hierarchy.signaling),
            tex(summary.hierarchy.delivery)
        );
        output.push_str("\\begin{itemize}\n");
        for standard in summary.standards {
            let _ = writeln!(
                output,
                "\\item {} --- {} ({})",
                tex(standard.code),
                tex(standard.title),
                tex(standard.scope)
            );
        }
        output.push_str("\\end{itemize}\n");
        let groups = summary
            .indicators
            .iter()
            .map(|item| item.group)
            .collect::<std::collections::BTreeSet<_>>();
        for group in groups {
            let _ = writeln!(
                output,
                "\\subsubsection{{{}}}\\begin{{longtable}}{{p{{55mm}}p{{25mm}}p{{20mm}}p{{55mm}}}}\\toprule\nIndicator & Status & Observed & Standard \\\\ \\midrule",
                tex(group)
            );
            for item in summary.indicators.iter().filter(|item| item.group == group) {
                let observed = item
                    .observed
                    .map_or_else(|| "---".to_owned(), |value| value.to_string());
                let _ = writeln!(
                    output,
                    "{} & {} & {} & {} \\\\",
                    tex(item.name),
                    tex(item.status.label()),
                    observed,
                    tex(item.reference)
                );
            }
            output.push_str("\\bottomrule\\end{longtable}\n");
        }
        let _ = writeln!(
            output,
            "\\noindent Event records: {}.\\par",
            summary.events.len()
        );
    }
    output.push_str("\\noindent Unmeasured, inapplicable and unimplemented checks are reported as distinct states.\\par\n");
    output
}

struct Cbor(Vec<u8>);

impl Cbor {
    fn major(&mut self, kind: u8, value: u64) {
        let prefix = kind << 5;
        if value < 24 {
            self.0.push(prefix | value as u8);
        } else if value <= u8::MAX as u64 {
            self.0.extend_from_slice(&[prefix | 24, value as u8]);
        } else if value <= u16::MAX as u64 {
            self.0.push(prefix | 25);
            self.0.extend_from_slice(&(value as u16).to_be_bytes());
        } else if value <= u32::MAX as u64 {
            self.0.push(prefix | 26);
            self.0.extend_from_slice(&(value as u32).to_be_bytes());
        } else {
            self.0.push(prefix | 27);
            self.0.extend_from_slice(&value.to_be_bytes());
        }
    }
    fn uint(&mut self, value: u64) {
        self.major(0, value);
    }
    fn array(&mut self, len: usize) {
        self.major(4, len as u64);
    }
    fn map(&mut self, len: usize) {
        self.major(5, len as u64);
    }
    fn bytes(&mut self, data: &[u8]) {
        self.major(2, data.len() as u64);
        self.0.extend_from_slice(data);
    }
    fn text(&mut self, data: &str) {
        self.major(3, data.len() as u64);
        self.0.extend_from_slice(data.as_bytes());
    }
    fn bool(&mut self, value: bool) {
        self.0.push(if value { 0xf5 } else { 0xf4 });
    }
    fn optional(&mut self, value: Option<u64>) {
        if let Some(value) = value {
            self.uint(value);
        } else {
            self.0.push(0xf6);
        }
    }
}

fn encode_cbor(inputs: &[ExportInput]) -> Vec<u8> {
    let mut cbor = Cbor(Vec::new());
    cbor.map(3);
    cbor.uint(0);
    cbor.uint(5); // schema version: explicit program TS bytes and closing GOP boundary
    cbor.uint(1);
    cbor.uint(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |time| time.as_secs()),
    );
    cbor.uint(2);
    cbor.array(inputs.len());
    for input in inputs {
        let report = &input.report;
        cbor.map(15);
        cbor.uint(0);
        cbor.text(&input.path.to_string_lossy());
        cbor.uint(1);
        cbor.text(&format!("{:?}", report.format));
        cbor.uint(2);
        cbor.text(&format!("{:?}", report.standard));
        cbor.uint(3);
        cbor.array(6);
        for value in [
            report.packets,
            report.malformed_packets,
            report.trailing_bytes,
            report.null_packets,
            report.valid_sections,
            report.section_crc_errors,
        ] {
            cbor.uint(value);
        }
        cbor.uint(4);
        cbor.array(report.pids.len());
        for (pid, data) in &report.pids {
            cbor.array(9);
            for value in [
                u64::from(*pid),
                data.packets,
                data.payload_packets,
                data.transport_errors,
                data.continuity_errors,
                data.duplicates,
                data.scrambled_packets,
                data.pcr_samples,
            ] {
                cbor.uint(value);
            }
            cbor.optional(data.max_pcr_gap_27mhz);
        }
        cbor.uint(5);
        cbor.array(report.programs.len());
        for (number, program) in &report.programs {
            cbor.array(4);
            cbor.uint(u64::from(*number));
            cbor.uint(u64::from(program.pmt_pid));
            cbor.optional(program.pcr_pid.map(u64::from));
            cbor.array(program.streams.len());
            for (pid, stream) in &program.streams {
                cbor.array(2);
                cbor.uint(u64::from(*pid));
                cbor.uint(u64::from(stream.stream_type));
            }
        }
        cbor.uint(6);
        cbor.array(report.tables.len());
        for ((pid, id), table) in &report.tables {
            cbor.array(8);
            cbor.uint(u64::from(*pid));
            cbor.uint(u64::from(*id));
            cbor.uint(table.sections);
            cbor.uint(table.crc_errors);
            cbor.optional(table.version.map(u64::from));
            cbor.optional(table.section_number.map(u64::from));
            cbor.optional(table.last_section_number.map(u64::from));
            cbor.bytes(&table.first_section);
        }
        cbor.uint(7);
        cbor.array(report.clock_points.len());
        for point in &report.clock_points {
            cbor.array(4);
            cbor.uint(point.packet_index);
            cbor.uint(u64::from(point.pid));
            cbor.uint(match point.kind {
                ClockKind::Pcr => 0,
                ClockKind::Pts => 1,
                ClockKind::Dts => 2,
            });
            cbor.uint(point.ticks);
        }
        cbor.uint(8);
        cbor.array(report.random_access_points.len());
        for (packet, pid) in &report.random_access_points {
            cbor.array(2);
            cbor.uint(*packet);
            cbor.uint(u64::from(*pid));
        }
        cbor.uint(9);
        cbor.array(report.video_metadata.len());
        for (pid, metadata) in &report.video_metadata {
            cbor.array(4);
            cbor.uint(u64::from(*pid));
            cbor.optional(metadata.width.map(u64::from));
            cbor.optional(metadata.height.map(u64::from));
            if let Some((numerator, denominator)) = metadata.frame_rate {
                cbor.array(2);
                cbor.uint(u64::from(numerator));
                cbor.uint(u64::from(denominator));
            } else {
                cbor.0.push(0xf6);
            }
        }
        cbor.uint(10);
        if let Some(native) = report.native {
            cbor.array(6);
            for value in [
                native.packets,
                native.continuity_errors,
                native.valid_sections,
                native.pat_sections,
                native.pmt_sections,
                u64::from(native.standards),
            ] {
                cbor.uint(value);
            }
        } else {
            cbor.0.push(0xf6);
        }
        cbor.uint(11);
        cbor.optional(
            fs::metadata(&input.path)
                .ok()
                .map(|metadata| metadata.len()),
        );
        cbor.uint(12);
        let compliance = tsan_analyzer::tr101290_report(
            report,
            tsan_analyzer::ComplianceProfile::suggested(report),
        );
        cbor.map(3);
        cbor.uint(0);
        cbor.array(compliance.indicators.len());
        for item in &compliance.indicators {
            cbor.array(6);
            cbor.text(item.group);
            cbor.text(item.name);
            cbor.text(item.status.label());
            cbor.optional(item.observed);
            cbor.text(item.note);
            cbor.text(item.reference);
        }
        cbor.uint(1);
        cbor.array(compliance.events.len());
        for event in &compliance.events {
            cbor.array(6);
            cbor.uint(event.packet_index);
            cbor.uint(report.packet_offset(event.packet_index));
            cbor.uint(u64::from(event.pid));
            cbor.text(event.indicator);
            cbor.text(&event.detail);
            cbor.bool(event.exact_packet);
        }
        cbor.uint(2);
        cbor.array(4);
        cbor.text(compliance.hierarchy.family.label());
        cbor.text(compliance.hierarchy.system.label());
        cbor.text(compliance.hierarchy.signaling);
        cbor.text(compliance.hierarchy.delivery);
        cbor.uint(13);
        cbor.array(report.video_gops.len());
        for (pid, video) in &report.video_gops {
            cbor.array(6);
            cbor.uint(u64::from(*pid));
            cbor.uint(u64::from(video.stream_type));
            cbor.uint(video.frame_count as u64);
            cbor.array(video.gops.len());
            for gop in &video.gops {
                cbor.array(8);
                cbor.uint(gop.first_packet);
                cbor.optional(gop.start_pts);
                cbor.uint(gop.pictures.len() as u64);
                cbor.uint(gop.vcl_bytes);
                cbor.bool(gop.complete);
                cbor.text(&gop.structure());
                cbor.optional(gop.next_packet);
                cbor.optional(gop.program_ts_bytes);
            }
            cbor.optional(video.program_number.map(u64::from));
            cbor.array(video.program_pids.len());
            for &program_pid in &video.program_pids {
                cbor.uint(u64::from(program_pid));
            }
        }
        cbor.uint(14);
        cbor.array(report.bitrate_windows.len());
        let rates = tsan_analyzer::bitrate_series(report);
        for (index, window) in report.bitrate_windows.iter().enumerate() {
            cbor.array(4);
            cbor.uint(window.first_packet);
            cbor.uint(u64::from(window.packet_count));
            cbor.map(window.pid_packets.len());
            for (pid, count) in &window.pid_packets {
                cbor.uint(u64::from(*pid));
                cbor.uint(u64::from(*count));
            }
            if let Some(rate) = rates.as_ref().and_then(|r| r.window_mbps.get(index)) {
                cbor.0.push(0xfb);
                cbor.0.extend_from_slice(&rate.to_be_bytes());
            } else {
                cbor.0.push(0xf6);
            }
        }
    }
    cbor.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_latex_control_characters() {
        assert_eq!(tex("a_b%{c}"), "a\\_b\\%\\{c\\}");
    }

    #[test]
    fn cbor_unsigned_integer_uses_shortest_width() {
        let mut writer = Cbor(Vec::new());
        writer.uint(23);
        writer.uint(24);
        writer.uint(256);
        assert_eq!(writer.0, [23, 24, 24, 25, 1, 0]);
    }

    #[test]
    fn exports_analysis_in_four_formats() -> Result<(), Box<dyn std::error::Error>> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |time| time.as_nanos());
        let directory =
            std::env::temp_dir().join(format!("tsan-export-test-{}-{nonce}", std::process::id()));
        fs::create_dir(&directory)?;
        let source = directory.join("sample.ts");
        let mut bytes = Vec::new();
        for counter in 0..10 {
            let mut packet = [0xff; 188];
            packet[..4].copy_from_slice(&[0x47, 0x1f, 0xff, 0x10 | counter]);
            bytes.extend_from_slice(&packet);
        }
        fs::write(&source, bytes)?;
        let report = tsan_analyzer::analyze_file(&source)?;
        let input = [ExportInput {
            path: source,
            report,
        }];
        let cbor = directory.join("report.cbor");
        export(ReportFormat::Cbor, &cbor, &input)?;
        assert!(fs::metadata(&cbor)?.len() > 20);
        let xlsx = directory.join("report.xlsx");
        export(ReportFormat::Xlsx, &xlsx, &input)?;
        assert!(fs::metadata(xlsx)?.len() > 1000);
        let latex = directory.join("report.tex");
        export(ReportFormat::Latex, &latex, &input)?;
        for name in [
            "Overview.tex",
            "PSI_SI_Tree.tex",
            "TR_101_290.tex",
            "Graphs.tex",
            "Bitrate.tex",
            "Timestamps.tex",
            "GOP.tex",
        ] {
            assert!(directory.join("report_sections").join(name).is_file());
        }
        if Command::new("xelatex").arg("--version").output().is_ok() {
            let pdf = directory.join("report.pdf");
            export(ReportFormat::Pdf, &pdf, &input)?;
            assert!(fs::metadata(&pdf)?.len() > 1000);
        }
        if std::env::var_os("TSAN_REPORT_KEEP_TEST").is_some() {
            println!(
                "PDF test report: {}",
                directory.join("report.pdf").display()
            );
        } else {
            fs::remove_dir_all(directory)?;
        }
        Ok(())
    }
    #[test]
    #[ignore = "exports local recordings; set TSAN_REPORT_TS_DIR and TSAN_REPORT_OUTPUT"]
    fn export_recordings() -> Result<(), Box<dyn std::error::Error>> {
        let directory =
            std::env::var_os("TSAN_REPORT_TS_DIR").ok_or("Missing TSAN_REPORT_TS_DIR")?;
        let output = PathBuf::from(
            std::env::var_os("TSAN_REPORT_OUTPUT").ok_or("Missing TSAN_REPORT_OUTPUT")?,
        );
        fs::create_dir_all(&output)?;
        let mut paths = fs::read_dir(directory)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ts")))
            .collect::<Vec<_>>();
        paths.sort();
        if let Ok(filter) = std::env::var("TSAN_REPORT_FILTER") {
            paths.retain(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with(&filter))
            });
        }
        for path in paths {
            let stem = path
                .file_stem()
                .ok_or("Missing stem")?
                .to_string_lossy()
                .into_owned();
            let input = [ExportInput {
                report: tsan_analyzer::analyze_file(&path)?,
                path,
            }];
            for format in [ReportFormat::Cbor, ReportFormat::Xlsx, ReportFormat::Latex] {
                export(
                    format,
                    &output.join(format!("{stem}.{}", format.extension())),
                    &input,
                )?;
            }
            if stem.starts_with("M25") {
                export(
                    ReportFormat::Pdf,
                    &output.join(format!("{stem}.pdf")),
                    &input,
                )?;
            }
            println!("Export verified: {stem}");
        }
        Ok(())
    }
}
