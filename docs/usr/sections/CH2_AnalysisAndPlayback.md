# CH2 — Analysis and Playback

## Windows

### Import and compare

  1. Choose Import Transport Stream → Import Files and select one or more local files.
  2. Wait for each background analysis to complete. Pending files show their queue or progress state.
  3. Check files in the left sidebar to compare their Analyzer pages. The displayed comparison limit adapts to the window width.
  4. Click a filename to make it the single active file, shown with a blue highlight. Player follows this selection.
  5. Use the close control to remove a document; closing a file with analysis in progress cancels that work.

  - The reader detects 188-byte TS, 192-byte prefixed TS, and 204-byte TS with a trailing code area. Packet numbers are zero based.
  - Each file keeps its own analysis results, filters, selected PID/GOP, and graph view during the current session.
  - Analysis does not modify source recordings or create an on-disk analysis cache. Keep source files available: Packets reads them on demand.
  - Previously imported paths are available from Import Recent Files. Clearing that list is described in [Chapter 3](CH3_ReportsAndSettings.md).

### Analyzer pages

| Page | What it shows |
| --- | --- |
| Overview | Detected transport format, media information, codecs, and service information. Unknown values indicate missing or unavailable evidence. |
| PSI / SI Tree | Observed table sections grouped by table/PID, with versions, occurrences, section details, and CRC information. Filter by table, PID, or table ID. |
| Packets | Packet number, file offset, PID, PUSI, continuity counter, PCR, flags, parsed fields, and raw bytes. Filter by PID or navigate to a packet. |
| TR 101 290 | Applicable checks and events. Unmeasured, inapplicable, and unimplemented checks are distinct from a passing result. |
| Graphs | Bitrate, PCR/PTS/DTS, and GOP views for the selected files. |

  - PSI/SI labels come from the stream's signalling. A QAM recording can contain ATSC PSIP tables such as MGT and cable VCT; these names do not identify the recording's RF modulation.
  - Ordinary TS files do not provide direct MER, BER, SNR, or RF level measurements.
  - In event tables, exact packet positions link to the packet detail. A leading ~ indicates an estimated analysis position for an absence or duration event.
  - Drag the Packets divider to resize its list and detail panes. Wide tables can be scrolled horizontally.

### Bitrate and clocks

  - Bitrate samples use packet counts and PCR-derived timing. The view reports its sampling interval and timing source.
  - The transport-total series and enabled PID contributions allow comparison of video, audio, signalling, and null packets. Use the series checkboxes and PID filter to choose visible data.
  - Statistics include packet count and minimum, maximum, and average bitrate. The X axis can use packet number or elapsed stream time.
  - PCR/PTS/DTS views expose observed clock values and timing relationships. Their timing differs from the Player's video-PTS seek index.
  - Left drag selects an X/Y zoom rectangle; right drag pans. The wheel or vertical middle-button drag zooms around the cursor.
  - Reset view restores the full data extent. Reset Filter restores filtering where the view offers filters. Legend controls the visible legend.
  - Zoom is bounded by the full data extent and a minimum of eight sample intervals. Automatic Y bounds follow the data; manual zoom retains the selected range.

### GOP analysis

  - Select a video PID. H.264 and H.265 analysis provides a picture count, complete-GOP count, length statistics, and the GOP list.
  - An intra picture starts a new group: I/IDR for AVC, or intra/IDR/CRA/BLA for HEVC as parsed from the stream. The structure is shown in decode order.
  - I means intra, P means predicted, and B means bidirectional. A question mark means the type could not be determined from the available headers.
  - GOP Length counts coded pictures, not TS packets. Multiple slices belonging to one picture do not create additional pictures.
  - The list retains partial or damaged groups. Statistics and graphs use complete groups only; this is a boundary/header completeness check, not proof that all pictures decode correctly.
  - Select a row to inspect the picture sequence. Drag a column header edge for column width or the list's bottom handle for list height. The table uses alternating row backgrounds.
  - GOP Length and GOP Bytes each plot one series, so they have Reset view without Reset Filter.

| GOP Bytes mode | Meaning | X position |
| --- | --- | --- |
| Program TS | Full input packet bytes for PAT, the selected program's PMT/PCR, and all its elementary streams, including headers, stuffing, and audio. Null and other-program packets are excluded. | The next GOP's first TS packet. |
| Compressed VCL | Compressed picture slice data and NAL headers. VCL means Video Coding Layer; start codes, TS/PES headers, and non-picture NAL units are excluded. | The current GOP's first TS packet. |

  - Program TS needs a unique program association. If that cannot be established, use the available VCL measurement instead of assuming a program.
  - Program TS uses the detected 188/192/204-byte input packet width and includes both GOP boundary packets. Adjacent values therefore must not be summed as disjoint intervals.
  - The Y axis uses bytes for GOP Bytes and pictures for GOP Length. The X axis uses zero-based TS packet numbers, rather than a GOP sequence index.
  - Automatic GOP Y scaling adds 10% of the observed range above and below the data. A constant series gets a minimum display range; there is no sample-specific multiplier.

### Player

  1. Select a filename in the sidebar and open Player → Play TS.
  2. Use play/pause, stop, and the timeline to control playback. The displayed input path identifies the loaded file.
  3. Click another filename to switch the Player input, even when several files are checked for analysis comparison.
  4. To change the video backend or GPU adapter, use Player settings. The choice applies to the next file loaded.

  - Only one file is active in Player. A switch preserves whether playback was running or paused.
  - Windows playback supports the packaged D3D11/D3D12 paths; actual codec/profile support depends on the selected GPU and driver.
  - Seeking uses video PES timestamps and available entry points. Damaged timestamps, missing parameters, and unusual multi-program streams may limit reliable seeking.
  - Player status lists the selected backend, adapter/decoder, warnings, and QoS events for diagnosis.
  - IP Streaming and recording are reserved for future implementation.

## macOS

  - The native GUI and playback backend are not implemented. The Windows operation steps above do not imply macOS support.

## Linux

  - The native GUI and playback backend are not implemented. Wayland/X11 integration and hardware-decoder support remain development work.
