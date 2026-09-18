use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Duration;

use eframe::egui;
use tsan_analyzer::{AnalysisReport, BroadcastStandard, analyze_file};
use tsan_input::InputSourceKind;
use tsan_player::platform::windows::{
    GpuAdapterSelection, GstreamerAdapter, PlayerBackendKind, WindowsVideoHost,
};
use tsan_player::{PlayerState, VideoRectangle, VideoSurface};

use crate::analysis_views::{self, ViewState};
use crate::logging::{LogCategory, LogEntry, LogLevel};
use crate::platform::windows::{
    open_transport_stream_dialog, save_analyzed_report_dialog, save_log_dialog,
    save_transport_stream_dialog,
};
use crate::player_worker::{PlayerCommand, PlayerSnapshot, PlayerWorkerHandle};
use crate::report_export::{self, ExportInput, ReportFormat};

const VIDEO_VIEWPORT_ID: &str = "tsan-video-output";
const PLAYER_CONTROLS_HEIGHT: f32 = 78.0;
const PLAYER_DETAILS_RESERVED_HEIGHT: f32 = 132.0;
const PLAYER_ICON_SIZE: f32 = 34.0;
const SYSTEM_TEXT_SIZE: f32 = 16.0;
const MAX_CONCURRENT_ANALYSES: usize = 2;
// 0 is fully transparent and 255 is fully opaque. This controls tint, not blur radius.
const DEFAULT_TRANSPARENT_BACKGROUND_OPACITY: u8 = 195;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Page {
    Analyzer,
    Player,
    Log,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AppTheme {
    System,
    Dark,
    Light,
    Transparent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AnalyzerView {
    Overview,
    PsiSiTree,
    Packets,
    Tr101290,
    Bitrate,
    Timestamps,
    Gop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PlayerView {
    PlayTs,
    IpStreaming,
}

#[derive(Clone, Copy)]
enum NavigationAction {
    Page(Page),
    Analyzer(AnalyzerView),
    Player(PlayerView),
    SelectDocument(usize),
    ExpandDocument(usize),
    PlayDocument(usize),
    CloseDocument(usize),
    ToggleVisibleDocument(usize),
}

struct AnalyzerDocument {
    path: PathBuf,
    receiver: Option<Receiver<Result<AnalysisReport, String>>>,
    report: Option<AnalysisReport>,
    error: Option<String>,
    expanded: bool,
}

impl AppTheme {
    const fn name(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::Transparent => "Transparent",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VideoMode {
    Embedded,
    Detached,
    Fullscreen,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IpStreamingProtocol {
    Udp,
    Rtp,
}

impl IpStreamingProtocol {
    const fn name(self) -> &'static str {
        match self {
            Self::Udp => "UDP",
            Self::Rtp => "RTP",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PlayerUiAction {
    Play,
    Pause,
    Stop,
    Seek(Duration),
    SetVideoMode(VideoMode),
    ToggleFullscreen,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PlayerControlIcon {
    Play,
    Pause,
    Stop,
    Embedded,
    Detached,
    Fullscreen,
}

pub struct TsanApp {
    page: Page,
    analyzer_view: AnalyzerView,
    player_view: PlayerView,
    documents: Vec<AnalyzerDocument>,
    selected_document: Option<usize>,
    visible_documents: Vec<usize>,
    pane_widths: [f32; 3],
    recent_files: Vec<PathBuf>,
    export_selected: Vec<bool>,
    export_receiver: Option<Receiver<Result<PathBuf, String>>>,
    theme: AppTheme,
    transparent_background_opacity: u8,
    source_view: Option<InputSourceKind>,
    ip_streaming_protocol: Option<IpStreamingProtocol>,
    record_max_time: String,
    record_output_path: Option<PathBuf>,
    video_mode: VideoMode,
    fullscreen_return_mode: VideoMode,
    selected_backend: PlayerBackendKind,
    selected_adapter: GpuAdapterSelection,
    snapshot: PlayerSnapshot,
    analyzer_pid_filter: String,
    analyzer_selected_pid: Option<u16>,
    analysis_view_state: ViewState,
    worker: PlayerWorkerHandle,
    embedded_video_host: Option<WindowsVideoHost>,
    embedded_window_rectangle: Option<VideoRectangle>,
    detached_video_host: Option<WindowsVideoHost>,
    attached_surface: Option<(usize, VideoRectangle)>,
    timeline_drag_seconds: Option<f64>,
    embedded_video_height: Option<f32>,
    local_error: Option<String>,
    log_entries: Vec<LogEntry>,
    next_log_sequence: u64,
    last_worker_log_sequence: u64,
}

impl TsanApp {
    pub fn new(context: &eframe::CreationContext<'_>, initial_input: Option<PathBuf>) -> Self {
        let transparent_background_opacity = DEFAULT_TRANSPARENT_BACKGROUND_OPACITY;
        apply_theme(
            &context.egui_ctx,
            AppTheme::Transparent,
            transparent_background_opacity,
        );

        let (embedded_video_host, local_error) = match context.winit_window() {
            Some(window) => match WindowsVideoHost::new(window.clone()) {
                Ok(host) => (Some(host), None),
                Err(error) => (None, Some(error.to_string())),
            },
            None => (None, Some("native root window is unavailable".to_owned())),
        };

        let mut app = Self {
            page: Page::Analyzer,
            analyzer_view: AnalyzerView::Overview,
            player_view: PlayerView::PlayTs,
            documents: Vec::new(),
            selected_document: None,
            visible_documents: Vec::new(),
            pane_widths: [0.5, 0.5, 0.0],
            recent_files: load_recent_files(),
            export_selected: Vec::new(),
            export_receiver: None,
            theme: AppTheme::Transparent,
            transparent_background_opacity,
            source_view: None,
            ip_streaming_protocol: None,
            record_max_time: "00:00:30".to_owned(),
            record_output_path: None,
            video_mode: VideoMode::Embedded,
            fullscreen_return_mode: VideoMode::Embedded,
            selected_backend: PlayerBackendKind::D3d12,
            selected_adapter: GpuAdapterSelection::Default,
            snapshot: PlayerSnapshot::default(),
            analyzer_pid_filter: String::new(),
            analyzer_selected_pid: None,
            analysis_view_state: ViewState::new(),
            worker: PlayerWorkerHandle::spawn(),
            embedded_video_host,
            embedded_window_rectangle: None,
            detached_video_host: None,
            attached_surface: None,
            timeline_drag_seconds: None,
            embedded_video_height: None,
            local_error,
            log_entries: Vec::new(),
            next_log_sequence: 1,
            last_worker_log_sequence: 0,
        };
        app.record_log(LogLevel::Info, LogCategory::System, "TS Analyzer started");
        app.record_log(
            LogLevel::Info,
            LogCategory::Analysis,
            "Analyzer workspace initialized",
        );
        if let Some(error) = app.local_error.clone() {
            app.record_log(LogLevel::Error, LogCategory::System, error);
        }
        if let Some(input) = initial_input {
            app.load_input(input);
        }
        app
    }

    fn selected_document(&self) -> Option<&AnalyzerDocument> {
        self.selected_document
            .and_then(|index| self.documents.get(index))
    }

    fn start_analysis(&mut self, input: &Path) {
        if let Some(index) = self
            .documents
            .iter()
            .position(|document| document.path == input)
        {
            self.selected_document = Some(index);
            if !self.visible_documents.contains(&index) {
                self.visible_documents.clear();
                self.visible_documents.push(index);
            }
            return;
        }

        self.export_selected.push(true);
        self.documents.push(AnalyzerDocument {
            path: input.to_path_buf(),
            receiver: None,
            report: None,
            error: None,
            expanded: false,
        });
        self.start_pending_analyses();
        self.selected_document = Some(self.documents.len() - 1);
        self.visible_documents.clear();
        self.visible_documents.push(self.documents.len() - 1);
        self.recent_files.retain(|path| path != input);
        self.recent_files.insert(0, input.to_path_buf());
        self.recent_files.truncate(12);
        if let Err(error) = save_recent_files(&self.recent_files) {
            self.record_log(
                LogLevel::Warning,
                LogCategory::Configuration,
                format!("Could not save recent files: {error}"),
            );
        }
        self.analyzer_pid_filter.clear();
        self.analyzer_selected_pid = None;
    }

    fn start_pending_analyses(&mut self) {
        let running = self
            .documents
            .iter()
            .filter(|document| document.receiver.is_some())
            .count();
        let available = MAX_CONCURRENT_ANALYSES.saturating_sub(running);
        let pending = self
            .documents
            .iter()
            .enumerate()
            .filter(|(_, document)| {
                document.receiver.is_none() && document.report.is_none() && document.error.is_none()
            })
            .take(available)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        for index in pending {
            let (sender, receiver) = mpsc::channel();
            let path = self.documents[index].path.clone();
            match thread::Builder::new()
                .name("tsan-analyzer-worker".to_owned())
                .spawn(move || {
                    let result = analyze_file(&path).map_err(|error| error.to_string());
                    let _ = sender.send(result);
                }) {
                Ok(_worker) => self.documents[index].receiver = Some(receiver),
                Err(error) => {
                    self.documents[index].error =
                        Some(format!("failed to start analyzer: {error}"));
                }
            }
        }
    }

    fn receive_analysis(&mut self) {
        let mut logs = Vec::new();
        for document in &mut self.documents {
            let outcome = document.receiver.as_ref().map(Receiver::try_recv);
            match outcome {
                Some(Ok(Ok(report))) => {
                    logs.push((
                        LogLevel::Info,
                        format!(
                            "Analyzed {}: {} TS packets across {} PIDs",
                            document.path.display(),
                            report.packets,
                            report.pids.len()
                        ),
                    ));
                    document.report = Some(report);
                    document.receiver = None;
                }
                Some(Ok(Err(error))) => {
                    logs.push((LogLevel::Error, error.clone()));
                    document.error = Some(error);
                    document.receiver = None;
                }
                Some(Err(TryRecvError::Disconnected)) => {
                    document.error = Some("analyzer worker stopped unexpectedly".to_owned());
                    document.receiver = None;
                }
                Some(Err(TryRecvError::Empty)) | None => {}
            }
        }
        for (level, message) in logs {
            self.record_log(level, LogCategory::Analysis, message);
        }
        self.start_pending_analyses();
    }

    fn receive_snapshots(&mut self) {
        self.receive_analysis();
        self.receive_export();
        if let Some(snapshot) = self.worker.latest_snapshot() {
            let last_worker_log_sequence = self.last_worker_log_sequence;
            for entry in snapshot
                .events
                .iter()
                .filter(|entry| entry.sequence > last_worker_log_sequence)
            {
                self.last_worker_log_sequence = self.last_worker_log_sequence.max(entry.sequence);
                self.log_entries.push(entry.clone());
            }
            if self.source_view != Some(InputSourceKind::IpStreaming) {
                self.source_view = snapshot.source_kind;
            }
            self.snapshot = snapshot;
            self.ensure_selected_adapter();
        }
    }

    fn record_log(&mut self, level: LogLevel, category: LogCategory, message: impl Into<String>) {
        self.log_entries.push(LogEntry::new(
            self.next_log_sequence,
            level,
            category,
            message,
        ));
        self.next_log_sequence = self.next_log_sequence.saturating_add(1);
    }

    fn set_local_error(&mut self, category: LogCategory, message: impl Into<String>) {
        let message = message.into();
        self.local_error = Some(message.clone());
        self.record_log(LogLevel::Error, category, message);
    }

    fn ensure_selected_adapter(&mut self) {
        let adapters = self.current_adapters();
        if adapters
            .iter()
            .any(|adapter| adapter.selection() == self.selected_adapter)
        {
            return;
        }
        if let Some(adapter) = adapters.first() {
            self.selected_adapter = adapter.selection();
        }
    }

    fn current_adapters(&self) -> &[GstreamerAdapter] {
        match self.selected_backend {
            PlayerBackendKind::D3d12 => &self.snapshot.d3d12_adapters,
            PlayerBackendKind::D3d11 => &self.snapshot.d3d11_adapters,
        }
    }

    fn send(&mut self, command: PlayerCommand) {
        if let Err(error) = self.worker.send(command) {
            self.set_local_error(LogCategory::System, error);
        }
    }

    fn load_input(&mut self, input: PathBuf) {
        self.start_analysis(&input);
        self.select_page(Page::Analyzer);
    }

    fn load_player_input(&mut self, input: PathBuf) {
        self.local_error = None;
        self.source_view = None;
        self.ip_streaming_protocol = None;
        self.player_view = PlayerView::PlayTs;
        self.set_video_mode(VideoMode::Embedded);
        self.send(PlayerCommand::Load {
            input,
            kind: self.selected_backend,
            adapter: self.selected_adapter,
        });
        self.page = Page::Player;
    }

    fn select_ip_streaming(&mut self, protocol: IpStreamingProtocol) {
        self.send(PlayerCommand::ResetInput);
        self.clear_surface();
        self.set_video_mode(VideoMode::Embedded);
        self.hide_embedded_video();
        self.source_view = Some(InputSourceKind::IpStreaming);
        self.ip_streaming_protocol = Some(protocol);
        self.player_view = PlayerView::IpStreaming;
        self.local_error = None;
        self.page = Page::Player;
        self.record_log(
            LogLevel::Info,
            LogCategory::Input,
            format!("Selected {} IP Streaming input", protocol.name()),
        );
    }

    fn open_transport_stream(&mut self, frame: &eframe::Frame) {
        let Some(window) = frame.winit_window() else {
            self.set_local_error(LogCategory::System, "native root window is unavailable");
            return;
        };

        match open_transport_stream_dialog(window) {
            Ok(Some(inputs)) => {
                for input in inputs {
                    self.load_input(input);
                }
            }
            Ok(None) => {}
            Err(error) => self.set_local_error(LogCategory::Input, error),
        }
    }

    fn export_dialog(&mut self, frame: &eframe::Frame, format: ReportFormat) {
        let mut inputs = Vec::new();
        for (index, document) in self.documents.iter().enumerate() {
            if !self.export_selected[index] {
                continue;
            }
            let Some(report) = document.report.clone() else {
                self.set_local_error(
                    LogCategory::ImportExport,
                    format!(
                        "Wait until {} finishes analysis before exporting.",
                        document.path.display()
                    ),
                );
                return;
            };
            inputs.push(ExportInput {
                path: document.path.clone(),
                report,
            });
        }
        if inputs.is_empty() {
            self.set_local_error(
                LogCategory::ImportExport,
                "Select at least one TS file to export.",
            );
            return;
        }
        let Some(window) = frame.winit_window() else {
            self.set_local_error(LogCategory::System, "native root window is unavailable");
            return;
        };
        let destination = match save_analyzed_report_dialog(window, format) {
            Ok(Some(path)) => path,
            Ok(None) => return,
            Err(error) => {
                self.set_local_error(LogCategory::ImportExport, error);
                return;
            }
        };
        let (sender, receiver) = mpsc::channel();
        let task_path = destination.clone();
        match thread::Builder::new()
            .name("tsan-report-export".to_owned())
            .spawn(move || {
                let result = report_export::export(format, &task_path, &inputs).map(|()| task_path);
                let _ = sender.send(result);
            }) {
            Ok(_) => {
                self.export_receiver = Some(receiver);
                self.record_log(
                    LogLevel::Info,
                    LogCategory::ImportExport,
                    format!("Exporting {} to {}", format.label(), destination.display()),
                );
            }
            Err(error) => self.set_local_error(
                LogCategory::ImportExport,
                format!("Could not start report export: {error}"),
            ),
        }
    }

    fn receive_export(&mut self) {
        let outcome = self.export_receiver.as_ref().map(Receiver::try_recv);
        match outcome {
            Some(Ok(Ok(path))) => {
                self.export_receiver = None;
                self.record_log(
                    LogLevel::Info,
                    LogCategory::ImportExport,
                    format!("Exported analyzed report to {}", path.display()),
                );
            }
            Some(Ok(Err(error))) => {
                self.export_receiver = None;
                self.set_local_error(LogCategory::ImportExport, error);
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.export_receiver = None;
                self.set_local_error(
                    LogCategory::ImportExport,
                    "Report export worker stopped unexpectedly.",
                );
            }
            Some(Err(TryRecvError::Empty)) | None => {}
        }
    }

    fn select_record_output(&mut self, frame: &eframe::Frame) {
        let Some(window) = frame.winit_window() else {
            self.set_local_error(LogCategory::System, "native root window is unavailable");
            return;
        };

        match save_transport_stream_dialog(window) {
            Ok(Some(path)) => {
                self.record_log(
                    LogLevel::Info,
                    LogCategory::Configuration,
                    format!("Recording output changed to {}", path.display()),
                );
                self.record_output_path = Some(path);
            }
            Ok(None) => {}
            Err(error) => self.set_local_error(LogCategory::Configuration, error),
        }
    }

    fn select_page(&mut self, page: Page) {
        if page != Page::Player && self.video_mode == VideoMode::Embedded {
            self.hide_embedded_video();
        }
        self.page = page;
    }

    fn set_video_mode(&mut self, mode: VideoMode) {
        if self.video_mode == mode {
            if mode == VideoMode::Embedded {
                self.page = Page::Player;
            }
            return;
        }

        let previous_mode = self.video_mode;
        if previous_mode == VideoMode::Embedded {
            self.hide_embedded_video();
            self.detached_video_host = None;
        }
        if mode == VideoMode::Fullscreen && previous_mode != VideoMode::Fullscreen {
            self.fullscreen_return_mode = previous_mode;
        }
        let detached_host = (mode == VideoMode::Embedded)
            .then(|| self.detached_video_host.take())
            .flatten();

        self.video_mode = mode;
        self.attached_surface = None;
        if mode == VideoMode::Embedded {
            self.page = Page::Player;
            self.restore_embedded_video();
        }
        if let Some(host) = detached_host {
            host.hide();
        }
    }

    fn apply_player_action(&mut self, action: PlayerUiAction) {
        match action {
            PlayerUiAction::Play => self.send(PlayerCommand::Play),
            PlayerUiAction::Pause => self.send(PlayerCommand::Pause),
            PlayerUiAction::Stop => self.send(PlayerCommand::Stop),
            PlayerUiAction::Seek(position) => self.send(PlayerCommand::Seek(position)),
            PlayerUiAction::SetVideoMode(mode) => self.set_video_mode(mode),
            PlayerUiAction::ToggleFullscreen => {
                let mode = if self.video_mode == VideoMode::Fullscreen {
                    self.fullscreen_return_mode
                } else {
                    VideoMode::Fullscreen
                };
                self.set_video_mode(mode);
            }
        }
    }

    fn clear_surface(&mut self) {
        self.hide_embedded_video();
        self.attached_surface = None;
        self.send(PlayerCommand::ClearSurface);
    }

    fn hide_embedded_video(&self) {
        if let Some(host) = &self.embedded_video_host {
            host.hide();
        }
    }

    fn restore_embedded_video(&mut self) {
        let Some(window_rectangle) = self.embedded_window_rectangle else {
            return;
        };
        let update = self.embedded_video_host.as_ref().map(|host| {
            let surface = host.surface();
            host.show(window_rectangle)
                .map(|host_rectangle| (surface, host_rectangle))
        });
        match update {
            Some(Ok((surface, rectangle))) => self.attach_surface(surface, rectangle),
            Some(Err(error)) => {
                self.set_local_error(LogCategory::Pipeline, error.to_string());
            }
            None => {}
        }
    }

    fn attach_surface(&mut self, surface: VideoSurface, rectangle: VideoRectangle) {
        let attachment = (surface.identity(), rectangle);
        if self.attached_surface == Some(attachment) {
            return;
        }

        self.send(PlayerCommand::SetSurface { surface, rectangle });
        self.attached_surface = Some(attachment);
    }

    fn application_header(
        &mut self,
        root_ui: &mut egui::Ui,
        frame: &eframe::Frame,
    ) -> Option<egui::Rect> {
        let mut open_requested = false;
        let mut recent_requested = None;
        let mut export_requested = None;
        let mut opacity_changed = false;
        let mut popup_rectangle = None;
        let previous_theme = self.theme;

        let mut header_frame = egui::Frame::side_top_panel(root_ui.style());
        if self.theme == AppTheme::Transparent {
            header_frame = header_frame
                .fill(root_ui.visuals().window_fill)
                .stroke(root_ui.visuals().window_stroke);
        }
        egui::Panel::top("application-header")
            .frame(header_frame)
            .show(root_ui, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    ui.menu_button(
                        egui::RichText::new(format!("Theme: {}", self.theme.name()))
                            .size(SYSTEM_TEXT_SIZE),
                        |ui| {
                            ui.selectable_value(
                                &mut self.theme,
                                AppTheme::System,
                                egui::RichText::new("System").size(SYSTEM_TEXT_SIZE),
                            );
                            ui.selectable_value(
                                &mut self.theme,
                                AppTheme::Dark,
                                egui::RichText::new("Dark").size(SYSTEM_TEXT_SIZE),
                            );
                            ui.selectable_value(
                                &mut self.theme,
                                AppTheme::Light,
                                egui::RichText::new("Light").size(SYSTEM_TEXT_SIZE),
                            );
                            let transparent_menu = ui.menu_button(
                                egui::RichText::new("Transparent").size(SYSTEM_TEXT_SIZE),
                                |ui| {
                                    ui.set_min_width(240.0);
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new("Opacity").size(SYSTEM_TEXT_SIZE),
                                        );
                                        opacity_changed = ui
                                            .add(
                                                egui::Slider::new(
                                                    &mut self.transparent_background_opacity,
                                                    0..=255,
                                                )
                                                .show_value(true),
                                            )
                                            .changed();
                                    });
                                    include_popup_rectangle(
                                        &mut popup_rectangle,
                                        ui.min_rect().expand(6.0),
                                    );
                                },
                            );
                            if transparent_menu.response.clicked() || opacity_changed {
                                self.theme = AppTheme::Transparent;
                            }
                            include_popup_rectangle(
                                &mut popup_rectangle,
                                ui.min_rect().expand(6.0),
                            );
                        },
                    );
                    ui.menu_button(
                        egui::RichText::new("Import Transport Stream ...").size(SYSTEM_TEXT_SIZE),
                        |ui| {
                            if ui.button("Import File ...").clicked() {
                                open_requested = true;
                                ui.close();
                            }
                            ui.menu_button("Import Recent Files ...", |ui| {
                                if self.recent_files.is_empty() {
                                    ui.weak("No recent files");
                                }
                                for path in &self.recent_files {
                                    if ui.button(path.display().to_string()).clicked() {
                                        recent_requested = Some(path.clone());
                                        ui.close();
                                    }
                                }
                                include_popup_rectangle(
                                    &mut popup_rectangle,
                                    ui.min_rect().expand(6.0),
                                );
                            });
                            include_popup_rectangle(
                                &mut popup_rectangle,
                                ui.min_rect().expand(6.0),
                            );
                        },
                    );
                    ui.menu_button(
                        egui::RichText::new("Export Analyzed Report ...").size(SYSTEM_TEXT_SIZE),
                        |ui| {
                            ui.set_min_width(360.0);
                            ui.strong("TS Queue — checked files are included");
                            if self.documents.is_empty() {
                                ui.weak("Import a transport stream first.");
                            }
                            for (index, document) in self.documents.iter().enumerate() {
                                let name = document
                                    .path
                                    .file_name()
                                    .map(|name| name.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| document.path.display().to_string());
                                ui.checkbox(&mut self.export_selected[index], name)
                                    .on_hover_text(document.path.display().to_string());
                            }
                            ui.separator();
                            for format in
                                [ReportFormat::Cbor, ReportFormat::Latex, ReportFormat::Pdf]
                            {
                                if ui
                                    .add_enabled(
                                        self.export_receiver.is_none()
                                            && self
                                                .export_selected
                                                .iter()
                                                .any(|selected| *selected),
                                        egui::Button::new(format.label()),
                                    )
                                    .clicked()
                                {
                                    export_requested = Some(format);
                                    ui.close();
                                }
                            }
                            if self.export_receiver.is_some() {
                                ui.spinner();
                                ui.weak("Export in progress...");
                            }
                            include_popup_rectangle(
                                &mut popup_rectangle,
                                ui.min_rect().expand(6.0),
                            );
                        },
                    );
                    ui.menu_button(egui::RichText::new("Help").size(SYSTEM_TEXT_SIZE), |ui| {
                        ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                        ui.hyperlink_to(
                            "GitHub Link",
                            "https://github.com/LunaticGhoulPiano/TS-Analyzer",
                        );
                        include_popup_rectangle(&mut popup_rectangle, ui.min_rect().expand(6.0));
                    });
                });
            });

        if open_requested {
            self.open_transport_stream(frame);
        }
        if let Some(format) = export_requested {
            self.export_dialog(frame, format);
        }
        if let Some(path) = recent_requested {
            if path.is_file() {
                self.load_input(path);
            } else {
                self.set_local_error(
                    LogCategory::Input,
                    format!("Recent file not found: {}", path.display()),
                );
            }
        }
        if previous_theme != self.theme || opacity_changed {
            apply_theme(
                root_ui.ctx(),
                self.theme,
                self.transparent_background_opacity,
            );
            root_ui.ctx().request_repaint();
        }
        if previous_theme != self.theme {
            self.record_log(
                LogLevel::Info,
                LogCategory::Configuration,
                format!("Theme changed to {}", self.theme.name()),
            );
        }
        popup_rectangle
    }

    fn record_menu_contents(&mut self, ui: &mut egui::Ui) -> bool {
        ui.set_min_width(280.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Max Record Time").size(SYSTEM_TEXT_SIZE));
            ui.add_sized(
                [92.0, 24.0],
                egui::TextEdit::singleline(&mut self.record_max_time).hint_text("hh:mm:ss"),
            );
        });
        if !valid_record_time(&self.record_max_time) {
            ui.colored_label(
                ui.visuals().error_fg_color,
                "Use hh:mm:ss (minutes and seconds: 00–59)",
            );
        }
        let output_requested = ui
            .button(egui::RichText::new("Output File Path...").size(SYSTEM_TEXT_SIZE))
            .clicked();
        ui.label(
            self.record_output_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "No output file selected".to_owned()),
        );
        ui.separator();
        ui.weak("Recording becomes available when the selected live input is connected.");
        output_requested
    }

    fn navigation(&mut self, root_ui: &mut egui::Ui) {
        let mut action = None;
        let mut navigation_frame = egui::Frame::side_top_panel(root_ui.style());
        if self.theme == AppTheme::Transparent {
            navigation_frame = navigation_frame
                .fill(root_ui.visuals().window_fill)
                .stroke(root_ui.visuals().window_stroke);
        }
        egui::Panel::left("navigation")
            .resizable(true)
            .default_size(210.0)
            .frame(navigation_frame)
            .show(root_ui, |ui| {
                ui.add_space(6.0);
                ui.strong("Pages");
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("pages-navigation-scroll")
                    .max_height((ui.available_height() * 0.55).max(180.0))
                    .show(ui, |ui| self.pages_navigation(ui, &mut action));
                ui.separator();
                ui.strong(format!("TS Queue ({})", self.documents.len()));
                egui::ScrollArea::vertical()
                    .id_salt("ts-queue-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.queue_navigation(ui, &mut action));
            });
        if let Some(action) = action {
            self.apply_navigation(action);
        }
    }

    fn pages_navigation(&self, ui: &mut egui::Ui, action: &mut Option<NavigationAction>) {
        let analyzer = egui::CollapsingHeader::new("Analyzer")
            .default_open(true)
            .show(ui, |ui| {
                for (view, label) in [
                    (AnalyzerView::Overview, "Overview"),
                    (AnalyzerView::PsiSiTree, "PSI / SI Tree"),
                    (AnalyzerView::Packets, "Packets"),
                    (AnalyzerView::Tr101290, "TR 101 290"),
                ] {
                    if ui
                        .selectable_label(
                            self.page == Page::Analyzer && self.analyzer_view == view,
                            label,
                        )
                        .clicked()
                    {
                        *action = Some(NavigationAction::Analyzer(view));
                    }
                }
                egui::CollapsingHeader::new("Graphs")
                    .default_open(true)
                    .show(ui, |ui| {
                        for (view, label) in [
                            (AnalyzerView::Bitrate, "Bitrate"),
                            (AnalyzerView::Timestamps, "PCR / PTS / DTS"),
                            (AnalyzerView::Gop, "GOP"),
                        ] {
                            if ui
                                .selectable_label(
                                    self.page == Page::Analyzer && self.analyzer_view == view,
                                    label,
                                )
                                .clicked()
                            {
                                *action = Some(NavigationAction::Analyzer(view));
                            }
                        }
                    });
            });
        if analyzer.header_response.clicked() {
            *action = Some(NavigationAction::Page(Page::Analyzer));
        }
        let player = egui::CollapsingHeader::new("Player")
            .default_open(true)
            .show(ui, |ui| {
                for (view, label) in [
                    (PlayerView::PlayTs, "Play TS"),
                    (PlayerView::IpStreaming, "IP Streaming"),
                ] {
                    if ui
                        .selectable_label(
                            self.page == Page::Player && self.player_view == view,
                            label,
                        )
                        .clicked()
                    {
                        *action = Some(NavigationAction::Player(view));
                    }
                }
            });
        if player.header_response.clicked() {
            *action = Some(NavigationAction::Page(Page::Player));
        }
        if ui.selectable_label(self.page == Page::Log, "Log").clicked() {
            *action = Some(NavigationAction::Page(Page::Log));
        }
    }

    fn queue_navigation(&self, ui: &mut egui::Ui, action: &mut Option<NavigationAction>) {
        for (index, document) in self.documents.iter().enumerate() {
            let name = document
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| document.path.display().to_string());
            ui.horizontal(|ui| {
                let mut visible = self.visible_documents.contains(&index);
                if ui
                    .checkbox(&mut visible, "")
                    .on_hover_text("Show this TS in the analyzer")
                    .changed()
                {
                    *action = Some(NavigationAction::ToggleVisibleDocument(index));
                }
                let response = ui
                    .selectable_label(
                        self.selected_document == Some(index),
                        format!("{} {}", if document.expanded { "▾" } else { "▸" }, name),
                    )
                    .on_hover_text(document.path.display().to_string());
                if response.double_clicked() {
                    *action = Some(NavigationAction::ExpandDocument(index));
                } else if response.clicked() {
                    *action = Some(NavigationAction::SelectDocument(index));
                }
                if ui.small_button("×").on_hover_text("Close TS").clicked() {
                    *action = Some(NavigationAction::CloseDocument(index));
                }
            });
            if document.expanded {
                ui.indent(("queue-document", index), |ui| {
                    ui.small(document.path.display().to_string());
                    if ui.button("Play TS").clicked() {
                        *action = Some(NavigationAction::PlayDocument(index));
                    }
                });
            }
        }
    }

    fn apply_navigation(&mut self, action: NavigationAction) {
        match action {
            NavigationAction::Page(page) => self.select_page(page),
            NavigationAction::Analyzer(view) => {
                self.analyzer_view = view;
                self.select_page(Page::Analyzer);
            }
            NavigationAction::Player(view) => {
                self.player_view = view;
                if view == PlayerView::IpStreaming {
                    self.select_ip_streaming(
                        self.ip_streaming_protocol
                            .unwrap_or(IpStreamingProtocol::Udp),
                    );
                } else {
                    self.select_page(Page::Player);
                }
            }
            NavigationAction::SelectDocument(index) => {
                self.selected_document = Some(index);
                if !self.visible_documents.contains(&index) {
                    self.visible_documents.clear();
                    self.visible_documents.push(index);
                }
                self.analyzer_selected_pid = None;
                self.select_page(Page::Analyzer);
            }
            NavigationAction::ExpandDocument(index) => {
                self.documents[index].expanded = !self.documents[index].expanded;
                self.selected_document = Some(index);
                self.select_page(Page::Analyzer);
            }
            NavigationAction::ToggleVisibleDocument(index) => {
                if let Some(position) = self
                    .visible_documents
                    .iter()
                    .position(|&item| item == index)
                {
                    if self.visible_documents.len() > 1 {
                        self.visible_documents.remove(position);
                        self.selected_document = self.visible_documents.first().copied();
                    }
                } else if self.visible_documents.len() < 3 {
                    self.visible_documents.push(index);
                    self.selected_document = Some(index);
                    self.pane_widths = [1.0 / self.visible_documents.len() as f32; 3];
                } else {
                    self.set_local_error(
                        LogCategory::Analysis,
                        "At most three TS files can be displayed side by side.",
                    );
                }
                self.select_page(Page::Analyzer);
            }
            NavigationAction::CloseDocument(index) => self.close_document(index),
            NavigationAction::PlayDocument(index) => {
                let path = self.documents[index].path.clone();
                self.load_player_input(path);
            }
        }
    }

    fn close_document(&mut self, index: usize) {
        if index >= self.documents.len() {
            return;
        }
        let path = self.documents.remove(index).path;
        self.export_selected.remove(index);
        self.visible_documents.retain(|&item| item != index);
        for item in &mut self.visible_documents {
            if *item > index {
                *item -= 1;
            }
        }
        self.selected_document = self.selected_document.and_then(|current| {
            if current == index {
                self.visible_documents.first().copied().or_else(|| {
                    (!self.documents.is_empty()).then_some(index.min(self.documents.len() - 1))
                })
            } else {
                Some(current - usize::from(current > index))
            }
        });
        if self.visible_documents.is_empty() {
            if let Some(current) = self.selected_document {
                self.visible_documents.push(current);
            }
        }
        self.pane_widths = [1.0 / self.visible_documents.len().max(1) as f32; 3];
        self.record_log(
            LogLevel::Info,
            LogCategory::Analysis,
            format!("Closed {}", path.display()),
        );
        self.start_pending_analyses();
    }

    fn system_information(&self, ui: &mut egui::Ui) {
        let mode = if self.selected_document().is_some() || self.source_view.is_some() {
            "MPEG-TS"
        } else {
            "No input"
        };
        let standard = match self
            .selected_document()
            .and_then(|document| document.report.as_ref())
            .map(|report| report.standard)
        {
            Some(BroadcastStandard::AtscPsip) => "ATSC (PSIP detected)",
            Some(BroadcastStandard::DvbSi) => "DVB (SI detected)",
            Some(BroadcastStandard::Isdb) => "ISDB (TSDuck detected)",
            Some(BroadcastStandard::Scte) => "SCTE (TSDuck detected)",
            Some(BroadcastStandard::Dtmb) => "DTMB (TSDuck detected)",
            Some(BroadcastStandard::Unknown) | None => "Unknown",
        };
        egui::CollapsingHeader::new("System")
            .default_open(true)
            .show(ui, |ui| {
                let text = format_information_rows([
                    ("Mode", mode.to_owned()),
                    ("Standard", standard.to_owned()),
                ]);
                readonly_text_block(ui, "system-information-text", &text);
            });
    }

    fn video_information(&self, ui: &mut egui::Ui) {
        let media = self
            .selected_document()
            .and_then(|document| document.report.as_ref())
            .map(AnalysisReport::media_information);
        let codecs = media.as_ref().map_or_else(
            || "Unknown".to_owned(),
            |media| {
                if media.video_codecs.is_empty() {
                    "None".to_owned()
                } else {
                    media.video_codecs.join(", ")
                }
            },
        );
        let services = media.as_ref().map_or_else(
            || "Unknown".to_owned(),
            |media| media.video_services.to_string(),
        );
        let player = (self
            .selected_document()
            .map(|document| document.path.as_path())
            == self.snapshot.input.as_deref())
        .then_some(&self.snapshot.media_info);
        let width = media
            .as_ref()
            .and_then(|info| info.video_width)
            .map(|value| format!("{value} px (analyzed)"))
            .or_else(|| {
                player
                    .and_then(|info| info.video_width())
                    .map(|value| format!("{value} px (playback)"))
            })
            .unwrap_or_else(|| "Unknown".to_owned());
        let height = media
            .as_ref()
            .and_then(|info| info.video_height)
            .map(|value| format!("{value} px (analyzed)"))
            .or_else(|| {
                player
                    .and_then(|info| info.video_height())
                    .map(|value| format!("{value} px (playback)"))
            })
            .unwrap_or_else(|| "Unknown".to_owned());
        let frame_rate = media
            .as_ref()
            .and_then(|info| info.video_frame_rate)
            .filter(|(_, denominator)| *denominator != 0)
            .map(|(numerator, denominator)| {
                format!(
                    "{:.3} fps (analyzed)",
                    f64::from(numerator) / f64::from(denominator)
                )
            })
            .or_else(|| {
                player
                    .and_then(|info| info.video_frame_rate())
                    .filter(|(_, denominator)| *denominator != 0)
                    .map(|(numerator, denominator)| {
                        format!(
                            "{:.3} fps (playback)",
                            f64::from(numerator) / f64::from(denominator)
                        )
                    })
            })
            .unwrap_or_else(|| "Unknown".to_owned());
        egui::CollapsingHeader::new("Video")
            .default_open(true)
            .show(ui, |ui| {
                let text = format_information_rows([
                    ("Codec", codecs),
                    ("Width", width),
                    ("Height", height),
                    ("Frame Rate", frame_rate),
                    ("Services", services),
                ]);
                readonly_text_block(ui, "video-information-text", &text);
            });
    }

    fn audio_information(&self, ui: &mut egui::Ui) {
        let media = self
            .selected_document()
            .and_then(|document| document.report.as_ref())
            .map(AnalysisReport::media_information);
        let codecs = media.as_ref().map_or_else(
            || "Unknown".to_owned(),
            |media| {
                if media.audio_codecs.is_empty() {
                    "None".to_owned()
                } else {
                    media.audio_codecs.join(", ")
                }
            },
        );
        let tracks = media.as_ref().map_or_else(
            || "Unknown".to_owned(),
            |media| media.audio_tracks.to_string(),
        );
        egui::CollapsingHeader::new("Audio")
            .default_open(true)
            .show(ui, |ui| {
                let text = format_information_rows([("Codec", codecs), ("Tracks", tracks)]);
                readonly_text_block(ui, "audio-information-text", &text);
            });
    }

    fn information(&self, ui: &mut egui::Ui) {
        ui.heading("Media Information");
        self.system_information(ui);
        self.video_information(ui);
        self.audio_information(ui);
    }

    fn error_banner(&mut self, root_ui: &mut egui::Ui) {
        let message = self
            .local_error
            .as_deref()
            .or(self.snapshot.error.as_deref());
        let Some(message) = message else {
            return;
        };

        egui::Panel::top("error-banner").show(root_ui, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgb(85, 25, 25))
                .inner_margin(8.0)
                .show(ui, |ui| {
                    ui.colored_label(egui::Color32::LIGHT_RED, message);
                });
        });
    }

    fn analyzer_page(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .id_salt(("analyzer-page-scroll", self.analyzer_view as u8))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                middle_drag_scroll(ui, "analyzer-page-middle-scroll");
                match self.analyzer_view {
                    AnalyzerView::Overview => self.analyzer_overview(ui),
                    view => self.analyzer_section(ui, view),
                }
            });
    }

    fn analyzer_overview(&mut self, ui: &mut egui::Ui) {
        if let Some(document) = self.selected_document() {
            ui.label(document.path.display().to_string());
        }
        egui::ScrollArea::vertical()
            .id_salt("information-scroll")
            .max_height(300.0)
            .show(ui, |ui| self.information(ui));
        ui.separator();
        if let Some(error) = self
            .selected_document()
            .and_then(|document| document.error.as_ref())
        {
            ui.colored_label(ui.visuals().error_fg_color, error);
            return;
        }
        let Some(report) = self
            .selected_document
            .and_then(|index| self.documents.get(index))
            .and_then(|document| document.report.as_ref())
        else {
            if self
                .selected_document()
                .is_some_and(|document| document.receiver.is_some())
            {
                ui.spinner();
                ui.label("Analyzing transport stream...");
            } else if self.selected_document().is_some() {
                ui.label("Queued for analysis...");
            } else {
                ui.label("Import an MPEG transport stream to begin analysis.");
            }
            return;
        };
        let standard = match report.standard {
            BroadcastStandard::AtscPsip => "ATSC (PSIP detected)",
            BroadcastStandard::DvbSi => "DVB (SI detected)",
            BroadcastStandard::Isdb => "ISDB (TSDuck detected)",
            BroadcastStandard::Scte => "SCTE (TSDuck detected)",
            BroadcastStandard::Dtmb => "DTMB (TSDuck detected)",
            BroadcastStandard::Unknown => "Unknown",
        };
        ui.heading("Transport Stream");
        let mut rows = vec![
            ("Packet format", format!("{:?}", report.format)),
            ("Standard", standard.to_owned()),
            ("TS packets", format!("{} packets", report.packets)),
            (
                "Malformed packets",
                format!("{} packets", report.malformed_packets),
            ),
            ("Trailing bytes", format!("{} bytes", report.trailing_bytes)),
            ("Null packets", format!("{} packets", report.null_packets)),
            ("PIDs", report.pids.len().to_string()),
            ("Programs", report.programs.len().to_string()),
            (
                "Valid PSI/SI",
                format!("{} sections", report.valid_sections),
            ),
            (
                "Section CRC errors",
                format!("{} sections", report.section_crc_errors),
            ),
        ];
        if let Some(native) = report.native {
            rows.extend([
                ("TSDuck sections", native.valid_sections.to_string()),
                ("TSDuck CC errors", native.continuity_errors.to_string()),
            ]);
        }
        let summary = format_information_rows(rows);
        readonly_text_block(ui, "analyzer-summary-text", &summary);
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("overview-programs-scroll")
            .max_height(320.0)
            .show(ui, |ui| {
                egui::CollapsingHeader::new(format!("Programs ({})", report.programs.len()))
                    .default_open(true)
                    .show(ui, |ui| {
                        if report.programs.is_empty() {
                            ui.label("No valid PAT/PMT found.");
                        }
                        for (number, program) in &report.programs {
                            egui::CollapsingHeader::new(format!("Program {number}"))
                                .id_salt(number)
                                .default_open(true)
                                .show(ui, |ui| {
                                    let mut details = format!(
                                        "PMT PID: 0x{:04X}\nPCR PID: {}\n",
                                        program.pmt_pid,
                                        program.pcr_pid.map_or_else(
                                            || "Unknown".to_owned(),
                                            |pid| format!("0x{pid:04X}")
                                        )
                                    );
                                    for (pid, stream) in &program.streams {
                                        let _ = writeln!(
                                            details,
                                            "Stream PID 0x{pid:04X}: {} (type 0x{:02X})",
                                            stream.name_for_standard(report.standard),
                                            stream.stream_type
                                        );
                                    }
                                    if program.streams.is_empty() {
                                        details.push_str("No elementary streams found.");
                                    }
                                    readonly_text_block(ui, "analyzer-program-text", &details);
                                });
                        }
                    });
            });
        ui.separator();
        ui.heading(format!("PIDs ({})", report.pids.len()));
        ui.horizontal(|ui| {
            ui.label("Filter:");
            ui.add(
                egui::TextEdit::singleline(&mut self.analyzer_pid_filter)
                    .hint_text("PID, hex or decimal")
                    .desired_width(180.0),
            );
        });
        let needle = self.analyzer_pid_filter.trim().to_ascii_lowercase();
        let mut visible = 0;
        egui::ScrollArea::vertical()
            .id_salt("overview-pids-scroll")
            .max_height(280.0)
            .show(ui, |ui| {
                for (pid, stats) in &report.pids {
                    let role = if *pid == 0 {
                        "PAT"
                    } else if *pid == 0x1fff {
                        "Null"
                    } else if *pid == 0x1ffb {
                        "ATSC PSIP"
                    } else if report
                        .programs
                        .values()
                        .any(|program| program.pmt_pid == *pid)
                    {
                        "PMT"
                    } else {
                        report
                            .programs
                            .values()
                            .find_map(|program| program.streams.get(pid))
                            .map_or("Other", |stream| stream.name_for_standard(report.standard))
                    };
                    let matches = needle.is_empty()
                        || format!("0x{pid:04x}").contains(&needle)
                        || format!("0x{pid:x}").contains(&needle)
                        || pid.to_string().contains(&needle)
                        || role.to_ascii_lowercase().contains(&needle);
                    if !matches {
                        continue;
                    }
                    visible += 1;
                    ui.selectable_value(
                        &mut self.analyzer_selected_pid,
                        Some(*pid),
                        format!("0x{pid:04X}  {role}  ({} packets)", stats.packets),
                    );
                }
                if visible == 0 {
                    ui.label("No PIDs match the filter.");
                }
            });
        if let Some(pid) = self.analyzer_selected_pid
            && let Some(stats) = report.pids.get(&pid)
        {
            ui.separator();
            ui.heading(format!("PID 0x{pid:04X}"));
            let details = format_information_rows([
                ("Packets", stats.packets.to_string()),
                ("Payload packets", stats.payload_packets.to_string()),
                ("Transport errors", stats.transport_errors.to_string()),
                ("CC errors", stats.continuity_errors.to_string()),
                ("Duplicates", stats.duplicates.to_string()),
                ("Scrambled packets", stats.scrambled_packets.to_string()),
                ("PCR samples", stats.pcr_samples.to_string()),
                (
                    "Max PCR gap",
                    stats.max_pcr_gap_27mhz.map_or_else(
                        || "Unknown".to_owned(),
                        |ticks| format!("{:.3} ms", ticks as f64 / 27_000.0),
                    ),
                ),
            ]);
            readonly_text_block(ui, "analyzer-pid-details", &details);
        }
    }

    fn analyzer_section(&mut self, ui: &mut egui::Ui, view: AnalyzerView) {
        let Some(index) = self.selected_document else {
            ui.label("Import a transport stream to analyze this page.");
            return;
        };
        let Some(document) = self.documents.get(index) else {
            return;
        };
        if let Some(error) = &document.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
            return;
        }
        let Some(report) = document.report.as_ref() else {
            ui.spinner();
            ui.label("Analyzing transport stream...");
            return;
        };
        let state = &mut self.analysis_view_state;
        match view {
            AnalyzerView::Overview => {}
            AnalyzerView::PsiSiTree => analysis_views::psi_si_tree(ui, report, state),
            AnalyzerView::Packets => analysis_views::packets(ui, &document.path, report, state),
            AnalyzerView::Tr101290 => analysis_views::tr_101_290(ui, report, state),
            AnalyzerView::Bitrate => analysis_views::bitrate(ui, report, state),
            AnalyzerView::Timestamps => analysis_views::timestamps(ui, report, state),
            AnalyzerView::Gop => analysis_views::gop(ui, report, state),
        }
    }

    fn player_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Player settings");
        ui.add_space(8.0);
        let previous_backend = self.selected_backend;
        let previous_adapter = self.selected_adapter;
        egui::ComboBox::from_label("Video backend")
            .selected_text(self.selected_backend.to_string())
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.selected_backend,
                    PlayerBackendKind::D3d12,
                    "D3D12",
                );
                ui.selectable_value(
                    &mut self.selected_backend,
                    PlayerBackendKind::D3d11,
                    "D3D11",
                );
            });
        if previous_backend != self.selected_backend {
            self.selected_adapter = GpuAdapterSelection::Default;
            self.ensure_selected_adapter();
        }

        let adapters = self.current_adapters().to_vec();
        let selected_label = adapters
            .iter()
            .find(|adapter| adapter.selection() == self.selected_adapter)
            .map(adapter_label)
            .unwrap_or_else(|| "No hardware adapter detected".to_owned());
        egui::ComboBox::from_label("GPU adapter")
            .selected_text(selected_label)
            .show_ui(ui, |ui| {
                for adapter in &adapters {
                    ui.selectable_value(
                        &mut self.selected_adapter,
                        adapter.selection(),
                        adapter_label(adapter),
                    );
                }
            });
        ui.label("The selected backend and adapter apply to the next file loaded in Player.");
        if previous_backend != self.selected_backend || previous_adapter != self.selected_adapter {
            self.record_log(
                LogLevel::Info,
                LogCategory::Configuration,
                format!(
                    "Player backend selection changed to {} on {}",
                    self.selected_backend, self.selected_adapter
                ),
            );
        }
    }

    fn document_tabs(&mut self, ui: &mut egui::Ui) {
        if self.documents.is_empty() {
            return;
        }
        let mut selected = None;
        let mut closed = None;
        egui::ScrollArea::horizontal()
            .id_salt("document-tabs-scroll")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (index, document) in self.documents.iter().enumerate() {
                        let name = document
                            .path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| document.path.display().to_string());
                        ui.horizontal(|ui| {
                            if ui
                                .selectable_label(self.selected_document == Some(index), name)
                                .on_hover_text(document.path.display().to_string())
                                .clicked()
                            {
                                selected = Some(index);
                            }
                            if ui.small_button("×").on_hover_text("Close TS").clicked() {
                                closed = Some(index);
                            }
                            ui.separator();
                        });
                    }
                });
            });
        ui.separator();
        if let Some(index) = closed {
            self.close_document(index);
        } else if let Some(index) = selected {
            self.selected_document = Some(index);
            if !self.visible_documents.contains(&index) {
                self.visible_documents.clear();
                self.visible_documents.push(index);
            }
            self.analyzer_selected_pid = None;
            self.select_page(Page::Analyzer);
        }
    }

    fn analyzer_workspace(&mut self, ui: &mut egui::Ui) {
        let visible = self.visible_documents.clone();
        if visible.len() < 2 {
            self.selected_document = visible.first().copied().or(self.selected_document);
            self.analyzer_page(ui);
            return;
        }

        let count = visible.len();
        let height = ui.available_height();
        let handle_width = 12.0;
        let usable =
            (ui.available_width() - handle_width * (count - 1) as f32).max(count as f32 * 220.0);
        let weight_sum: f32 = self.pane_widths[..count].iter().sum();
        let widths = (0..count)
            .map(|pane| usable * self.pane_widths[pane] / weight_sum.max(f32::EPSILON))
            .collect::<Vec<_>>();
        let focused = self.selected_document;
        let mut dragged = None;
        egui::ScrollArea::horizontal()
            .id_salt("analyzer-panes-horizontal")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (pane, index) in visible.iter().copied().enumerate() {
                        ui.allocate_ui(egui::vec2(widths[pane], height), |ui| {
                            ui.set_width(widths[pane]);
                            self.selected_document = Some(index);
                            ui.push_id(("analyzer-pane", index), |ui| self.analyzer_page(ui));
                        });
                        if pane + 1 < count {
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(handle_width, height),
                                egui::Sense::drag(),
                            );
                            let response = response
                                .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
                                .on_hover_text("Drag to resize analyzer panes");
                            ui.painter().rect_filled(
                                rect,
                                2.0,
                                ui.visuals().widgets.inactive.bg_fill,
                            );
                            ui.painter().vline(
                                rect.center().x,
                                rect.y_range().shrink(8.0),
                                ui.visuals().widgets.noninteractive.fg_stroke,
                            );
                            if response.dragged() {
                                dragged = Some(pane);
                            }
                        }
                    }
                });
            });
        self.selected_document = focused;
        if let Some(pane) = dragged {
            let delta = ui.ctx().input(|input| input.pointer.delta().x) / usable;
            let lower = 220.0 / usable;
            let left = self.pane_widths[pane];
            let right = self.pane_widths[pane + 1];
            let updated = (left + delta).clamp(lower, left + right - lower);
            self.pane_widths[pane] = updated;
            self.pane_widths[pane + 1] = left + right - updated;
            ui.ctx().request_repaint();
        }
    }

    fn player_page(
        &mut self,
        ui: &mut egui::Ui,
        context: &egui::Context,
        popup_rectangle: Option<egui::Rect>,
        frame: &eframe::Frame,
    ) {
        if self.player_view == PlayerView::IpStreaming {
            self.ip_streaming_page(ui, frame);
            ui.add_space(16.0);
            self.player_settings(ui);
            return;
        }

        match self.source_view {
            Some(InputSourceKind::TransportStreamFile) => {
                match self.video_mode {
                    VideoMode::Embedded => {
                        self.embedded_video(ui, context, popup_rectangle);
                    }
                    VideoMode::Detached | VideoMode::Fullscreen => {
                        egui::Frame::group(ui.style())
                            .inner_margin(24.0)
                            .show(ui, |ui| {
                                ui.label("Video is displayed in a separate window");
                            });
                    }
                }

                ui.add_space(12.0);
                egui::ScrollArea::vertical()
                    .id_salt("player-details-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        middle_drag_scroll(ui, "player-details-middle-scroll");
                        self.player_details(ui);
                        ui.add_space(16.0);
                        self.player_settings(ui);
                    });
                return;
            }
            Some(InputSourceKind::IpStreaming) | None => {
                ui.add_space(12.0);
                egui::Frame::group(ui.style())
                    .inner_margin(24.0)
                    .show(ui, |ui| {
                        ui.label("No playable MPEG transport stream is loaded.");
                        ui.label("Import a TS file, then select it from the queue.");
                    });
                if let Some(path) = self
                    .selected_document()
                    .map(|document| document.path.clone())
                    && ui.button("Play selected TS").clicked()
                {
                    self.load_player_input(path);
                }
            }
        }

        ui.add_space(16.0);
        self.player_settings(ui);
    }

    fn ip_streaming_page(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        ui.heading("IP Streaming");
        let mut requested_protocol = None;
        ui.horizontal(|ui| {
            for protocol in [IpStreamingProtocol::Udp, IpStreamingProtocol::Rtp] {
                if ui
                    .selectable_label(
                        self.ip_streaming_protocol == Some(protocol),
                        protocol.name(),
                    )
                    .clicked()
                {
                    requested_protocol = Some(protocol);
                }
            }
        });
        if let Some(protocol) = requested_protocol {
            self.select_ip_streaming(protocol);
        }
        ui.add_space(12.0);
        egui::Frame::group(ui.style())
            .inner_margin(24.0)
            .show(ui, |ui| {
                ui.label("UDP/RTP live input is not connected yet.");
                ui.weak(
                    "Protocol selection is available; receiver and recording backend are pending.",
                );
            });
        ui.add_space(12.0);
        let output_requested = self.record_menu_contents(ui);
        if output_requested {
            self.select_record_output(frame);
        }
    }

    fn embedded_video(
        &mut self,
        ui: &mut egui::Ui,
        context: &egui::Context,
        popup_rectangle: Option<egui::Rect>,
    ) {
        let width = ui.available_width().max(320.0);
        let maximum_height =
            (ui.available_height() - PLAYER_CONTROLS_HEIGHT - PLAYER_DETAILS_RESERVED_HEIGHT)
                .max(180.0);
        let height = self
            .embedded_video_height
            .unwrap_or(width * 9.0 / 16.0)
            .clamp(180.0, maximum_height);
        let (rectangle, _) =
            ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
        ui.painter()
            .rect_filled(rectangle, 4.0, egui::Color32::BLACK);
        ui.painter().rect_stroke(
            rectangle,
            4.0,
            egui::Stroke::new(1.0, egui::Color32::DARK_GRAY),
            egui::StrokeKind::Inside,
        );

        if let Some(video_rectangle) =
            physical_video_rectangle(rectangle, context.pixels_per_point())
            && let Some(host) = &self.embedded_video_host
        {
            self.embedded_window_rectangle = Some(video_rectangle);
            let surface = host.surface();
            let exclusion = popup_rectangle.and_then(|popup_rectangle| {
                physical_video_exclusion(rectangle, popup_rectangle, context.pixels_per_point())
            });
            let output_ready = matches!(
                self.snapshot.state,
                PlayerState::Playing | PlayerState::Paused
            );
            let placement = if output_ready {
                host.show_with_exclusion(video_rectangle, exclusion)
            } else {
                host.prepare_with_exclusion(video_rectangle, exclusion)
            };
            match placement {
                Ok(host_rectangle) => self.attach_surface(surface, host_rectangle),
                Err(error) => self.set_local_error(LogCategory::Pipeline, error.to_string()),
            }
        }

        let (handle, response) =
            ui.allocate_exact_size(egui::vec2(width, 8.0), egui::Sense::drag());
        ui.painter().line_segment(
            [
                egui::pos2(handle.left() + 24.0, handle.center().y),
                egui::pos2(handle.right() - 24.0, handle.center().y),
            ],
            egui::Stroke::new(3.0, ui.visuals().widgets.noninteractive.fg_stroke.color),
        );
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        if response.dragged() {
            self.embedded_video_height = Some(
                (height + ui.input(|input| input.pointer.delta().y)).clamp(180.0, maximum_height),
            );
        }

        if let Some(action) = player_control_bar(
            ui,
            self.snapshot.state,
            self.video_mode,
            self.snapshot.position,
            self.snapshot.duration,
            &mut self.timeline_drag_seconds,
        ) {
            self.apply_player_action(action);
        }
    }

    fn detached_video(&mut self, context: &egui::Context) {
        let title = format!("TS Analyzer Video [{}]", std::process::id());
        if self.detached_video_host.is_none() {
            match WindowsVideoHost::for_process_window_title(&title) {
                Ok(Some(host)) => self.detached_video_host = Some(host),
                Ok(None) => {}
                Err(error) => {
                    self.set_local_error(LogCategory::Pipeline, error.to_string());
                }
            }
        }

        let fullscreen = self.video_mode == VideoMode::Fullscreen;
        let mut close_requested = false;
        let mut exit_fullscreen_requested = false;
        let mut control_action = None;
        let mut surface_update = None;
        let mut surface_error = None;
        let builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size([1280.0, 720.0])
            .with_min_inner_size([640.0, 360.0])
            .with_fullscreen(fullscreen);

        context.show_viewport_immediate(
            egui::ViewportId::from_hash_of(VIDEO_VIEWPORT_ID),
            builder,
            |viewport_ui, _class| {
                close_requested = viewport_ui.input(|input| input.viewport().close_requested());
                exit_fullscreen_requested =
                    viewport_ui.input(|input| input.key_pressed(egui::Key::Escape) && fullscreen);
                let pixels_per_point = viewport_ui.ctx().pixels_per_point();
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(egui::Color32::BLACK))
                    .show(viewport_ui, |ui| {
                        let rectangle = ui.available_rect_before_wrap();
                        ui.allocate_rect(rectangle, egui::Sense::hover());
                        let controls_top =
                            (rectangle.max.y - PLAYER_CONTROLS_HEIGHT).max(rectangle.min.y);
                        let video_area = egui::Rect::from_min_max(
                            rectangle.min,
                            egui::pos2(rectangle.max.x, controls_top),
                        );
                        let controls_area = egui::Rect::from_min_max(
                            egui::pos2(rectangle.min.x, controls_top),
                            rectangle.max,
                        );
                        ui.painter()
                            .rect_filled(video_area, 0.0, egui::Color32::BLACK);
                        ui.scope_builder(egui::UiBuilder::new().max_rect(controls_area), |ui| {
                            control_action = player_control_bar(
                                ui,
                                self.snapshot.state,
                                self.video_mode,
                                self.snapshot.position,
                                self.snapshot.duration,
                                &mut self.timeline_drag_seconds,
                            );
                        });
                        let Some(video_rectangle) =
                            physical_video_rectangle(video_area, pixels_per_point)
                        else {
                            return;
                        };
                        let Some(host) = self.detached_video_host.as_ref() else {
                            return;
                        };
                        let surface = host.surface();
                        match host.show(video_rectangle) {
                            Ok(host_rectangle) => {
                                surface_update = Some((surface, host_rectangle));
                            }
                            Err(error) => surface_error = Some(error.to_string()),
                        }
                    });
            },
        );

        if let Some(error) = surface_error {
            self.set_local_error(LogCategory::Pipeline, error);
        }
        if let Some((surface, rectangle)) = surface_update {
            self.attach_surface(surface, rectangle);
        }
        if close_requested {
            self.set_video_mode(VideoMode::Embedded);
        } else if exit_fullscreen_requested {
            self.set_video_mode(self.fullscreen_return_mode);
        } else if let Some(action) = control_action {
            self.apply_player_action(action);
        }
    }

    fn player_details(&self, ui: &mut egui::Ui) {
        let mut rows = vec![
            (
                "Input",
                self.snapshot
                    .input
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "Not loaded".to_owned()),
            ),
            ("Source type", "MPEG transport stream file".to_owned()),
            (
                "Packet size",
                self.snapshot
                    .transport_packet_size
                    .map(|size| format!("{size} bytes"))
                    .unwrap_or_else(|| "Unknown".to_owned()),
            ),
            ("Player state", state_name(self.snapshot.state).to_owned()),
            ("Warnings", self.snapshot.warning_count.to_string()),
            ("QoS events", self.snapshot.qos_count.to_string()),
            ("Backend", self.snapshot.kind.to_string()),
            ("Adapter", self.snapshot.adapter.to_string()),
            (
                "Video pad linked",
                if self.snapshot.video_pad_linked {
                    "Yes".to_owned()
                } else {
                    "No".to_owned()
                },
            ),
        ];
        if let Some(decoder) = &self.snapshot.decoder {
            rows.extend([
                ("Decoder", decoder.name().to_owned()),
                ("Factory", decoder.factory().to_owned()),
                ("Vendor ID", format_optional_hex(decoder.vendor_id())),
                ("Device ID", format_optional_hex(decoder.device_id())),
                (
                    "Adapter LUID",
                    decoder
                        .adapter_luid()
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "Unavailable".to_owned()),
                ),
            ]);
        }
        let text = format_information_rows(rows);
        readonly_text_block(ui, "player-details-text", &text)
            .on_hover_text("Select text normally; use Shift+Arrow to extend the selection");
    }

    fn log_page(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        ui.horizontal(|ui| {
            ui.heading("Log");
            if ui.button("Dump...").clicked() {
                self.dump_log(frame);
            }
        });
        ui.add_space(8.0);
        ui.label(
            "Categories: System, Input, Playback, Pipeline, Analysis, Configuration, Import/Export",
        );
        ui.separator();

        let text = self
            .log_entries
            .iter()
            .map(LogEntry::dump_line)
            .collect::<Vec<_>>()
            .join("\n");
        egui::ScrollArea::both()
            .id_salt("log-scroll")
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                middle_drag_scroll(ui, "log-middle-scroll");
                readonly_log_text(ui, "log-text", &text);
            });
    }

    fn dump_log(&mut self, frame: &eframe::Frame) {
        let Some(window) = frame.winit_window() else {
            self.set_local_error(LogCategory::System, "native root window is unavailable");
            return;
        };

        let path = match save_log_dialog(window) {
            Ok(Some(path)) => path,
            Ok(None) => return,
            Err(error) => {
                self.set_local_error(LogCategory::ImportExport, error);
                return;
            }
        };
        self.record_log(
            LogLevel::Info,
            LogCategory::ImportExport,
            format!("Dumping log to {}", path.display()),
        );
        let mut contents = String::from(
            "TS Analyzer log\nCategories: System, Input, Playback, Pipeline, Analysis, Configuration, Import/Export\n\n",
        );
        for entry in &self.log_entries {
            contents.push_str(&entry.dump_line());
            contents.push('\n');
        }

        if let Err(error) = fs::write(&path, contents) {
            let message = format!("failed to dump log to '{}': {error}", path.display());
            self.set_local_error(LogCategory::ImportExport, message);
        }
    }
}

fn include_popup_rectangle(target: &mut Option<egui::Rect>, rectangle: egui::Rect) {
    *target = Some(match *target {
        Some(current) => current.union(rectangle),
        None => rectangle,
    });
}

fn valid_record_time(value: &str) -> bool {
    let mut fields = value.split(':');
    let (Some(hours), Some(minutes), Some(seconds), None) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return false;
    };
    hours.parse::<u32>().is_ok()
        && minutes.parse::<u8>().is_ok_and(|value| value < 60)
        && seconds.parse::<u8>().is_ok_and(|value| value < 60)
}

fn format_information_rows(rows: impl IntoIterator<Item = (&'static str, String)>) -> String {
    rows.into_iter()
        .map(|(label, value)| format!("{label:<18}{value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn readonly_text_block(ui: &mut egui::Ui, id_salt: &'static str, text: &str) -> egui::Response {
    let mut buffer = text;
    ui.add(
        egui::TextEdit::multiline(&mut buffer)
            .id_salt(id_salt)
            .font(egui::TextStyle::Monospace)
            .desired_width(f32::INFINITY)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO),
    )
}

fn readonly_log_text(ui: &mut egui::Ui, id_salt: &'static str, text: &str) -> egui::Response {
    let mut buffer = text;
    let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, wrap_width: f32| {
        let mut job = egui::text::LayoutJob::default();
        job.wrap.max_width = wrap_width;
        let font_id = egui::TextStyle::Monospace.resolve(ui.style());
        for line in buffer.as_str().split_inclusive('\n') {
            let color = if line.contains("[ERROR]") {
                ui.visuals().error_fg_color
            } else if line.contains("[WARN]") {
                ui.visuals().warn_fg_color
            } else {
                ui.visuals().text_color()
            };
            job.append(
                line,
                0.0,
                egui::TextFormat {
                    font_id: font_id.clone(),
                    color,
                    ..Default::default()
                },
            );
        }
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    ui.add(
        egui::TextEdit::multiline(&mut buffer)
            .id_salt(id_salt)
            .desired_width(f32::INFINITY)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .layouter(&mut layouter),
    )
}

fn recent_files_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|directory| {
        PathBuf::from(directory)
            .join("TS-Analyzer")
            .join("recent-files.txt")
    })
}

fn load_recent_files() -> Vec<PathBuf> {
    let Some(path) = recent_files_path() else {
        return Vec::new();
    };
    fs::read_to_string(path).map_or_else(
        |_| Vec::new(),
        |contents| {
            contents
                .lines()
                .filter(|line| !line.is_empty())
                .take(12)
                .map(PathBuf::from)
                .collect()
        },
    )
}

fn save_recent_files(paths: &[PathBuf]) -> std::io::Result<()> {
    let Some(path) = recent_files_path() else {
        return Ok(());
    };
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory)?;
    }
    let contents = paths
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(path, contents)
}

fn middle_drag_scroll(ui: &egui::Ui, id_salt: &'static str) {
    let state_id = ui.id().with(id_salt);
    let clip_rectangle = ui.clip_rect();
    let (pressed, released, down, press_origin, delta) = ui.input(|input| {
        (
            input.pointer.button_pressed(egui::PointerButton::Middle),
            input.pointer.button_released(egui::PointerButton::Middle),
            input.pointer.middle_down(),
            input.pointer.press_origin(),
            input.pointer.delta(),
        )
    });

    if pressed && press_origin.is_some_and(|position| clip_rectangle.contains(position)) {
        ui.data_mut(|data| data.insert_temp(state_id, true));
    }
    if released {
        ui.data_mut(|data| data.remove::<bool>(state_id));
    }
    let active = ui
        .data(|data| data.get_temp::<bool>(state_id))
        .unwrap_or(false);
    if active && down && delta != egui::Vec2::ZERO {
        ui.scroll_with_delta(delta);
    }
}

impl eframe::App for TsanApp {
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        if self.theme == AppTheme::Transparent {
            egui::Color32::TRANSPARENT.to_normalized_gamma_f32()
        } else {
            egui::Color32::from_rgb(
                visuals.panel_fill.r(),
                visuals.panel_fill.g(),
                visuals.panel_fill.b(),
            )
            .to_normalized_gamma_f32()
        }
    }

    fn ui(&mut self, root_ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.receive_snapshots();
        let context = root_ui.ctx().clone();
        context.request_repaint_after(Duration::from_millis(50));

        let popup_rectangle = self.application_header(root_ui, frame);
        self.error_banner(root_ui);
        self.navigation(root_ui);
        egui::CentralPanel::default().show(root_ui, |ui| {
            if self.page == Page::Analyzer {
                self.document_tabs(ui);
            }
            match self.page {
                Page::Analyzer => self.analyzer_workspace(ui),
                Page::Player => self.player_page(ui, &context, popup_rectangle, frame),
                Page::Log => self.log_page(ui, frame),
            }
        });

        if self.source_view == Some(InputSourceKind::TransportStreamFile)
            && matches!(self.video_mode, VideoMode::Detached | VideoMode::Fullscreen)
        {
            self.detached_video(&context);
        }
    }
}

fn player_control_bar(
    ui: &mut egui::Ui,
    state: PlayerState,
    video_mode: VideoMode,
    position: Option<Duration>,
    duration: Option<Duration>,
    timeline_drag_seconds: &mut Option<f64>,
) -> Option<PlayerUiAction> {
    let mut action = None;
    let available_width = ui.available_width();
    egui::Frame::new()
        .fill(egui::Color32::from_gray(20))
        .inner_margin(egui::Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.set_width((available_width - 16.0).max(0.0));
            ui.horizontal(|ui| {
                let total_seconds = duration.map(|value| value.as_secs_f64()).unwrap_or(0.0);
                let current_seconds = position.map(|value| value.as_secs_f64()).unwrap_or(0.0);
                let mut displayed_seconds = timeline_drag_seconds
                    .unwrap_or(current_seconds)
                    .clamp(0.0, total_seconds.max(0.0));
                let seek_enabled = total_seconds > 0.0
                    && matches!(state, PlayerState::Playing | PlayerState::Paused);
                let slider_width = (ui.available_width() - 110.0).max(80.0);
                let slider =
                    egui::Slider::new(&mut displayed_seconds, 0.0..=total_seconds.max(1.0))
                        .show_value(false)
                        .trailing_fill(true);
                let response = ui.add_enabled_ui(seek_enabled, |ui| {
                    ui.spacing_mut().slider_width = slider_width;
                    ui.add(slider)
                });
                let response = response.inner;
                let pointer_active = response.is_pointer_button_down_on() || response.dragged();
                if pointer_active {
                    *timeline_drag_seconds = Some(displayed_seconds);
                }
                let pointer_committed = response.drag_stopped_by(egui::PointerButton::Primary)
                    || response.clicked_by(egui::PointerButton::Primary);
                let keyboard_committed =
                    response.changed() && !pointer_active && !pointer_committed;
                if pointer_committed || keyboard_committed {
                    action = Some(PlayerUiAction::Seek(Duration::from_secs_f64(
                        displayed_seconds,
                    )));
                    *timeline_drag_seconds = Some(displayed_seconds);
                } else if !seek_enabled
                    || (!pointer_active
                        && timeline_drag_seconds
                            .is_some_and(|target| (current_seconds - target).abs() <= 1.0))
                {
                    *timeline_drag_seconds = None;
                }
                ui.label(format!(
                    "{} / {}",
                    format_playback_time(Duration::from_secs_f64(displayed_seconds)),
                    duration
                        .map(format_playback_time)
                        .unwrap_or_else(|| "--:--".to_owned())
                ));
            });
            ui.horizontal(|ui| {
                let loaded = !matches!(state, PlayerState::Idle);
                if state == PlayerState::Playing {
                    if icon_button(ui, PlayerControlIcon::Pause, true, false, "Pause") {
                        action = Some(PlayerUiAction::Pause);
                    }
                } else if icon_button(
                    ui,
                    PlayerControlIcon::Play,
                    matches!(state, PlayerState::Stopped | PlayerState::Paused),
                    false,
                    "Play",
                ) {
                    action = Some(PlayerUiAction::Play);
                }
                if icon_button(ui, PlayerControlIcon::Stop, loaded, false, "Stop") {
                    action = Some(PlayerUiAction::Stop);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icon_button(
                        ui,
                        PlayerControlIcon::Fullscreen,
                        loaded,
                        video_mode == VideoMode::Fullscreen,
                        if video_mode == VideoMode::Fullscreen {
                            "Exit fullscreen"
                        } else {
                            "Fullscreen"
                        },
                    ) {
                        action = Some(PlayerUiAction::ToggleFullscreen);
                    }
                    if icon_button(
                        ui,
                        PlayerControlIcon::Detached,
                        loaded && video_mode != VideoMode::Detached,
                        video_mode == VideoMode::Detached,
                        "Open in new window",
                    ) {
                        action = Some(PlayerUiAction::SetVideoMode(VideoMode::Detached));
                    }
                    if icon_button(
                        ui,
                        PlayerControlIcon::Embedded,
                        loaded && video_mode != VideoMode::Embedded,
                        video_mode == VideoMode::Embedded,
                        "Embedded player",
                    ) {
                        action = Some(PlayerUiAction::SetVideoMode(VideoMode::Embedded));
                    }
                });
            });
        });
    action
}

fn apply_theme(context: &egui::Context, theme: AppTheme, transparent_background_opacity: u8) {
    context.set_visuals_of(egui::Theme::Dark, egui::Visuals::dark());
    context.set_visuals_of(egui::Theme::Light, egui::Visuals::light());

    match theme {
        AppTheme::System => context.set_theme(egui::ThemePreference::System),
        AppTheme::Dark => context.set_theme(egui::ThemePreference::Dark),
        AppTheme::Light => context.set_theme(egui::ThemePreference::Light),
        AppTheme::Transparent => {
            context.set_theme(egui::ThemePreference::Dark);
            context.set_visuals(transparent_visuals(transparent_background_opacity));
        }
    }
}

fn transparent_visuals(background_opacity: u8) -> egui::Visuals {
    let mut visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(egui::Color32::WHITE);
    visuals.panel_fill = egui::Color32::from_rgba_unmultiplied(48, 52, 60, background_opacity);
    visuals.window_fill =
        egui::Color32::from_rgba_unmultiplied(16, 16, 16, background_opacity.saturating_add(40));
    visuals.extreme_bg_color =
        egui::Color32::from_rgba_unmultiplied(8, 8, 8, background_opacity.saturating_add(8));
    visuals.faint_bg_color = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 20);
    visuals.window_corner_radius = egui::CornerRadius::same(10);
    visuals.menu_corner_radius = egui::CornerRadius::same(8);
    visuals.window_stroke = egui::Stroke::new(
        1.0,
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 80),
    );
    visuals.selection.bg_fill = egui::Color32::from_rgb(40, 132, 190);
    visuals.widgets.noninteractive.bg_fill =
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 24);
    visuals.widgets.inactive.bg_fill = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 32);
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 56);
    visuals.widgets.active.bg_fill = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 72);
    visuals
}

fn format_playback_time(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3_600;
    let minutes = total_seconds % 3_600 / 60;
    let seconds = total_seconds % 60;

    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn icon_button(
    ui: &mut egui::Ui,
    icon: PlayerControlIcon,
    enabled: bool,
    selected: bool,
    tooltip: &str,
) -> bool {
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rectangle, response) =
        ui.allocate_exact_size(egui::vec2(PLAYER_ICON_SIZE, PLAYER_ICON_SIZE), sense);
    let response = response.on_hover_text(tooltip);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, tooltip));

    if selected {
        ui.painter()
            .rect_filled(rectangle, 4.0, egui::Color32::from_gray(65));
    } else if enabled && response.hovered() {
        ui.painter()
            .rect_filled(rectangle, 4.0, egui::Color32::from_gray(48));
    }

    let color = if enabled {
        egui::Color32::WHITE
    } else {
        egui::Color32::from_gray(95)
    };
    paint_control_icon(ui.painter(), rectangle, icon, color);
    enabled && response.clicked()
}

fn paint_control_icon(
    painter: &egui::Painter,
    rectangle: egui::Rect,
    icon: PlayerControlIcon,
    color: egui::Color32,
) {
    let center = rectangle.center();
    let stroke = egui::Stroke::new(1.8, color);
    match icon {
        PlayerControlIcon::Play => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(center.x - 5.0, center.y - 8.0),
                    egui::pos2(center.x + 8.0, center.y),
                    egui::pos2(center.x - 5.0, center.y + 8.0),
                ],
                color,
                egui::Stroke::NONE,
            ));
        }
        PlayerControlIcon::Pause => {
            painter.rect_filled(
                egui::Rect::from_center_size(
                    egui::pos2(center.x - 4.0, center.y),
                    egui::vec2(4.0, 16.0),
                ),
                0.0,
                color,
            );
            painter.rect_filled(
                egui::Rect::from_center_size(
                    egui::pos2(center.x + 4.0, center.y),
                    egui::vec2(4.0, 16.0),
                ),
                0.0,
                color,
            );
        }
        PlayerControlIcon::Stop => {
            painter.rect_filled(
                egui::Rect::from_center_size(center, egui::vec2(13.0, 13.0)),
                1.0,
                color,
            );
        }
        PlayerControlIcon::Embedded => {
            painter.rect_stroke(
                egui::Rect::from_center_size(center, egui::vec2(20.0, 15.0)),
                1.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.rect_stroke(
                egui::Rect::from_min_size(
                    egui::pos2(center.x + 1.0, center.y + 1.0),
                    egui::vec2(7.0, 5.0),
                ),
                0.0,
                stroke,
                egui::StrokeKind::Inside,
            );
        }
        PlayerControlIcon::Detached => {
            painter.rect_stroke(
                egui::Rect::from_center_size(center, egui::vec2(19.0, 15.0)),
                1.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x, center.y),
                    egui::pos2(center.x + 8.0, center.y - 8.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x + 3.0, center.y - 8.0),
                    egui::pos2(center.x + 8.0, center.y - 8.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x + 8.0, center.y - 8.0),
                    egui::pos2(center.x + 8.0, center.y - 3.0),
                ],
                stroke,
            );
        }
        PlayerControlIcon::Fullscreen => {
            let min = center - egui::vec2(9.0, 7.0);
            let max = center + egui::vec2(9.0, 7.0);
            for points in [
                [
                    egui::pos2(min.x + 6.0, min.y),
                    min,
                    egui::pos2(min.x, min.y + 5.0),
                ],
                [
                    egui::pos2(max.x - 6.0, min.y),
                    egui::pos2(max.x, min.y),
                    egui::pos2(max.x, min.y + 5.0),
                ],
                [
                    egui::pos2(min.x, max.y - 5.0),
                    egui::pos2(min.x, max.y),
                    egui::pos2(min.x + 6.0, max.y),
                ],
                [
                    egui::pos2(max.x, max.y - 5.0),
                    max,
                    egui::pos2(max.x - 6.0, max.y),
                ],
            ] {
                painter.line(points.to_vec(), stroke);
            }
        }
    }
}

fn physical_video_rectangle(
    rectangle: egui::Rect,
    pixels_per_point: f32,
) -> Option<VideoRectangle> {
    VideoRectangle::new(
        (rectangle.min.x * pixels_per_point).round() as i32,
        (rectangle.min.y * pixels_per_point).round() as i32,
        (rectangle.width() * pixels_per_point).round() as i32,
        (rectangle.height() * pixels_per_point).round() as i32,
    )
}

fn physical_video_exclusion(
    video_rectangle: egui::Rect,
    popup_rectangle: egui::Rect,
    pixels_per_point: f32,
) -> Option<VideoRectangle> {
    let intersection = video_rectangle.intersect(popup_rectangle);
    if intersection.width() <= 0.0 || intersection.height() <= 0.0 {
        return None;
    }

    let minimum_x =
        ((intersection.min.x - video_rectangle.min.x) * pixels_per_point).floor() as i32;
    let minimum_y =
        ((intersection.min.y - video_rectangle.min.y) * pixels_per_point).floor() as i32;
    let maximum_x = ((intersection.max.x - video_rectangle.min.x) * pixels_per_point).ceil() as i32;
    let maximum_y = ((intersection.max.y - video_rectangle.min.y) * pixels_per_point).ceil() as i32;
    VideoRectangle::new(
        minimum_x,
        minimum_y,
        maximum_x - minimum_x,
        maximum_y - minimum_y,
    )
}

fn adapter_label(adapter: &GstreamerAdapter) -> String {
    format!("{} — {}", adapter.selection(), adapter.name())
}

const fn state_name(state: PlayerState) -> &'static str {
    match state {
        PlayerState::Idle => "Idle",
        PlayerState::Stopped => "Stopped",
        PlayerState::Playing => "Playing",
        PlayerState::Paused => "Paused",
        PlayerState::Failed => "Failed",
    }
}

fn format_optional_hex(value: Option<u32>) -> String {
    value
        .map(|value| format!("0x{value:04X}"))
        .unwrap_or_else(|| "Unavailable".to_owned())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use eframe::egui;

    use super::{format_playback_time, physical_video_exclusion, valid_record_time};

    #[test]
    fn playback_time_uses_clock_format() {
        assert_eq!(format_playback_time(Duration::ZERO), "00:00");
        assert_eq!(format_playback_time(Duration::from_secs(65)), "01:05");
        assert_eq!(format_playback_time(Duration::from_secs(3_661)), "1:01:01");
    }

    #[test]
    fn record_time_requires_hours_minutes_and_seconds() {
        assert!(valid_record_time("00:00:30"));
        assert!(valid_record_time("120:59:59"));
        assert!(!valid_record_time("00:60:00"));
        assert!(!valid_record_time("00:00"));
        assert!(!valid_record_time("not-a-time"));
    }

    #[test]
    fn popup_exclusion_is_relative_to_video_and_dpi_scaled() {
        let video = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(500.0, 300.0));
        let popup = egui::Rect::from_min_max(egui::pos2(150.0, 80.0), egui::pos2(250.0, 160.0));

        let exclusion = physical_video_exclusion(video, popup, 1.5);

        assert_eq!(exclusion, tsan_player::VideoRectangle::new(75, 0, 150, 90));
    }

    #[test]
    fn popup_outside_video_needs_no_exclusion() {
        let video = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(500.0, 300.0));
        let popup = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(40.0, 40.0));

        assert_eq!(physical_video_exclusion(video, popup, 1.0), None);
    }
}
