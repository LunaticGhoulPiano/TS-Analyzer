//! Portable vector plots used by the LaTeX bundle (PDF) and standalone review (SVG).
use crate::report_export::ExportInput;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use tsan_analyzer::ClockKind;

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
fn pdf_text(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}
fn chart(
    directory: &Path,
    name: &str,
    title: &str,
    xlabel: &str,
    ylabel: &str,
    points: &[(f64, f64)],
) -> Result<(), String> {
    let finite = points
        .iter()
        .copied()
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .collect::<Vec<_>>();
    if finite.is_empty() {
        return Ok(());
    }
    let xmin = finite.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let xmax = finite
        .iter()
        .map(|p| p.0)
        .fold(f64::NEG_INFINITY, f64::max)
        .max(xmin + 1.0);
    let rawmin = finite.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let rawmax = finite.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    let (ymin, ymax) = if name.contains("_gop_") {
        crate::plot_range::gop_axis_range(rawmin, rawmax)
    } else {
        let bottom = rawmin.min(0.0);
        (bottom, rawmax + ((rawmax - bottom) * 0.1).max(1.0))
    };
    let pos = |(x, y): (f64, f64)| {
        (
            70.0 + (x - xmin) / (xmax - xmin) * 500.0,
            50.0 + (y - ymin) / (ymax - ymin) * 200.0,
        )
    };
    let mut svg = String::from(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="620" height="320" viewBox="0 0 620 320"><rect width="620" height="320" fill="white"/><g font-family="Arial" font-size="11" fill="#263746">"##,
    );
    let mut commands = String::from("1 1 1 rg 0 0 620 320 re f\n0.15 0.22 0.28 rg\n");
    let mut text = |x: f64, y: f64, label: &str| {
        let _ = write!(
            svg,
            r#"<text x="{x:.2}" y="{:.2}">{}</text>"#,
            320.0 - y,
            escape(label)
        );
        let _ = writeln!(
            commands,
            "BT /F1 11 Tf {x:.2} {y:.2} Td ({}) Tj ET",
            pdf_text(label)
        );
    };
    text(70.0, 295.0, title);
    text(70.0, 272.0, ylabel);
    text(230.0, 14.0, xlabel);
    for i in 0..=4 {
        let t = f64::from(i) / 4.0;
        text(
            5.0,
            48.0 + t * 200.0,
            &if ylabel.ends_with("(bytes)") || ylabel == "Pictures per GOP" {
                format!("{:.0}", ymin + (ymax - ymin) * t)
            } else {
                format!("{:.2}", ymin + (ymax - ymin) * t)
            },
        );
        text(
            65.0 + t * 500.0,
            32.0,
            &format!("{:.0}", xmin + (xmax - xmin) * t),
        );
    }
    svg.push_str("</g>");
    commands.push_str("0.8 0.83 0.85 RG 0.5 w\n");
    for i in 0..=4 {
        let y = 50.0 + f64::from(i) * 50.0;
        let _ = write!(
            svg,
            r##"<path d="M70 {} H570" stroke="#ccd3da"/>"##,
            320.0 - y
        );
        let _ = writeln!(commands, "70 {y} m 570 {y} l S");
    }
    // Min/max envelope keeps spikes visible when a long recording is reduced for a report page.
    let mut reduced = Vec::new();
    for chunk in finite.chunks(finite.len().div_ceil(1000).max(1)) {
        let min = chunk
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.1.total_cmp(&b.1.1));
        let max = chunk
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.1.total_cmp(&b.1.1));
        if let (Some(a), Some(b)) = (min, max) {
            if a.0 <= b.0 {
                reduced.push(*a.1);
                reduced.push(*b.1);
            } else {
                reduced.push(*b.1);
                reduced.push(*a.1);
            }
        }
    }
    svg.push_str(r##"<polyline fill="none" stroke="#176fa6" stroke-width="1.2" points=""##);
    commands.push_str("0.09 0.44 0.65 RG 1.2 w\n");
    for (i, point) in reduced.iter().enumerate() {
        let (x, y) = pos(*point);
        let _ = write!(svg, "{x:.2},{:.2} ", 320.0 - y);
        let _ = writeln!(commands, "{x:.2} {y:.2} {}", if i == 0 { "m" } else { "l" });
    }
    svg.push_str("\"/></svg>");
    commands.push_str("S\n");
    fs::write(directory.join(format!("{name}.svg")), svg).map_err(|e| e.to_string())?;
    let objects=["<< /Type /Catalog /Pages 2 0 R >>".to_owned(),"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 620 320] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_owned(),"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),format!("<< /Length {} >>\nstream\n{}endstream",commands.len(),commands)];
    let mut pdf = String::from("%PDF-1.4\n");
    let mut offsets = vec![0];
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        let _ = write!(pdf, "{} 0 obj\n{obj}\nendobj\n", i + 1);
    }
    let xref = pdf.len();
    let _ = write!(pdf, "xref\n0 6\n0000000000 65535 f \n");
    for offset in &offsets[1..] {
        let _ = writeln!(pdf, "{offset:010} 00000 n ");
    }
    let _ = write!(
        pdf,
        "trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
    );
    fs::write(directory.join(format!("{name}.pdf")), pdf).map_err(|e| e.to_string())
}

pub fn write_sections(
    sections: &Path,
    relative: &str,
    inputs: &[ExportInput],
) -> Result<(), String> {
    let images = sections.join("images");
    fs::create_dir_all(&images).map_err(|e| e.to_string())?;
    let mut bitrate = String::from(
        "\\section{Bitrate}\nPCR-derived rates; samples without a local PCR interval use the median rate.\\par\n",
    );
    let mut clocks = String::from(
        "\\section{PCR / PTS / DTS}\nRaw clock ticks are preserved; PCR uses 27 MHz, PTS and DTS use 90 kHz.\\par\n",
    );
    let mut gops = String::from(
        "\\section{GOP}\nStructure is in bitstream/decode order. Length counts coded pictures. Program TS bytes count PAT, PMT, PCR and elementary-stream packets, including both GOP boundary packets; plot at the next GOP start. Adjacent GOPs share a boundary packet, so these values are not additive. VCL bytes exclude TS/PES headers, start codes and non-VCL NALs. Partial or damaged GOPs are excluded from statistics and charts.\\par\n",
    );
    for (i, input) in inputs.iter().enumerate() {
        let report = &input.report;
        let heading = format!(
            "\\subsection{{{}}}\n",
            super::report_export::tex(input.path.file_name().unwrap_or_default().to_string_lossy())
        );
        bitrate.push_str(&heading);
        clocks.push_str(&heading);
        gops.push_str(&heading);
        let include = |out: &mut String,
                       name: &str,
                       title: &str,
                       x: &str,
                       y: &str,
                       points: Vec<(f64, f64)>|
         -> Result<(), String> {
            if points.is_empty() {
                return Ok(());
            }
            chart(&images, name, title, x, y, &points)?;
            let _ = writeln!(
                out,
                "\\includegraphics[width=\\linewidth]{{{relative}/images/{name}.pdf}}\\par"
            );
            Ok(())
        };
        if let Some(rates) = tsan_analyzer::bitrate_series(report) {
            include(
                &mut bitrate,
                &format!("file_{i}_bitrate"),
                "Transport bitrate",
                "TS packet number",
                "Bitrate (Mb/s)",
                report
                    .bitrate_windows
                    .iter()
                    .zip(rates.window_mbps)
                    .map(|(w, r)| (w.first_packet as f64, r))
                    .collect(),
            )?;
        }
        for (kind, label) in [
            (ClockKind::Pcr, "PCR"),
            (ClockKind::Pts, "PTS"),
            (ClockKind::Dts, "DTS"),
        ] {
            let pids = report
                .clock_points
                .iter()
                .filter(|p| p.kind == kind)
                .map(|p| p.pid)
                .collect::<std::collections::BTreeSet<_>>();
            for pid in pids {
                include(
                    &mut clocks,
                    &format!("file_{i}_{label}_{pid}"),
                    &format!("{label} PID {pid}"),
                    "TS packet number",
                    "Raw timestamp (seconds)",
                    report
                        .clock_points
                        .iter()
                        .filter(|p| p.kind == kind && p.pid == pid)
                        .map(|p| {
                            (
                                p.packet_index as f64,
                                p.ticks as f64
                                    / if kind == ClockKind::Pcr {
                                        27_000_000.0
                                    } else {
                                        90_000.0
                                    },
                            )
                        })
                        .collect(),
                )?;
            }
        }
        for (pid, video) in &report.video_gops {
            let _ = writeln!(gops, "\\subsubsection{{Video PID {pid}}}");
            for bytes in [false, true] {
                include(
                    &mut gops,
                    &format!(
                        "file_{i}_gop_{pid}_{}",
                        if bytes { "bytes" } else { "length" }
                    ),
                    if bytes { "GOP Bytes" } else { "GOP Length" },
                    if bytes {
                        "Next GOP start: TS packet number"
                    } else {
                        "GOP start: TS packet number"
                    },
                    if bytes {
                        "Program TS (bytes)"
                    } else {
                        "Pictures per GOP"
                    },
                    video
                        .gops
                        .iter()
                        .enumerate()
                        .filter(|(_, g)| g.complete && (!bytes || g.program_ts_bytes.is_some()))
                        .map(|(_, g)| {
                            (
                                if bytes {
                                    g.next_packet.unwrap_or(g.first_packet)
                                } else {
                                    g.first_packet
                                } as f64,
                                if bytes {
                                    g.program_ts_bytes.unwrap_or(0) as f64
                                } else {
                                    g.pictures.len() as f64
                                },
                            )
                        })
                        .collect(),
                )?;
            }
            gops.push_str("\\begin{longtable}{r r r r r l p{48mm}}\\toprule\nGOP & \\shortstack{Start\\\\packet} & Length & \\shortstack{Program\\\\bytes} & \\shortstack{VCL\\\\bytes} & Status & Structure \\\\ \\midrule\\endhead\n");
            for (index, gop) in video.gops.iter().enumerate() {
                let _ = writeln!(
                    gops,
                    "{index} & {} & {} & {} & {} & {} & {} \\\\",
                    gop.first_packet,
                    gop.pictures.len(),
                    gop.program_ts_bytes
                        .map(|b| b.to_string())
                        .unwrap_or_else(|| "---".into()),
                    gop.vcl_bytes,
                    if gop.complete { "Complete" } else { "Partial" },
                    gop.structure()
                );
            }
            gops.push_str("\\bottomrule\\end{longtable}\n");
        }
    }
    for (name, content) in [("Bitrate", bitrate), ("Timestamps", clocks), ("GOP", gops)] {
        fs::write(sections.join(format!("{name}.tex")), content).map_err(|e| e.to_string())?;
    }
    fs::write(sections.join("Graphs.tex"),format!("\\input{{{relative}/Bitrate.tex}}\\clearpage\n\\input{{{relative}/Timestamps.tex}}\\clearpage\n\\input{{{relative}/GOP.tex}}\n")).map_err(|e|e.to_string())
}
