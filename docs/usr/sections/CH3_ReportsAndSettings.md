# CH3 — Reports and Settings

## Windows

### Export reports

  1. Import files and wait for their analyses.
  2. Open Export Analyzed Report. Check the files to include in that menu; verify the list before exporting.
  3. Select a format, then choose the filename and destination in the save dialog.
  4. Wait for export to finish. Consult Log if it fails.

| Format | Contents | Runtime requirement |
| --- | --- | --- |
| CBOR | Structured analysis snapshot, preserving detailed values for further processing. | Application only. |
| XLSX | Overview, PIDs, Programs, PSI SI, Clocks, GOP, Bitrate samples, Compliance, and Events sheets. | Excel and LaTeX are not required to generate it. |
| LaTeX | Main .tex document, chapter files, and chart images. | No compiler is run during source export. |
| PDF | Compiled report, retaining its LaTeX source, chart images, and compilation diagnostics. | Uses the package's private XeLaTeX subset. |

  - Numbers remain numeric in XLSX. Large tables split across sheets when needed; unsupported cell/ZIP limits report an error rather than silently truncating data.
  - Each export produces the selected format. Exporting a PDF does not also create CBOR and XLSX files.
  - For a PDF destination named report.pdf, the adjacent files are:

~~~text
report.pdf
report.tex
report_sections/
    Overview.tex
    PSI_SI_Tree.tex
    TR_101_290.tex
    Graphs.tex
    Bitrate.tex
    Timestamps.tex
    GOP.tex
    images/
        file_0_bitrate.svg
        file_0_bitrate.pdf
        file_0_gop_102_length.svg
        file_0_gop_102_length.pdf
        ...
    build/
        report.pdf
        report.log
        report.fls
        report.aux
        ...
~~~

  - The companion directory is derived from the chosen filename stem plus _sections. Characters other than ASCII letters, digits, hyphens, and underscores become underscores in that directory name.
  - Chart names identify the input's index and chart type; GOP names include the PID in decimal.
  - LaTeX-only export omits PDF compilation and its build directory. Keep the main .tex file with its companion directory when moving or editing the source.
  - PDF failures retain build diagnostics beside the chosen destination. No multi-hour TeX download is required by the packaged application.
  - The included TeX subset supports the application's generated reports. Editing arbitrary TeX documents may require your own TeX environment.
  - Use separate destinations for reports that need the same filename stem; generated files can share the same companion-directory name.

### Themes and Liquid Glass

  - Theme provides System, Light, Dark, Transparent, and Liquid Glass (experimental). Opacity and glass controls open in submenus.
  - Liquid Glass uses the desktop behind the window. Its controls include refraction, frost, RGB dispersion, magnification, lens depth, dark tint, and squircle shape.
  - Live glass excludes the main application window from capture tools that honor Windows capture exclusion. To take an ordinary screenshot, select Freeze background / screenshot mode from the glass submenu.
  - Freeze keeps the captured background in memory and stops live capture. Resume live background explicitly to start updating it again.
  - Live glass requires the client area to fit within one monitor. Crossing monitor boundaries, moving partially off screen, or minimizing temporarily stops capture.
  - A capture failure reports an error and falls back to Transparent. Multi-monitor composition and HDR capture are not implemented.

### Saved settings and recent files

  - The file is tsan-config.toml. Its installed and portable locations are listed in [Chapter 1](CH1_InstallationAndUpdates.md).
  - Saved fields include theme, transparent_opacity, player_backend, and recent_files. The recent list is deduplicated and limited to 12 paths.
  - Import Transport Stream → Import Recent Files → Clear Recent Files clears and saves the history immediately. It neither closes open documents nor deletes recordings.
  - Full sessions, open-document order, comparison selections, table widths, PID/GOP selection, graph zoom, and detailed glass controls are not all restored on restart.
  - An invalid configuration is reported instead of being silently overwritten. Correct or back up that file before replacing it.
  - Legacy recent-files.txt is imported only when no TOML configuration exists; the original TXT is retained.

### Logs and troubleshooting

  - Application, analysis, and player events are automatically written to local diagnostic files. The Log page shows the latest 4096 entries and the active diagnostic directory; Dump exports the displayed history to a chosen location.
  - Installed and direct source builds use %LOCALAPPDATA%/TS-Analyzer/diagnostics/. Portable packages use data/diagnostics/ inside the package. The development Run task uses outputs/windows/diagnostics/. Each process creates a session-<timestamp>-<pid>/ directory.
  - application.log contains events; session.txt identifies the build and process. Rust panics add a backtrace to panic.log. On Windows, an independent collector attempts to save crash.dmp and native-crash.txt for an unhandled native exception, and records the exit code in process-exit.txt when the application exits without sending a native exception request.
  - Each application log rotates at 2 MiB with three backups. Startup cleanup aims to retain at most 20 completed sessions and 256 MiB across recognized diagnostic sessions; active sessions are protected, so these are not hard disk quotas.
  - If the preferred directory cannot be written, diagnostics try the platform user directory and then the OS temporary directory. The Log page reports the actual location or an initialization/write error.
  - Collection is local; there is no automatic upload. Before sharing diagnostics, review logs for input paths and consider that a minidump can contain fragments of process memory. A dump is not guaranteed for forced termination, power loss, or failures before application initialization.
  - PDF build logs and updater logs remain separate files.
  - For playback issues, record the input format, selected backend, adapter, decoder, and displayed warning.
  - For PDF issues, retain the main source and the companion build directory.
  - For update issues, retain update.log and, for an installed update, installer.log from the updates directory.
  - Help shows the platform release identity. GitHub Link opens the repository using the operating system's default browser.

## macOS

  - Native report dialogs, application settings integration, and themes have not been released.
  - Shared file logging and Rust panic capture use ~/Library/Logs/TS-Analyzer/. Native crash dump collection is not implemented.

## Linux

  - Native report dialogs, application settings integration, and themes have not been released.
  - Shared file logging and Rust panic capture use $XDG_STATE_HOME/TS-Analyzer/diagnostics/, falling back to ~/.local/state/TS-Analyzer/diagnostics/. Native crash dump collection is not implemented.
