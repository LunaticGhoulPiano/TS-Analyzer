use std::ops::Range;

use eframe::egui;

const SCROLL_BAR_WIDTH: f32 = 16.0;
const SCROLL_BAR_INNER_MARGIN: f32 = 2.0;
const SCROLL_HANDLE_MIN_LENGTH: f32 = 36.0;
const DEFAULT_ARROW_STEP: f32 = 16.0;
const HELD_SCROLL_SPEED: f32 = 96.0;

#[derive(Clone, Copy)]
enum ArrowDirection {
    Left,
    Right,
    Up,
    Down,
}

impl ArrowDirection {
    fn id_salt(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

/// A reusable scroll area with fixed-width scroll bars, persistent draggable
/// handles and clickable arrow buttons at both ends.
pub struct ArrowScrollArea {
    inner: egui::ScrollArea,
    axes: [bool; 2],
    arrow_step: f32,
}

impl ArrowScrollArea {
    pub fn horizontal() -> Self {
        Self {
            inner: egui::ScrollArea::horizontal(),
            axes: [true, false],
            arrow_step: DEFAULT_ARROW_STEP,
        }
    }

    pub fn vertical() -> Self {
        Self {
            inner: egui::ScrollArea::vertical(),
            axes: [false, true],
            arrow_step: DEFAULT_ARROW_STEP,
        }
    }

    pub fn both() -> Self {
        Self {
            inner: egui::ScrollArea::both(),
            axes: [true, true],
            arrow_step: DEFAULT_ARROW_STEP,
        }
    }

    pub fn id_salt(mut self, id_salt: impl egui::AsIdSalt) -> Self {
        self.inner = self.inner.id_salt(id_salt);
        self
    }

    pub fn max_width(mut self, width: f32) -> Self {
        self.inner = self.inner.max_width(width);
        self
    }

    pub fn max_height(mut self, height: f32) -> Self {
        self.inner = self.inner.max_height(height);
        self
    }

    pub fn min_scrolled_height(mut self, height: f32) -> Self {
        self.inner = self.inner.min_scrolled_height(height);
        self
    }

    pub fn auto_shrink(mut self, auto_shrink: [bool; 2]) -> Self {
        self.inner = self.inner.auto_shrink(auto_shrink);
        self
    }

    pub fn stick_to_bottom(mut self, stick: bool) -> Self {
        self.inner = self.inner.stick_to_bottom(stick);
        self
    }

    pub fn show<R>(self, ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
        let Self {
            inner,
            axes,
            arrow_step,
        } = self;
        let inner = prepare_scroll_area(inner, axes);
        let mut output = inner.show(ui, add_contents);
        add_scroll_bars(ui, &mut output, axes, arrow_step);
        output.inner
    }

    pub fn show_rows<R>(
        self,
        ui: &mut egui::Ui,
        row_height: f32,
        total_rows: usize,
        add_contents: impl FnOnce(&mut egui::Ui, Range<usize>) -> R,
    ) -> R {
        let Self {
            inner,
            axes,
            arrow_step,
        } = self;
        let inner = prepare_scroll_area(inner, axes);
        let mut output = inner.show_rows(ui, row_height, total_rows, add_contents);
        add_scroll_bars(ui, &mut output, axes, arrow_step);
        output.inner
    }
}

fn prepare_scroll_area(inner: egui::ScrollArea, axes: [bool; 2]) -> egui::ScrollArea {
    let mut margin = egui::Margin::ZERO;
    if axes[0] {
        margin.bottom = SCROLL_BAR_WIDTH as i8;
    }
    if axes[1] {
        margin.right = SCROLL_BAR_WIDTH as i8;
    }
    inner
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .content_margin(margin)
}

fn add_scroll_bars<R>(
    ui: &mut egui::Ui,
    output: &mut egui::scroll_area::ScrollAreaOutput<R>,
    axes: [bool; 2],
    arrow_step: f32,
) {
    let maximum = egui::vec2(
        (output.content_size.x - output.inner_rect.width()).max(0.0),
        (output.content_size.y - output.inner_rect.height()).max(0.0),
    );
    let horizontal = axes[0] && maximum.x > 0.5;
    let vertical = axes[1] && maximum.y > 0.5;
    if !horizontal && !vertical {
        return;
    }

    let mut offset = output.state.offset;
    let mut changed = false;
    ui.push_id(output.id.with("scroll-bars"), |ui| {
        if vertical && output.inner_rect.height() >= SCROLL_BAR_WIDTH * 3.0 {
            changed |= vertical_scroll_bar(
                ui,
                output.inner_rect,
                horizontal,
                output.content_size.y,
                &mut offset.y,
                maximum.y,
                arrow_step,
            );
        }
        if horizontal && output.inner_rect.width() >= SCROLL_BAR_WIDTH * 3.0 {
            changed |= horizontal_scroll_bar(
                ui,
                output.inner_rect,
                vertical,
                output.content_size.x,
                &mut offset.x,
                maximum.x,
                arrow_step,
            );
        }
        if horizontal && vertical {
            let corner = egui::Rect::from_min_max(
                output.inner_rect.right_bottom() - egui::vec2(SCROLL_BAR_WIDTH, SCROLL_BAR_WIDTH),
                output.inner_rect.right_bottom(),
            );
            ui.painter()
                .rect_filled(corner, 0.0, ui.visuals().extreme_bg_color);
        }
    });

    if changed {
        output.state.offset = offset;
        output.state.store(ui.ctx(), output.id);
        ui.ctx().request_repaint();
    }
}

fn vertical_scroll_bar(
    ui: &mut egui::Ui,
    viewport: egui::Rect,
    has_horizontal: bool,
    content_length: f32,
    offset: &mut f32,
    maximum: f32,
    arrow_step: f32,
) -> bool {
    let bottom_inset = if has_horizontal {
        SCROLL_BAR_WIDTH
    } else {
        0.0
    };
    let lane = egui::Rect::from_min_max(
        egui::pos2(viewport.right() - SCROLL_BAR_WIDTH, viewport.top()),
        egui::pos2(viewport.right(), viewport.bottom() - bottom_inset),
    );
    let up = egui::Rect::from_min_size(lane.min, egui::vec2(SCROLL_BAR_WIDTH, SCROLL_BAR_WIDTH));
    let down = egui::Rect::from_min_size(
        egui::pos2(lane.left(), lane.bottom() - SCROLL_BAR_WIDTH),
        egui::vec2(SCROLL_BAR_WIDTH, SCROLL_BAR_WIDTH),
    );
    let track = egui::Rect::from_min_max(
        egui::pos2(lane.left(), up.bottom()),
        egui::pos2(lane.right(), down.top()),
    );
    paint_track(ui, track);
    let mut changed = apply_track_and_handle(
        ui,
        track,
        true,
        viewport.height(),
        content_length,
        offset,
        maximum,
    );
    changed |= apply_arrow(ui, up, ArrowDirection::Up, offset, maximum, arrow_step);
    changed |= apply_arrow(ui, down, ArrowDirection::Down, offset, maximum, arrow_step);
    changed
}

fn horizontal_scroll_bar(
    ui: &mut egui::Ui,
    viewport: egui::Rect,
    has_vertical: bool,
    content_length: f32,
    offset: &mut f32,
    maximum: f32,
    arrow_step: f32,
) -> bool {
    let right_inset = if has_vertical { SCROLL_BAR_WIDTH } else { 0.0 };
    let lane = egui::Rect::from_min_max(
        egui::pos2(viewport.left(), viewport.bottom() - SCROLL_BAR_WIDTH),
        egui::pos2(viewport.right() - right_inset, viewport.bottom()),
    );
    let left = egui::Rect::from_min_size(lane.min, egui::vec2(SCROLL_BAR_WIDTH, SCROLL_BAR_WIDTH));
    let right = egui::Rect::from_min_size(
        egui::pos2(lane.right() - SCROLL_BAR_WIDTH, lane.top()),
        egui::vec2(SCROLL_BAR_WIDTH, SCROLL_BAR_WIDTH),
    );
    let track = egui::Rect::from_min_max(
        egui::pos2(left.right(), lane.top()),
        egui::pos2(right.left(), lane.bottom()),
    );
    paint_track(ui, track);
    let mut changed = apply_track_and_handle(
        ui,
        track,
        false,
        viewport.width(),
        content_length,
        offset,
        maximum,
    );
    changed |= apply_arrow(ui, left, ArrowDirection::Left, offset, maximum, arrow_step);
    changed |= apply_arrow(
        ui,
        right,
        ArrowDirection::Right,
        offset,
        maximum,
        arrow_step,
    );
    changed
}

fn paint_track(ui: &egui::Ui, rectangle: egui::Rect) {
    ui.painter()
        .rect_filled(rectangle, 0.0, ui.visuals().extreme_bg_color);
    ui.painter().rect_stroke(
        rectangle,
        0.0,
        ui.visuals().widgets.noninteractive.bg_stroke,
        egui::StrokeKind::Inside,
    );
}

fn apply_track_and_handle(
    ui: &mut egui::Ui,
    track: egui::Rect,
    vertical: bool,
    viewport_length: f32,
    content_length: f32,
    offset: &mut f32,
    maximum: f32,
) -> bool {
    let track_length = if vertical {
        track.height()
    } else {
        track.width()
    };
    if track_length <= 1.0 {
        return false;
    }
    let handle_length = (track_length * viewport_length / content_length.max(viewport_length))
        .clamp(SCROLL_HANDLE_MIN_LENGTH.min(track_length), track_length);
    let travel = (track_length - handle_length).max(0.0);
    let handle_start = if maximum > 0.0 {
        (*offset / maximum).clamp(0.0, 1.0) * travel
    } else {
        0.0
    };
    let handle = if vertical {
        egui::Rect::from_min_size(
            egui::pos2(
                track.left() + SCROLL_BAR_INNER_MARGIN,
                track.top() + handle_start,
            ),
            egui::vec2(track.width() - SCROLL_BAR_INNER_MARGIN * 2.0, handle_length),
        )
    } else {
        egui::Rect::from_min_size(
            egui::pos2(
                track.left() + handle_start,
                track.top() + SCROLL_BAR_INNER_MARGIN,
            ),
            egui::vec2(
                handle_length,
                track.height() - SCROLL_BAR_INNER_MARGIN * 2.0,
            ),
        )
    };

    let axis = if vertical { "vertical" } else { "horizontal" };
    let mut changed = false;
    let page_step = (viewport_length * 0.8).max(DEFAULT_ARROW_STEP);
    let before = if vertical {
        egui::Rect::from_min_max(track.min, egui::pos2(track.right(), handle.top()))
    } else {
        egui::Rect::from_min_max(track.min, egui::pos2(handle.left(), track.bottom()))
    };
    let after = if vertical {
        egui::Rect::from_min_max(egui::pos2(track.left(), handle.bottom()), track.max)
    } else {
        egui::Rect::from_min_max(egui::pos2(handle.right(), track.top()), track.max)
    };
    if before.is_positive()
        && ui
            .interact(
                before,
                ui.id().with((axis, "track-before")),
                egui::Sense::click(),
            )
            .clicked()
    {
        *offset = (*offset - page_step).max(0.0);
        changed = true;
    }
    if after.is_positive()
        && ui
            .interact(
                after,
                ui.id().with((axis, "track-after")),
                egui::Sense::click(),
            )
            .clicked()
    {
        *offset = (*offset + page_step).min(maximum);
        changed = true;
    }

    let response = ui.interact(
        handle,
        ui.id().with((axis, "handle")),
        egui::Sense::click_and_drag(),
    );
    if response.dragged() && travel > 0.0 {
        let drag_delta = if vertical {
            response.drag_delta().y
        } else {
            response.drag_delta().x
        };
        let next = (*offset + drag_delta / travel * maximum).clamp(0.0, maximum);
        changed |= (next - *offset).abs() > f32::EPSILON;
        *offset = next;
    }
    let visuals = ui.style().interact(&response);
    ui.painter()
        .rect_filled(handle, visuals.corner_radius, visuals.bg_fill);
    ui.painter().rect_stroke(
        handle,
        visuals.corner_radius,
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    changed
}

fn apply_arrow(
    ui: &mut egui::Ui,
    rectangle: egui::Rect,
    direction: ArrowDirection,
    offset: &mut f32,
    maximum: f32,
    arrow_step: f32,
) -> bool {
    let towards_start = matches!(direction, ArrowDirection::Left | ArrowDirection::Up);
    let enabled = if towards_start {
        *offset > 0.5
    } else {
        *offset < maximum - 0.5
    };
    let response = ui.interact(
        rectangle,
        ui.id().with(direction.id_salt()),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let visuals = if enabled {
        ui.style().interact(&response)
    } else {
        &ui.visuals().widgets.noninteractive
    };
    ui.painter()
        .rect_filled(rectangle, visuals.corner_radius, visuals.weak_bg_fill);
    ui.painter().rect_stroke(
        rectangle,
        visuals.corner_radius,
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );

    let center = rectangle.center();
    let radius = (rectangle.width().min(rectangle.height()) * 0.22).max(2.5);
    let points = match direction {
        ArrowDirection::Left => vec![
            egui::pos2(center.x + radius * 0.5, center.y - radius),
            egui::pos2(center.x - radius * 0.5, center.y),
            egui::pos2(center.x + radius * 0.5, center.y + radius),
        ],
        ArrowDirection::Right => vec![
            egui::pos2(center.x - radius * 0.5, center.y - radius),
            egui::pos2(center.x + radius * 0.5, center.y),
            egui::pos2(center.x - radius * 0.5, center.y + radius),
        ],
        ArrowDirection::Up => vec![
            egui::pos2(center.x - radius, center.y + radius * 0.5),
            egui::pos2(center.x, center.y - radius * 0.5),
            egui::pos2(center.x + radius, center.y + radius * 0.5),
        ],
        ArrowDirection::Down => vec![
            egui::pos2(center.x - radius, center.y - radius * 0.5),
            egui::pos2(center.x, center.y + radius * 0.5),
            egui::pos2(center.x + radius, center.y - radius * 0.5),
        ],
    };
    ui.painter().add(egui::Shape::line(
        points,
        egui::Stroke::new(1.5, visuals.fg_stroke.color),
    ));

    if !enabled {
        return false;
    }
    let click_delta = if response.clicked() {
        arrow_step.min(maximum)
    } else {
        0.0
    };
    let held_delta = if response.is_pointer_button_down_on() {
        ui.ctx().request_repaint();
        HELD_SCROLL_SPEED * ui.input(|input| input.stable_dt).min(0.05)
    } else {
        0.0
    };
    let direction_sign = if towards_start { -1.0 } else { 1.0 };
    let next = (*offset + direction_sign * (held_delta + click_delta)).clamp(0.0, maximum);
    let changed = (next - *offset).abs() > f32::EPSILON;
    *offset = next;
    changed
}

/// A reusable draggable divider. \`horizontal\` moves left/right and paints a
/// vertical divider; \`vertical\` moves up/down and paints a horizontal divider.
pub struct ResizeHandle<'a> {
    axis: ResizeAxis,
    span: f32,
    thickness: f32,
    inset: f32,
    filled: bool,
    hover_text: Option<&'a str>,
}

#[derive(Clone, Copy)]
enum ResizeAxis {
    Horizontal,
    Vertical,
}

impl<'a> ResizeHandle<'a> {
    pub fn horizontal(span: f32) -> Self {
        Self::new(ResizeAxis::Horizontal, span)
    }

    pub fn vertical(span: f32) -> Self {
        Self::new(ResizeAxis::Vertical, span)
    }

    fn new(axis: ResizeAxis, span: f32) -> Self {
        Self {
            axis,
            span,
            thickness: 10.0,
            inset: 0.0,
            filled: false,
            hover_text: None,
        }
    }

    pub fn thickness(mut self, thickness: f32) -> Self {
        self.thickness = thickness.max(1.0);
        self
    }

    pub fn inset(mut self, inset: f32) -> Self {
        self.inset = inset.max(0.0);
        self
    }

    pub fn filled(mut self, filled: bool) -> Self {
        self.filled = filled;
        self
    }

    pub fn hover_text(mut self, hover_text: &'a str) -> Self {
        self.hover_text = Some(hover_text);
        self
    }

    pub fn show(self, ui: &mut egui::Ui) -> egui::Response {
        let size = match self.axis {
            ResizeAxis::Horizontal => egui::vec2(self.thickness, self.span),
            ResizeAxis::Vertical => egui::vec2(self.span, self.thickness),
        };
        let (rectangle, mut response) = ui.allocate_exact_size(size, egui::Sense::drag());
        let cursor = match self.axis {
            ResizeAxis::Horizontal => egui::CursorIcon::ResizeHorizontal,
            ResizeAxis::Vertical => egui::CursorIcon::ResizeVertical,
        };
        response = response.on_hover_cursor(cursor);
        if let Some(hover_text) = self.hover_text {
            response = response.on_hover_text(hover_text);
        }
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(cursor);
        }

        if self.filled {
            let fill = if response.hovered() || response.dragged() {
                ui.visuals().selection.bg_fill
            } else {
                ui.visuals().widgets.inactive.bg_fill
            };
            ui.painter().rect_filled(rectangle, 0.0, fill);
        }
        let stroke = if response.hovered() || response.dragged() {
            egui::Stroke::new(3.0, ui.visuals().widgets.active.fg_stroke.color)
        } else {
            ui.visuals().widgets.noninteractive.fg_stroke
        };
        match self.axis {
            ResizeAxis::Horizontal => ui.painter().vline(
                rectangle.center().x,
                rectangle.y_range().shrink(self.inset),
                stroke,
            ),
            ResizeAxis::Vertical => ui.painter().hline(
                rectangle.x_range().shrink(self.inset),
                rectangle.center().y,
                stroke,
            ),
        };
        response
    }
}
