//! Excel export consumes the same versioned CBOR snapshot as raw-data export.
//! OOXML uses stored ZIP entries: no Office, LaTeX or external process is required.
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug)]
enum Value {
    Float(f64),
    Uint(u64),
    Text(String),
    Array(Vec<Value>),
    Map(Vec<(Value, Value)>),
    Bool(bool),
    Null,
}
impl Value {
    fn array(&self) -> &[Value] {
        if let Self::Array(v) = self { v } else { &[] }
    }
    fn at(&self, index: usize) -> &Value {
        self.array().get(index).unwrap_or(&Self::Null)
    }
    fn key(&self, key: u64) -> &Value {
        if let Self::Map(m) = self {
            m.iter()
                .find(|(k, _)| matches!(k,Self::Uint(n) if *n==key))
                .map(|(_, v)| v)
                .unwrap_or(&Self::Null)
        } else {
            &Self::Null
        }
    }
}
fn decode(data: &[u8], offset: &mut usize, depth: usize) -> Result<Value, String> {
    if depth > 32 {
        return Err("CBOR nesting exceeds limit".into());
    }
    let head = *data.get(*offset).ok_or("Truncated CBOR")?;
    *offset += 1;
    if head == 0xfb {
        let bytes: [u8; 8] = data
            .get(*offset..*offset + 8)
            .ok_or("Truncated CBOR float")?
            .try_into()
            .map_err(|_| "Invalid float")?;
        *offset += 8;
        return Ok(Value::Float(f64::from_be_bytes(bytes)));
    }
    if head == 0xf6 {
        return Ok(Value::Null);
    }
    if head == 0xf4 || head == 0xf5 {
        return Ok(Value::Bool(head == 0xf5));
    }
    let n = match head & 31 {
        n @ 0..=23 => u64::from(n),
        24..=27 => {
            let count = 1_usize << ((head & 31) - 24);
            let mut n = 0;
            for _ in 0..count {
                n = (n << 8) | u64::from(*data.get(*offset).ok_or("Truncated CBOR length")?);
                *offset += 1;
            }
            n
        }
        _ => return Err("Unsupported CBOR encoding".into()),
    };
    let len = usize::try_from(n).map_err(|_| "CBOR length overflow")?;
    match head >> 5 {
        0 => Ok(Value::Uint(n)),
        2 | 3 => {
            let end = offset.checked_add(len).ok_or("CBOR length overflow")?;
            let bytes = data.get(*offset..end).ok_or("Truncated CBOR string")?;
            *offset = end;
            Ok(if head >> 5 == 3 {
                Value::Text(String::from_utf8(bytes.to_vec()).map_err(|_| "Invalid CBOR UTF-8")?)
            } else {
                Value::Text(bytes.iter().map(|b| format!("{b:02X}")).collect())
            })
        }
        4 | 5 => {
            if len > data.len().saturating_sub(*offset) {
                return Err("Invalid CBOR collection size".into());
            }
            if head >> 5 == 4 {
                let mut values = Vec::new();
                for _ in 0..len {
                    values.push(decode(data, offset, depth + 1)?);
                }
                Ok(Value::Array(values))
            } else {
                let mut values = Vec::new();
                for _ in 0..len {
                    values.push((
                        decode(data, offset, depth + 1)?,
                        decode(data, offset, depth + 1)?,
                    ));
                }
                Ok(Value::Map(values))
            }
        }
        _ => Err("Unsupported CBOR type".into()),
    }
}
fn xml(text: &str) -> String {
    text.chars()
        .filter(|&c| c == '\n' || c == '\t' || c == '\r' || c >= ' ')
        .map(|c| match c {
            '&' => "&amp;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '"' => "&quot;".into(),
            '\'' => "&apos;".into(),
            _ => c.to_string(),
        })
        .collect()
}
struct Sheet {
    name: String,
    rows: Vec<Vec<Value>>,
}
impl Sheet {
    fn new(name: &str, headings: &[&str]) -> Self {
        Self {
            name: name.into(),
            rows: vec![headings.iter().map(|s| Value::Text((*s).into())).collect()],
        }
    }
    fn row(&mut self, values: Vec<Value>) {
        self.rows.push(values);
    }
}
fn column(mut n: usize) -> String {
    let mut result = String::new();
    n += 1;
    while n > 0 {
        n -= 1;
        result.insert(0, (b'A' + (n % 26) as u8) as char);
        n /= 26;
    }
    result
}
fn column_width(sheet: &Sheet, i: usize, width: usize) -> usize {
    if i == 0 {
        48
    } else if i == width - 1
        && (sheet.name.starts_with("GOP") || sheet.name.starts_with("Overview"))
    {
        90
    } else if (sheet.name.starts_with("Compliance") && i >= 5)
        || (sheet.name.starts_with("Events") && i == 5)
    {
        60
    } else {
        24
    }
}
fn worksheet(sheet: &Sheet) -> String {
    let width = sheet.rows.first().map_or(1, Vec::len);
    let end = format!("{}{}", column(width - 1), sheet.rows.len());
    let mut out = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><dimension ref="A1:{end}"/><sheetViews><sheetView workbookViewId="0"><pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/></sheetView></sheetViews><cols>"#
    );
    for i in 0..width {
        let w = column_width(sheet, i, width);
        let _ = write!(
            out,
            r#"<col min="{}" max="{}" width="{w}" customWidth="1"/>"#,
            i + 1,
            i + 1
        );
    }
    out.push_str(r#"</cols><sheetData>"#);
    for (r, row) in sheet.rows.iter().enumerate() {
        let _ = write!(
            out,
            r#"<row r="{}" ht="{}" customHeight="1">"#,
            r + 1,
            if r == 0 {
                42
            } else {
                22 * row
                    .iter()
                    .enumerate()
                    .map(|(c, v)| match v {
                        Value::Text(t) => t
                            .chars()
                            .count()
                            .div_ceil(column_width(sheet, c, width).saturating_sub(3))
                            .clamp(1, 6),
                        _ => 1,
                    })
                    .max()
                    .unwrap_or(1)
            }
        );
        for (c, value) in row.iter().enumerate() {
            let pos = format!("{}{}", column(c), r + 1);
            let style = if r == 0 { 1 } else { 4 };
            match value {
                Value::Uint(n) if *n <= 9_007_199_254_740_991 => {
                    let _ = write!(out, r#"<c r="{pos}" s="2"><v>{n}</v></c>"#);
                }
                Value::Float(n) if n.is_finite() => {
                    let _ = write!(out, r#"<c r="{pos}" s="3"><v>{n}</v></c>"#);
                }
                Value::Null => {}
                _ => {
                    let text = match value {
                        Value::Text(s) => s.clone(),
                        Value::Uint(n) => n.to_string(),
                        Value::Bool(v) => if *v { "Yes" } else { "No" }.into(),
                        _ => String::new(),
                    };
                    let _ = write!(
                        out,
                        r#"<c r="{pos}" s="{style}" t="inlineStr"><is><t xml:space="preserve">{}</t></is></c>"#,
                        xml(&text)
                    );
                }
            }
        }
        out.push_str(r#"</row>"#);
    }
    let _ = write!(
        out,
        r#"</sheetData><autoFilter ref="A1:{end}"/></worksheet>"#
    );
    out
}

pub fn export(path: &Path, data: &[u8]) -> Result<(), String> {
    let mut offset = 0;
    let root = decode(data, &mut offset, 0)?;
    if offset != data.len() || !matches!(root.key(0), Value::Uint(4 | 5)) {
        return Err("Invalid or unsupported analysis CBOR schema".into());
    }
    let mut sheets = vec![
        Sheet::new(
            "Overview",
            &[
                "Source file",
                "Format",
                "Standard",
                "Packets",
                "Malformed",
                "Trailing bytes",
                "Null packets",
                "Valid sections",
                "CRC errors",
                "Source bytes",
                "Source path",
            ],
        ),
        Sheet::new(
            "PIDs",
            &[
                "Source file",
                "PID (decimal)",
                "Packets",
                "Payload packets",
                "Transport errors",
                "Continuity errors",
                "Duplicates",
                "Scrambled",
                "PCR samples",
                "Max PCR gap (27 MHz ticks)",
            ],
        ),
        Sheet::new(
            "Programs",
            &[
                "Source file",
                "Program",
                "PMT PID",
                "PCR PID",
                "ES PID",
                "Stream type (decimal)",
            ],
        ),
        Sheet::new(
            "PSI SI",
            &[
                "Source file",
                "PID",
                "Table ID",
                "Sections",
                "CRC errors",
                "Version",
                "Section number",
                "Last section",
                "First section (hex)",
            ],
        ),
        Sheet::new(
            "Clocks",
            &[
                "Source file",
                "Packet (zero based)",
                "PID",
                "Clock",
                "Ticks (PCR 27 MHz; PTS DTS 90 kHz)",
            ],
        ),
        Sheet::new(
            "GOP",
            &[
                "Source file",
                "Video PID",
                "Codec",
                "GOP index",
                "Start packet",
                "Start PTS (90 kHz)",
                "Length (pictures)",
                "Compressed VCL bytes",
                "Complete",
                "Structure (decode order)",
                "Next GOP start packet",
                "Program TS bytes (both boundaries included)",
                "Program number",
                "Program PIDs (decimal)",
            ],
        ),
        Sheet::new(
            "Bitrate samples",
            &[
                "Source file",
                "First packet",
                "Window packets",
                "PID",
                "PID packets",
                "Transport rate (Mb/s; PCR derived)",
                "PID rate (Mb/s)",
            ],
        ),
        Sheet::new(
            "Compliance",
            &[
                "Source file",
                "Group",
                "Indicator",
                "Status",
                "Observed",
                "Note",
                "Reference",
            ],
        ),
        Sheet::new(
            "Events",
            &[
                "Source file",
                "Packet",
                "Byte offset",
                "PID",
                "Indicator",
                "Detail",
                "Exact packet",
            ],
        ),
    ];
    for (file_index, file) in root.key(2).array().iter().enumerate() {
        let full_source = file.key(0).clone();
        let source = if let Value::Text(path) = &full_source {
            Value::Text(format!(
                "{}: {}",
                file_index + 1,
                path.rsplit(['/', '\\']).next().unwrap_or(path)
            ))
        } else {
            full_source.clone()
        };
        let mut row = vec![source.clone(), file.key(1).clone(), file.key(2).clone()];
        row.extend_from_slice(file.key(3).array());
        row.push(file.key(11).clone());
        row.push(full_source);
        sheets[0].row(row);
        for (key, index) in [(4, 1), (6, 3)] {
            for item in file.key(key).array() {
                let mut row = vec![source.clone()];
                row.extend_from_slice(item.array());
                sheets[index].row(row);
            }
        }
        for program in file.key(5).array() {
            for stream in program.at(3).array() {
                sheets[2].row(vec![
                    source.clone(),
                    program.at(0).clone(),
                    program.at(1).clone(),
                    program.at(2).clone(),
                    stream.at(0).clone(),
                    stream.at(1).clone(),
                ]);
            }
        }
        for clock in file.key(7).array() {
            let label = match clock.at(2) {
                Value::Uint(0) => "PCR",
                Value::Uint(1) => "PTS",
                _ => "DTS",
            };
            sheets[4].row(vec![
                source.clone(),
                clock.at(0).clone(),
                clock.at(1).clone(),
                Value::Text(label.into()),
                clock.at(3).clone(),
            ]);
        }
        for video in file.key(13).array() {
            for (i, gop) in video.at(3).array().iter().enumerate() {
                let mut row = vec![
                    source.clone(),
                    video.at(0).clone(),
                    Value::Text(
                        if matches!(video.at(1), Value::Uint(0x24)) {
                            "H.265"
                        } else {
                            "H.264"
                        }
                        .into(),
                    ),
                    Value::Uint(i as u64),
                ];
                row.extend((0..8).map(|i| gop.at(i).clone()));
                row.push(video.at(4).clone());
                row.push(Value::Text(
                    video
                        .at(5)
                        .array()
                        .iter()
                        .filter_map(|pid| {
                            if let Value::Uint(pid) = pid {
                                Some(pid.to_string())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", "),
                ));
                sheets[5].row(row);
            }
        }
        for window in file.key(14).array() {
            if let Value::Map(pids) = window.at(2) {
                for (pid, count) in pids {
                    sheets[6].row(vec![
                        source.clone(),
                        window.at(0).clone(),
                        window.at(1).clone(),
                        pid.clone(),
                        count.clone(),
                        window.at(3).clone(),
                        match (window.at(3), window.at(1), count) {
                            (Value::Float(rate), Value::Uint(total), Value::Uint(count))
                                if *total > 0 =>
                            {
                                Value::Float(rate * *count as f64 / *total as f64)
                            }
                            _ => Value::Null,
                        },
                    ]);
                }
            }
        }
        for (key, index) in [(0, 7), (1, 8)] {
            for item in file.key(12).key(key).array() {
                let mut row = vec![source.clone()];
                row.extend_from_slice(item.array());
                sheets[index].row(row);
            }
        }
    }
    // Respect Excel's row limit without silently discarding analysis records.
    let mut split = Vec::new();
    for sheet in sheets {
        for (part, rows) in sheet.rows[1..].chunks(1_048_575).enumerate() {
            let mut out = Sheet {
                name: if part == 0 {
                    sheet.name.clone()
                } else {
                    format!("{} {}", sheet.name, part + 1)
                },
                rows: vec![sheet.rows[0].clone()],
            };
            out.rows.extend_from_slice(rows);
            split.push(out);
        }
        if sheet.rows.len() == 1 {
            split.push(sheet);
        }
    }
    for sheet in &split {
        for row in &sheet.rows {
            for cell in row {
                if let Value::Text(text) = cell
                    && text.encode_utf16().count() > 32_767
                {
                    return Err(format!(
                        "{} contains text exceeding the Excel cell limit; use CBOR to preserve it.",
                        sheet.name
                    ));
                }
            }
        }
    }
    let mut zip = Zip::default();
    zip.add("_rels/.rels",r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#)?;
    let mut types = String::from(
        r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>"#,
    );
    let mut book = String::from(
        r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>"#,
    );
    let mut rels = String::from(
        r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    for (i, sheet) in split.iter().enumerate() {
        let n = i + 1;
        let _ = write!(
            types,
            r#"<Override PartName="/xl/worksheets/sheet{n}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#
        );
        let _ = write!(
            book,
            r#"<sheet name="{}" sheetId="{n}" r:id="rId{n}"/>"#,
            xml(&sheet.name)
        );
        let _ = write!(
            rels,
            r#"<Relationship Id="rId{n}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{n}.xml"/>"#
        );
        zip.add(&format!("xl/worksheets/sheet{n}.xml"), &worksheet(sheet))?;
    }
    let _ = write!(
        rels,
        r#"<Relationship Id="rId{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#,
        split.len() + 1
    );
    types.push_str("</Types>");
    book.push_str("</sheets></workbook>");
    zip.add("[Content_Types].xml", &types)?;
    zip.add("xl/workbook.xml", &book)?;
    zip.add("xl/_rels/workbook.xml.rels", &rels)?;
    zip.add("xl/styles.xml",r#"<?xml version="1.0"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts count="1"><numFmt numFmtId="164" formatCode="0.000"/></numFmts><fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><color rgb="FFFFFFFF"/><sz val="11"/><name val="Calibri"/></font></fonts><fills count="3"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FF204E70"/><bgColor indexed="64"/></patternFill></fill></fills><borders count="1"><border/></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="5"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="1" fillId="2" borderId="0" xfId="0" applyAlignment="1"><alignment wrapText="1" vertical="center"/></xf><xf numFmtId="3" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1" applyAlignment="1"><alignment vertical="top"/></xf><xf numFmtId="164" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0" applyAlignment="1"><alignment wrapText="1" vertical="top"/></xf></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#)?;
    fs::write(path, zip.finish()?).map_err(|e| e.to_string())
}

#[derive(Default)]
struct Zip {
    data: Vec<u8>,
    directory: Vec<u8>,
    count: u16,
}
fn u16le(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn u32le(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for b in data {
        crc ^= u32::from(*b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320_u32.wrapping_mul(crc & 1));
        }
    }
    !crc
}
impl Zip {
    fn add(&mut self, name: &str, text: &str) -> Result<(), String> {
        let size = u32::try_from(text.len()).map_err(|_| "XLSX entry exceeds 4 GiB")?;
        let offset = u32::try_from(self.data.len()).map_err(|_| "XLSX exceeds 4 GiB")?;
        let n = u16::try_from(name.len()).map_err(|_| "ZIP name too long")?;
        let crc = crc32(text.as_bytes());
        u32le(&mut self.data, 0x04034b50);
        for v in [20, 0, 0, 0, 33] {
            u16le(&mut self.data, v);
        }
        for v in [crc, size, size] {
            u32le(&mut self.data, v);
        }
        u16le(&mut self.data, n);
        u16le(&mut self.data, 0);
        self.data.extend_from_slice(name.as_bytes());
        self.data.extend_from_slice(text.as_bytes());
        u32le(&mut self.directory, 0x02014b50);
        for v in [20, 20, 0, 0, 0, 33] {
            u16le(&mut self.directory, v);
        }
        for v in [crc, size, size] {
            u32le(&mut self.directory, v);
        }
        for v in [n, 0, 0, 0, 0] {
            u16le(&mut self.directory, v);
        }
        u32le(&mut self.directory, 0);
        u32le(&mut self.directory, offset);
        self.directory.extend_from_slice(name.as_bytes());
        self.count = self.count.checked_add(1).ok_or("Too many XLSX entries")?;
        Ok(())
    }
    fn finish(mut self) -> Result<Vec<u8>, String> {
        let offset = u32::try_from(self.data.len()).map_err(|_| "XLSX exceeds 4 GiB")?;
        let size = u32::try_from(self.directory.len()).map_err(|_| "ZIP directory too large")?;
        self.data.extend_from_slice(&self.directory);
        u32le(&mut self.data, 0x06054b50);
        for v in [0, 0, self.count, self.count] {
            u16le(&mut self.data, v);
        }
        u32le(&mut self.data, size);
        u32le(&mut self.data, offset);
        u16le(&mut self.data, 0);
        Ok(self.data)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_truncated_cbor() {
        assert!(decode(&[0x83, 1], &mut 0, 0).is_err());
    }
    #[test]
    fn standard_crc() {
        assert_eq!(crc32(b"123456789"), 0xcbf43926);
    }
    #[test]
    fn strings_cannot_become_formulas() {
        let mut s = Sheet::new("test", &["Value"]);
        s.row(vec![Value::Text(r#"=HYPERLINK("x") & <y>"#.into())]);
        let x = worksheet(&s);
        assert!(x.contains("inlineStr"));
        assert!(!x.contains(r#"<f>"#));
        assert!(x.contains("&amp;"));
    }
    #[test]
    #[ignore = "regenerates XLSX from CBOR; set TSAN_REPORT_OUTPUT"]
    fn export_existing_cbor_snapshots() -> Result<(), Box<dyn std::error::Error>> {
        let directory =
            std::env::var_os("TSAN_REPORT_OUTPUT").ok_or("Missing TSAN_REPORT_OUTPUT")?;
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "cbor") {
                export(&path.with_extension("xlsx"), &fs::read(&path)?)?;
                println!("CBOR -> XLSX: {}", path.display());
            }
        }
        Ok(())
    }
}
