# TS Analyzer
- Desktop MPEG transport stream analysis, playback, and reporting.
- License: [Apache-2.0](LICENSE).

## Features
- Inspect 188-, 192-, and 204-byte transport streams; compare multiple files.
- Browse media information, PID statistics, PSI/SI tables, packets, and compliance events.
- Plot bitrate and PCR/PTS/DTS; inspect H.264/H.265 GOP length, structure, and byte counts.
- Play local streams with hardware decoding and timeline seeking.
- Export CBOR, XLSX, LaTeX, and PDF reports.
- Save themes and recent files; update installed and portable copies in place.

## Windows
- Windows 10 22H2 / Windows 11 x64; installer and portable ZIP.
- Download the Windows installer or portable ZIP from [Releases](https://github.com/LunaticGhoulPiano/TS-Analyzer/releases).
- Live UDP/RTP input, recording, and the CLI are planned.

## macOS
- Native application and packages are not implemented yet.

## Linux
- Native application and packages are not implemented yet.

## Documentation
- Each guide has an outlines.md index and numbered chapters under sections/.

| Guide | Contents |
| --- | --- |
| [User guide](docs/usr/outlines.md) | Installation, updates, analysis, playback, reports, and settings. |
| [Developer guide](docs/dev/outlines.md) | Environment setup, build commands, VS Code tasks, packaging, architecture, GUI, and planned work. |
