use crate::params::{FilterModel, FilterType, LfoWaveform, OscMode, OscWaveform};
use crate::ui::theme::{
    draw_p5_3d_text, font_id, FONT_HEAVY, FONT_IMPACT, P5_BG, P5_BLACK, P5_CARD, P5_CYAN, P5_DIM,
    P5_GOLD, P5_MAGENTA, P5_MUTED, P5_PANEL, P5_RED, P5_RED_DEEP, P5_SURFACE, P5_WHITE,
};
use crate::SharedUiState;
use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, Color32, CornerRadius, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2,
};
use std::f32::consts::{PI, TAU};

// ============================================================================
// Custom P5 Dropdown Overlay State (ZERO stock egui::ComboBox!)
// ============================================================================

#[derive(Clone)]
pub struct ActiveDropdownPopup {
    pub id: String,
    pub anchor: Rect,
    pub items: Vec<String>,
    pub selected: usize,
    pub accent: Color32,
}

#[derive(Default)]
pub struct PopupOverlayState {
    pub active: Option<ActiveDropdownPopup>,
    pub pending_selection: Option<(String, usize)>,
}

impl PopupOverlayState {
    pub fn take_selection(&mut self, id: &str) -> Option<usize> {
        if let Some((sel_id, idx)) = &self.pending_selection {
            if sel_id == id {
                let out = *idx;
                self.pending_selection = None;
                return Some(out);
            }
        }
        None
    }

    pub fn open(
        &mut self,
        id: &str,
        anchor: Rect,
        items: Vec<String>,
        selected: usize,
        accent: Color32,
    ) {
        if let Some(cur) = &self.active {
            if cur.id == id {
                self.active = None;
                return;
            }
        }
        self.active = Some(ActiveDropdownPopup {
            id: id.to_owned(),
            anchor,
            items,
            selected,
            accent,
        });
    }

    /// Call once per frame AFTER all panels are drawn so the custom P5 dropdown floats above everything.
    pub fn render_overlay(&mut self, ctx: &egui::Context) {
        let Some(popup) = self.active.clone() else {
            return;
        };

        let screen = ctx.screen_rect();
        let item_h = 22.0_f32;
        let max_visible = 12usize;
        let visible_rows = popup.items.len().min(max_visible);
        let popup_h = (visible_rows as f32 * item_h + 10.0).min(290.0);
        let popup_w = popup.anchor.width().max(118.0).min(230.0);

        let below_y = popup.anchor.bottom() + 2.0;
        let pos_y = if below_y + popup_h <= screen.bottom() - 6.0 {
            below_y
        } else {
            (popup.anchor.top() - popup_h - 2.0).max(screen.top() + 6.0)
        };
        let pos_x = popup
            .anchor
            .left()
            .min(screen.right() - popup_w - 8.0)
            .max(screen.left() + 6.0);

        let popup_rect = Rect::from_min_size(Pos2::new(pos_x, pos_y), Vec2::new(popup_w, popup_h));
        let mut close_now = false;

        // Close on Escape
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            close_now = true;
        }

        // Close on click outside popup & anchor
        if ctx.input(|i| i.pointer.any_pressed()) {
            if let Some(p) = ctx.input(|i| i.pointer.interact_pos()) {
                if !popup_rect.contains(p) && !popup.anchor.contains(p) {
                    close_now = true;
                }
            }
        }

        egui::Area::new(egui::Id::new("p5_dropdown_overlay_area"))
            .order(egui::Order::Foreground)
            .fixed_pos(popup_rect.min)
            .show(ctx, |ui| {
                let p = ui.painter();
                p.rect_filled(
                    popup_rect.translate(Vec2::new(4.0, 4.0)),
                    CornerRadius::ZERO,
                    P5_BLACK,
                );
                egui::Frame::new()
                    .fill(P5_CARD)
                    .stroke(Stroke::new(2.0_f32, P5_WHITE))
                    .inner_margin(4.0)
                    .show(ui, |ui| {
                        ui.set_width(popup_w - 8.0);
                        egui::ScrollArea::vertical()
                            .max_height(popup_h - 8.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 1.0;
                                for (idx, name) in popup.items.iter().enumerate() {
                                    let is_sel = idx == popup.selected;
                                    let (r, resp) = ui.allocate_exact_size(
                                        Vec2::new(ui.available_width(), item_h),
                                        Sense::click(),
                                    );
                                    let row_p = ui.painter();
                                    let bg = if is_sel {
                                        P5_RED
                                    } else if resp.hovered() {
                                        P5_RED_DEEP
                                    } else {
                                        P5_PANEL
                                    };
                                    row_p.rect_filled(r, CornerRadius::ZERO, bg);
                                    if is_sel || resp.hovered() {
                                        // Tilted diamond bullet
                                        let c = Pos2::new(r.left() + 8.0, r.center().y);
                                        let dia = vec![
                                            Pos2::new(c.x, c.y - 3.5),
                                            Pos2::new(c.x + 3.5, c.y),
                                            Pos2::new(c.x, c.y + 3.5),
                                            Pos2::new(c.x - 3.5, c.y),
                                        ];
                                        row_p.add(Shape::convex_polygon(
                                            dia,
                                            if is_sel { P5_WHITE } else { popup.accent },
                                            Stroke::NONE,
                                        ));
                                    }
                                    row_p.text(
                                        Pos2::new(r.left() + 16.0, r.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        name,
                                        font_id(11.0, FONT_HEAVY),
                                        if is_sel { P5_WHITE } else { P5_WHITE },
                                    );
                                    if resp.clicked() {
                                        self.pending_selection = Some((popup.id.clone(), idx));
                                        close_now = true;
                                    }
                                }
                            });
                    });
            });

        if close_now {
            self.active = None;
        }
    }
}

// ============================================================================
// Cutout Card Frame (Persona 5 Signature Dark Card w/ White Border & 3D Title)
// ============================================================================

pub fn draw_cutout_card(
    ui: &mut egui::Ui,
    title: &str,
    accent: Color32,
    border_color: Color32,
    min_height: f32,
    header_right: impl FnOnce(&mut egui::Ui),
    body: impl FnOnce(&mut egui::Ui),
) {
    let start_pos = ui.cursor().min;
    let avail_w = ui.available_width();
    // Pre-paint subtle hard shadow behind frame
    let shadow_rect = Rect::from_min_size(
        start_pos + Vec2::new(3.0, 3.0),
        Vec2::new(avail_w, min_height),
    );
    ui.painter()
        .rect_filled(shadow_rect, CornerRadius::ZERO, P5_BLACK);

    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(12, 12, 16, 246))
        .stroke(Stroke::new(2.0_f32, border_color))
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_min_height((min_height - 12.0).max(20.0));
            ui.set_max_height((min_height - 12.0).max(20.0));
            ui.horizontal(|ui| {
                let (tag_r, _) = ui.allocate_exact_size(Vec2::new(152.0, 22.0), Sense::hover());
                draw_p5_3d_text(
                    ui.painter(),
                    Pos2::new(tag_r.left() + 2.0, tag_r.center().y),
                    egui::Align2::LEFT_CENTER,
                    title,
                    13.0,
                    accent,
                    P5_WHITE,
                    FONT_IMPACT,
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    header_right(ui);
                });
            });
            ui.add_space(3.0);
            body(ui);
        });
}

// ============================================================================
// Custom P5 270-Degree Knob
// ============================================================================

fn format_param_display<P: Param>(param: &P) -> String {
    let raw = param.to_string();
    if let Some(stripped) = raw.strip_suffix(" kHz Hz") {
        format!("{stripped} kHz")
    } else if let Some(stripped) = raw.strip_suffix(" Hz Hz") {
        format!("{stripped} Hz")
    } else if let Some(stripped) = raw.strip_suffix(" BPM BPM") {
        format!("{stripped} BPM")
    } else {
        raw
    }
}

#[allow(clippy::too_many_arguments)]
pub fn p5_knob_sized<P: Param>(
    ui: &mut egui::Ui,
    label: &str,
    param: &P,
    setter: &ParamSetter,
    ui_state: &SharedUiState,
    accent: Color32,
    cell_width: f32,
    knob_diam: f32,
    bipolar: bool,
    midi_learn: &mut Option<String>,
) {
    let total_h = knob_diam + 26.0;
    ui.allocate_ui_with_layout(
        Vec2::new(cell_width, total_h),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let (rect, resp) =
                ui.allocate_exact_size(Vec2::splat(knob_diam), Sense::click_and_drag());

            if resp.clicked() {
                if let Some(target_out) = midi_learn.as_mut() {
                    *target_out = format!("LEARN: {}", param.name());
                    ui_state.set_last_param_label(target_out);
                }
            }
            if resp.drag_started() {
                setter.begin_set_parameter(param);
            }
            if resp.dragged() {
                let fine = ui.input(|i| i.modifiers.shift);
                let sens = if fine { 0.0012 } else { 0.0052 };
                let delta = -resp.drag_delta().y * sens;
                let cur = param.unmodulated_normalized_value();
                let next = (cur + delta).clamp(0.0, 1.0);
                setter.set_parameter(param, param.preview_plain(next));
                let val_disp = format_param_display(param);
                ui_state.set_last_param_label_with_val(
                    &format!("{}: {}", param.name(), val_disp),
                    next,
                );
            }
            if resp.drag_stopped() {
                setter.end_set_parameter(param);
            }
            if resp.hovered() {
                let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll.abs() > 0.1 {
                    let step = scroll.signum() * 0.025;
                    let next = (param.unmodulated_normalized_value() + step).clamp(0.0, 1.0);
                    setter.begin_set_parameter(param);
                    setter.set_parameter(param, param.preview_plain(next));
                    setter.end_set_parameter(param);
                    let val_disp = format_param_display(param);
                    ui_state.set_last_param_label_with_val(
                        &format!("{}: {}", param.name(), val_disp),
                        next,
                    );
                } else {
                    ui_state.set_last_param_label(&format!(
                        "{}: {}",
                        param.name(),
                        format_param_display(param)
                    ));
                }
            }
            if resp.double_clicked() {
                setter.begin_set_parameter(param);
                setter.set_parameter(param, param.default_plain_value());
                setter.end_set_parameter(param);
                ui_state.set_last_param_label(&format!(
                    "{}: {}",
                    param.name(),
                    format_param_display(param)
                ));
            }

            let norm = param.modulated_normalized_value().clamp(0.0, 1.0);
            let p = ui.painter();
            let center = rect.center();
            let r = knob_diam * 0.41;

            // Hard drop shadow + obsidian cap + crisp white rim
            p.circle_filled(center + Vec2::new(2.0, 2.0), r, P5_BLACK);
            p.circle_filled(center, r, P5_SURFACE);
            p.circle_stroke(
                center,
                r,
                Stroke::new(
                    if resp.hovered() || resp.dragged() { 1.8_f32 } else { 1.4_f32 },
                    if resp.dragged() { accent } else { P5_WHITE },
                ),
            );

            // 270-degree background arc
            let start_ang = 0.75 * PI;
            let sweep = 1.5 * PI;
            let arc_r = r + 3.0;
            let bg_pts: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let a = start_ang + (i as f32 / 24.0) * sweep;
                    Pos2::new(center.x + a.cos() * arc_r, center.y + a.sin() * arc_r)
                })
                .collect();
            p.add(Shape::line(
                bg_pts,
                Stroke::new(2.8_f32, Color32::from_rgb(48, 48, 60)),
            ));

            // Active colored arc (unipolar or bipolar from 12-o'clock)
            let (a0, a1) = if bipolar {
                let mid = start_ang + 0.5 * sweep;
                let cur = start_ang + norm * sweep;
                if cur >= mid { (mid, cur) } else { (cur, mid) }
            } else {
                (start_ang, start_ang + norm * sweep)
            };
            let span = (a1 - a0).abs();
            if span > 0.01 {
                let steps = ((span / sweep * 24.0).ceil() as usize).max(2);
                let fg_pts: Vec<Pos2> = (0..=steps)
                    .map(|i| {
                        let a = a0 + (i as f32 / steps as f32) * (a1 - a0);
                        Pos2::new(center.x + a.cos() * arc_r, center.y + a.sin() * arc_r)
                    })
                    .collect();
                p.add(Shape::line(fg_pts, Stroke::new(3.1_f32, accent)));
            }

            // Crisp white pointer tick
            let end_ang = start_ang + norm * sweep;
            let ptr_start = Pos2::new(
                center.x + end_ang.cos() * (r * 0.25),
                center.y + end_ang.sin() * (r * 0.25),
            );
            let ptr_end = Pos2::new(
                center.x + end_ang.cos() * (r - 2.2),
                center.y + end_ang.sin() * (r - 2.2),
            );
            p.line_segment([ptr_start, ptr_end], Stroke::new(2.1_f32, P5_WHITE));

            // Cutout label + value readout
            let (lbl_r, _) = ui.allocate_exact_size(Vec2::new(cell_width, 13.0), Sense::hover());
            let lp = ui.painter();
            lp.rect_filled(
                Rect::from_center_size(lbl_r.center(), Vec2::new((cell_width - 4.0).max(24.0), 12.0)),
                CornerRadius::ZERO,
                P5_BLACK,
            );
            lp.text(
                lbl_r.center(),
                egui::Align2::CENTER_CENTER,
                label,
                font_id(9.5, FONT_IMPACT),
                P5_WHITE,
            );

            let (val_r, _) = ui.allocate_exact_size(Vec2::new(cell_width, 11.0), Sense::hover());
            ui.painter().text(
                val_r.center(),
                egui::Align2::CENTER_CENTER,
                format_param_display(param),
                font_id(8.5, FONT_HEAVY),
                accent,
            );
        },
    );
}

#[allow(clippy::too_many_arguments)]
pub fn p5_knob<P: Param>(
    ui: &mut egui::Ui,
    label: &str,
    param: &P,
    setter: &ParamSetter,
    ui_state: &SharedUiState,
    accent: Color32,
    width: f32,
    midi_learn: &mut Option<String>,
) {
    p5_knob_sized(
        ui, label, param, setter, ui_state, accent, width, 34.0, false, midi_learn,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn p5_knob_mini<P: Param>(
    ui: &mut egui::Ui,
    label: &str,
    param: &P,
    setter: &ParamSetter,
    ui_state: &SharedUiState,
    accent: Color32,
    width: f32,
    midi_learn: &mut Option<String>,
) {
    p5_knob_sized(
        ui, label, param, setter, ui_state, accent, width, 28.0, false, midi_learn,
    );
}

// ============================================================================
// Custom P5 Horizontal Sliders (Unipolar & Bipolar — ZERO ParamSlider!)
// ============================================================================

#[allow(clippy::too_many_arguments)]
pub fn p5_slider<P: Param>(
    ui: &mut egui::Ui,
    label: &str,
    param: &P,
    setter: &ParamSetter,
    ui_state: &SharedUiState,
    accent: Color32,
    width: f32,
    height: f32,
    bipolar: bool,
) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());

    if resp.drag_started() {
        setter.begin_set_parameter(param);
    }
    if resp.dragged() || resp.clicked() {
        if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
            let norm = ((pos.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
            if resp.clicked() && !resp.dragged() {
                setter.begin_set_parameter(param);
                setter.set_parameter(param, param.preview_plain(norm));
                setter.end_set_parameter(param);
            } else {
                setter.set_parameter(param, param.preview_plain(norm));
            }
            ui_state.set_last_param_label_with_val(
                &format!("{}: {}", param.name(), format_param_display(param)),
                norm,
            );
        }
    }
    if resp.drag_stopped() {
        setter.end_set_parameter(param);
    }
    if resp.double_clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, param.default_plain_value());
        setter.end_set_parameter(param);
        ui_state.set_last_param_label(&format!(
            "{}: {}",
            param.name(),
            format_param_display(param)
        ));
    }

    let norm = param.modulated_normalized_value().clamp(0.0, 1.0);
    let p = ui.painter();

    p.rect_filled(rect.translate(Vec2::new(2.0, 2.0)), CornerRadius::ZERO, P5_BLACK);
    p.rect_filled(rect, CornerRadius::ZERO, P5_SURFACE);

    let inner = rect.shrink(1.5);
    if bipolar {
        let mid_x = inner.center().x;
        let cur_x = inner.left() + norm * inner.width();
        let fill_r = Rect::from_min_max(
            Pos2::new(mid_x.min(cur_x), inner.top()),
            Pos2::new(mid_x.max(cur_x), inner.bottom()),
        );
        let bar_col = if cur_x >= mid_x { accent } else { P5_CYAN };
        p.rect_filled(fill_r, CornerRadius::ZERO, bar_col);
        p.line_segment(
            [Pos2::new(mid_x, inner.top()), Pos2::new(mid_x, inner.bottom())],
            Stroke::new(1.5_f32, P5_WHITE),
        );
    } else {
        let fill_r = Rect::from_min_size(inner.min, Vec2::new(inner.width() * norm, inner.height()));
        p.rect_filled(fill_r, CornerRadius::ZERO, accent);
    }

    p.rect_stroke(
        rect,
        CornerRadius::ZERO,
        Stroke::new(1.4_f32, if resp.hovered() { P5_WHITE } else { P5_DIM }),
        StrokeKind::Inside,
    );

    let val_str = format_param_display(param);
    let outline_offs = [
        Vec2::new(-1.0, 0.0),
        Vec2::new(1.0, 0.0),
        Vec2::new(0.0, -1.0),
        Vec2::new(0.0, 1.0),
        Vec2::new(1.2, 1.2),
    ];

    if !label.is_empty() {
        let lpos = Pos2::new(rect.left() + 5.0, rect.center().y);
        let lfont = font_id(9.5, FONT_IMPACT);
        for off in outline_offs {
            p.text(lpos + off, egui::Align2::LEFT_CENTER, label, lfont.clone(), P5_BLACK);
        }
        p.text(lpos, egui::Align2::LEFT_CENTER, label, lfont, P5_WHITE);
    }
    let vpos = Pos2::new(rect.right() - 5.0, rect.center().y);
    let vfont = font_id(9.0, FONT_HEAVY);
    for off in outline_offs {
        p.text(vpos + off, egui::Align2::RIGHT_CENTER, &val_str, vfont.clone(), P5_BLACK);
    }
    p.text(vpos, egui::Align2::RIGHT_CENTER, val_str, vfont, P5_WHITE);
}

// ============================================================================
// Custom P5 Vertical Envelope Fader
// ============================================================================

#[allow(clippy::too_many_arguments)]
pub fn p5_v_fader<P: Param>(
    ui: &mut egui::Ui,
    label: &str,
    param: &P,
    setter: &ParamSetter,
    ui_state: &SharedUiState,
    accent: Color32,
    width: f32,
    track_h: f32,
) {
    ui.allocate_ui_with_layout(
        Vec2::new(width, track_h + 28.0),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let (rect, resp) =
                ui.allocate_exact_size(Vec2::new(width - 4.0, track_h), Sense::click_and_drag());

            if resp.drag_started() {
                setter.begin_set_parameter(param);
            }
            if resp.dragged() || resp.clicked() {
                if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                    let norm = (1.0 - (pos.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
                    if resp.clicked() && !resp.dragged() {
                        setter.begin_set_parameter(param);
                        setter.set_parameter(param, param.preview_plain(norm));
                        setter.end_set_parameter(param);
                    } else {
                        setter.set_parameter(param, param.preview_plain(norm));
                    }
                    ui_state.set_last_param_label_with_val(
                        &format!("{}: {}", param.name(), param),
                        norm,
                    );
                }
            }
            if resp.drag_stopped() {
                setter.end_set_parameter(param);
            }
            if resp.double_clicked() {
                setter.begin_set_parameter(param);
                setter.set_parameter(param, param.default_plain_value());
                setter.end_set_parameter(param);
            }

            let norm = param.modulated_normalized_value().clamp(0.0, 1.0);
            let p = ui.painter();

            // Slot channel
            let slot_w = 8.0;
            let slot = Rect::from_center_size(rect.center(), Vec2::new(slot_w, rect.height() - 4.0));
            p.rect_filled(slot, CornerRadius::ZERO, P5_BLACK);
            p.rect_stroke(
                slot,
                CornerRadius::ZERO,
                Stroke::new(1.0_f32, P5_DIM),
                StrokeKind::Inside,
            );

            // Active fill from bottom
            let fill_h = (slot.height() - 2.0) * norm;
            let fill_r = Rect::from_min_max(
                Pos2::new(slot.left() + 1.0, slot.bottom() - 1.0 - fill_h),
                Pos2::new(slot.right() - 1.0, slot.bottom() - 1.0),
            );
            p.rect_filled(fill_r, CornerRadius::ZERO, accent);

            // Crisp P5 fader cap
            let cap_y = slot.bottom() - norm * slot.height();
            let cap = Rect::from_center_size(
                Pos2::new(rect.center().x, cap_y.clamp(rect.top() + 5.0, rect.bottom() - 5.0)),
                Vec2::new(rect.width().min(22.0), 8.0),
            );
            p.rect_filled(cap.translate(Vec2::new(1.5, 1.5)), CornerRadius::ZERO, P5_BLACK);
            p.rect(
                cap,
                CornerRadius::ZERO,
                P5_WHITE,
                Stroke::new(1.5_f32, P5_BLACK),
                StrokeKind::Inside,
            );
            p.line_segment(
                [
                    Pos2::new(cap.left() + 3.0, cap.center().y),
                    Pos2::new(cap.right() - 3.0, cap.center().y),
                ],
                Stroke::new(2.0_f32, accent),
            );

            // Label
            let (lbl_r, _) = ui.allocate_exact_size(Vec2::new(width, 12.0), Sense::hover());
            ui.painter().text(
                lbl_r.center(),
                egui::Align2::CENTER_CENTER,
                label,
                font_id(9.5, FONT_IMPACT),
                P5_WHITE,
            );
            let (val_r, _) = ui.allocate_exact_size(Vec2::new(width, 11.0), Sense::hover());
            ui.painter().text(
                val_r.center(),
                egui::Align2::CENTER_CENTER,
                format_param_display(param),
                font_id(8.0, FONT_HEAVY),
                P5_MUTED,
            );
        },
    );
}

// ============================================================================
// Custom P5 Dropdown (NEVER stock egui::ComboBox!)
// ============================================================================

#[allow(clippy::too_many_arguments)]
pub fn p5_enum_dropdown<T: Enum + PartialEq + Copy + 'static>(
    ui: &mut egui::Ui,
    id: &str,
    param: &EnumParam<T>,
    setter: &ParamSetter,
    ui_state: &SharedUiState,
    popups: &mut PopupOverlayState,
    width: f32,
    height: f32,
    accent: Color32,
) -> bool {
    let variants = T::variants();
    let current = param.value();
    let cur_idx = current.to_index();
    let cur_name = variants.get(cur_idx).copied().unwrap_or("--");

    let mut changed = false;
    if let Some(sel) = popups.take_selection(id) {
        if sel < variants.len() {
            let next_val = T::from_index(sel);
            setter.begin_set_parameter(param);
            setter.set_parameter(param, next_val);
            setter.end_set_parameter(param);
            ui_state.set_last_param_label(&format!("{}: {}", param.name(), variants[sel]));
            changed = true;
        }
    }

    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let is_open = popups.active.as_ref().map(|a| a.id == id).unwrap_or(false);

    if resp.clicked() {
        popups.open(
            id,
            rect,
            variants.iter().map(|s| (*s).to_owned()).collect(),
            cur_idx,
            accent,
        );
    }

    let p = ui.painter();
    p.rect_filled(rect.translate(Vec2::new(2.0, 2.0)), CornerRadius::ZERO, P5_BLACK);
    p.rect(
        rect,
        CornerRadius::ZERO,
        if is_open {
            P5_RED_DEEP
        } else if resp.hovered() {
            Color32::from_rgb(38, 16, 22)
        } else {
            P5_SURFACE
        },
        Stroke::new(
            1.5_f32,
            if is_open {
                accent
            } else if resp.hovered() {
                P5_WHITE
            } else {
                P5_DIM
            },
        ),
        StrokeKind::Inside,
    );

    // Clip label safely so it never overlaps the chevron
    let text_clip = Rect::from_min_max(
        Pos2::new(rect.left() + 5.0, rect.top()),
        Pos2::new(rect.right() - 16.0, rect.bottom()),
    );
    ui.painter_at(text_clip).text(
        Pos2::new(rect.left() + 6.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        cur_name,
        font_id(10.0, FONT_HEAVY),
        P5_WHITE,
    );

    // Custom P5 chevron triangle
    let cx = rect.right() - 9.0;
    let cy = rect.center().y;
    let tri = vec![
        Pos2::new(cx - 3.5, cy - 2.0),
        Pos2::new(cx + 3.5, cy - 2.0),
        Pos2::new(cx, cy + 3.0),
    ];
    p.add(Shape::convex_polygon(tri, accent, Stroke::NONE));

    changed
}

#[allow(clippy::too_many_arguments)]
pub fn p5_string_dropdown(
    ui: &mut egui::Ui,
    id: &str,
    items: &[&str],
    cur_idx: usize,
    popups: &mut PopupOverlayState,
    width: f32,
    height: f32,
    accent: Color32,
) -> Option<usize> {
    let selected = popups.take_selection(id);
    let cur_name = items.get(cur_idx).copied().unwrap_or("--");

    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let is_open = popups.active.as_ref().map(|a| a.id == id).unwrap_or(false);

    if resp.clicked() {
        popups.open(
            id,
            rect,
            items.iter().map(|s| (*s).to_owned()).collect(),
            cur_idx,
            accent,
        );
    }

    let p = ui.painter();
    p.rect_filled(rect.translate(Vec2::new(2.0, 2.0)), CornerRadius::ZERO, P5_BLACK);
    p.rect(
        rect,
        CornerRadius::ZERO,
        if is_open {
            P5_RED_DEEP
        } else if resp.hovered() {
            Color32::from_rgb(38, 16, 22)
        } else {
            P5_SURFACE
        },
        Stroke::new(
            1.5_f32,
            if is_open {
                accent
            } else if resp.hovered() {
                P5_WHITE
            } else {
                P5_DIM
            },
        ),
        StrokeKind::Inside,
    );

    let text_clip = Rect::from_min_max(
        Pos2::new(rect.left() + 5.0, rect.top()),
        Pos2::new(rect.right() - 16.0, rect.bottom()),
    );
    ui.painter_at(text_clip).text(
        Pos2::new(rect.left() + 6.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        cur_name,
        font_id(10.0, FONT_HEAVY),
        P5_WHITE,
    );

    let cx = rect.right() - 9.0;
    let cy = rect.center().y;
    let tri = vec![
        Pos2::new(cx - 3.5, cy - 2.0),
        Pos2::new(cx + 3.5, cy - 2.0),
        Pos2::new(cx, cy + 3.0),
    ];
    p.add(Shape::convex_polygon(tri, accent, Stroke::NONE));

    selected
}

// ============================================================================
// Custom P5 Buttons, Toggle Pills, Diamond-LED FX Pills & Steppers
// ============================================================================

pub fn p5_button(
    ui: &mut egui::Ui,
    label: &str,
    width: f32,
    height: f32,
    active: bool,
    accent: Color32,
    enabled: bool,
) -> bool {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), sense);
    let p = ui.painter();

    p.rect_filled(rect.translate(Vec2::new(2.0, 2.0)), CornerRadius::ZERO, P5_BLACK);

    let (bg, fg, border) = if !enabled {
        (Color32::from_rgb(18, 18, 22), Color32::from_rgb(72, 72, 82), Color32::from_rgb(45, 45, 55))
    } else if active {
        (P5_WHITE, P5_BLACK, P5_BLACK)
    } else if resp.hovered() {
        (P5_RED, P5_WHITE, P5_WHITE)
    } else {
        (P5_SURFACE, P5_WHITE, P5_WHITE)
    };

    p.rect(
        rect,
        CornerRadius::ZERO,
        bg,
        Stroke::new(1.5_f32, border),
        StrokeKind::Inside,
    );
    if active && enabled {
        // Accent bottom bar on active tab/pill
        p.rect_filled(
            Rect::from_min_max(
                Pos2::new(rect.left() + 1.5, rect.bottom() - 3.5),
                Pos2::new(rect.right() - 1.5, rect.bottom() - 1.5),
            ),
            CornerRadius::ZERO,
            accent,
        );
    }

    p.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        font_id(10.5, FONT_IMPACT),
        fg,
    );

    enabled && resp.clicked()
}

pub fn p5_toggle_pill(
    ui: &mut egui::Ui,
    label: &str,
    param: &BoolParam,
    setter: &ParamSetter,
    ui_state: &SharedUiState,
    width: f32,
    height: f32,
    accent: Color32,
) {
    let active = param.value();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    if resp.clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, !active);
        setter.end_set_parameter(param);
        ui_state.set_last_param_label(&format!(
            "{}: {}",
            param.name(),
            if !active { "ON" } else { "OFF" }
        ));
    }

    let p = ui.painter();
    p.rect_filled(rect.translate(Vec2::new(2.0, 2.0)), CornerRadius::ZERO, P5_BLACK);

    let bg = if active {
        accent
    } else if resp.hovered() {
        Color32::from_rgb(42, 16, 24)
    } else {
        P5_SURFACE
    };
    let fg = if active && (accent == P5_WHITE || accent == P5_GOLD || accent == P5_CYAN) {
        P5_BLACK
    } else if active {
        P5_WHITE
    } else {
        P5_MUTED
    };
    let border = if active { P5_WHITE } else { P5_DIM };

    p.rect(
        rect,
        CornerRadius::ZERO,
        bg,
        Stroke::new(1.5_f32, border),
        StrokeKind::Inside,
    );
    p.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        font_id(10.0, FONT_IMPACT),
        fg,
    );
}

/// Phantom Voicer signature diamond-LED pill (used for FX MUTE, EQ, WARM/SOFT/BOOST, ARP ON).
pub fn p5_diamond_pill(
    ui: &mut egui::Ui,
    label: &str,
    param: &BoolParam,
    setter: &ParamSetter,
    ui_state: &SharedUiState,
    width: f32,
    height: f32,
    accent: Color32,
    invert_display: bool,
) {
    let raw = param.value();
    let lit = if invert_display { !raw } else { raw };

    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    if resp.clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, !raw);
        setter.end_set_parameter(param);
        ui_state.set_last_param_label(&format!(
            "{}: {}",
            param.name(),
            if !raw { "ON" } else { "OFF" }
        ));
    }

    let p = ui.painter();
    p.rect_filled(rect.translate(Vec2::new(2.0, 2.0)), CornerRadius::ZERO, P5_BLACK);
    let bg = if lit {
        P5_WHITE
    } else if resp.hovered() {
        Color32::from_rgb(38, 16, 22)
    } else {
        P5_SURFACE
    };
    p.rect(
        rect,
        CornerRadius::ZERO,
        bg,
        Stroke::new(1.6_f32, if lit { P5_BLACK } else { P5_DIM }),
        StrokeKind::Inside,
    );

    let c = Pos2::new(rect.left() + 10.0, rect.center().y);
    let dia = vec![
        Pos2::new(c.x, c.y - 4.5),
        Pos2::new(c.x + 4.5, c.y),
        Pos2::new(c.x, c.y + 4.5),
        Pos2::new(c.x - 4.5, c.y),
    ];
    p.add(Shape::convex_polygon(
        dia,
        if lit { accent } else { Color32::from_rgb(52, 52, 62) },
        Stroke::new(1.1_f32, P5_BLACK),
    ));

    p.text(
        Pos2::new(rect.left() + 19.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        font_id(10.5, FONT_IMPACT),
        if lit { P5_BLACK } else { P5_MUTED },
    );
}

pub fn p5_int_stepper(
    ui: &mut egui::Ui,
    prefix: &str,
    param: &IntParam,
    setter: &ParamSetter,
    ui_state: &SharedUiState,
    width: f32,
    height: f32,
    accent: Color32,
) {
    let cur = param.value();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());
    let btn_w = 18.0_f32.min(width * 0.25);
    let left_r = Rect::from_min_size(rect.min, Vec2::new(btn_w, height));
    let right_r = Rect::from_min_size(Pos2::new(rect.right() - btn_w, rect.top()), Vec2::new(btn_w, height));

    let step_by = |delta: i32| {
        let next = cur.saturating_add(delta);
        setter.begin_set_parameter(param);
        setter.set_parameter(param, next);
        setter.end_set_parameter(param);
        ui_state.set_last_param_label(&format!("{}: {}", param.name(), param));
    };

    if resp.clicked() {
        if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
            if left_r.contains(pos) {
                step_by(-1);
            } else if right_r.contains(pos) {
                step_by(1);
            }
        }
    }
    if resp.double_clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, param.default_plain_value());
        setter.end_set_parameter(param);
    }
    if resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.5 {
            step_by(if scroll > 0.0 { 1 } else { -1 });
        }
    }

    let p = ui.painter();
    p.rect_filled(rect.translate(Vec2::new(2.0, 2.0)), CornerRadius::ZERO, P5_BLACK);
    p.rect(
        rect,
        CornerRadius::ZERO,
        P5_SURFACE,
        Stroke::new(1.4_f32, if resp.hovered() { P5_WHITE } else { P5_DIM }),
        StrokeKind::Inside,
    );
    p.rect_filled(left_r.shrink(1.5), CornerRadius::ZERO, P5_PANEL);
    p.rect_filled(right_r.shrink(1.5), CornerRadius::ZERO, P5_PANEL);
    p.text(left_r.center(), egui::Align2::CENTER_CENTER, "-", font_id(12.0, FONT_HEAVY), accent);
    p.text(right_r.center(), egui::Align2::CENTER_CENTER, "+", font_id(12.0, FONT_HEAVY), accent);

    let text = if prefix.is_empty() {
        param.to_string()
    } else {
        format!("{prefix} {param}")
    };
    p.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        font_id(10.0, FONT_HEAVY),
        P5_WHITE,
    );
}

/// Custom vertical Pitch / Mod Wheel
pub fn p5_wheel(
    ui: &mut egui::Ui,
    label: &str,
    param: &FloatParam,
    setter: &ParamSetter,
    ui_state: &SharedUiState,
    width: f32,
    height: f32,
    accent: Color32,
    spring_center: bool,
) {
    ui.allocate_ui_with_layout(
        Vec2::new(width, height + 14.0),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let (rect, resp) =
                ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());

            if resp.drag_started() {
                setter.begin_set_parameter(param);
            }
            if resp.dragged() {
                if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                    let norm = (1.0 - (pos.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
                    setter.set_parameter(param, param.preview_plain(norm));
                    ui_state.set_last_param_label_with_val(
                        &format!("{}: {}", param.name(), param),
                        norm,
                    );
                }
            }
            if resp.drag_stopped() {
                if spring_center {
                    setter.set_parameter(param, param.default_plain_value());
                }
                setter.end_set_parameter(param);
            }
            if resp.double_clicked() {
                setter.begin_set_parameter(param);
                setter.set_parameter(param, param.default_plain_value());
                setter.end_set_parameter(param);
            }

            let norm = param.modulated_normalized_value().clamp(0.0, 1.0);
            let p = ui.painter();
            p.rect_filled(rect.translate(Vec2::new(2.0, 2.0)), CornerRadius::ZERO, P5_BLACK);
            p.rect(
                rect,
                CornerRadius::ZERO,
                P5_BG,
                Stroke::new(1.5_f32, P5_WHITE),
                StrokeKind::Inside,
            );

            // Glow fill
            let inner = rect.shrink(2.0);
            if spring_center {
                let mid_y = inner.center().y;
                let cur_y = inner.bottom() - norm * inner.height();
                let fr = Rect::from_min_max(
                    Pos2::new(inner.left(), mid_y.min(cur_y)),
                    Pos2::new(inner.right(), mid_y.max(cur_y)),
                );
                p.rect_filled(fr, CornerRadius::ZERO, accent);
                p.line_segment(
                    [Pos2::new(inner.left(), mid_y), Pos2::new(inner.right(), mid_y)],
                    Stroke::new(1.0_f32, P5_DIM),
                );
            } else {
                let fill_h = inner.height() * norm;
                let fr = Rect::from_min_max(
                    Pos2::new(inner.left(), inner.bottom() - fill_h),
                    inner.max,
                );
                p.rect_filled(fr, CornerRadius::ZERO, accent);
            }

            // Thumb rib bar
            let thumb_y = (inner.bottom() - norm * inner.height())
                .clamp(inner.top() + 4.0, inner.bottom() - 4.0);
            let thumb = Rect::from_center_size(
                Pos2::new(inner.center().x, thumb_y),
                Vec2::new(inner.width(), 7.0),
            );
            p.rect(
                thumb,
                CornerRadius::ZERO,
                P5_WHITE,
                Stroke::new(1.4_f32, P5_BLACK),
                StrokeKind::Inside,
            );

            let (lr, _) = ui.allocate_exact_size(Vec2::new(width, 12.0), Sense::hover());
            ui.painter().text(
                lr.center(),
                egui::Align2::CENTER_CENTER,
                label,
                font_id(9.0, FONT_IMPACT),
                P5_WHITE,
            );
        },
    );
}

// ============================================================================
// DSP Visualizers (Oscillator, Dual Filter, Envelope, LFO, 3-Band EQ, Meter)
// ============================================================================

pub fn draw_stereo_meter(painter: &egui::Painter, rect: Rect, peak_l: f32, peak_r: f32) {
    painter.rect_filled(rect, CornerRadius::ZERO, P5_BG);
    painter.rect_stroke(
        rect,
        CornerRadius::ZERO,
        Stroke::new(1.4_f32, P5_WHITE),
        StrokeKind::Inside,
    );

    let segments = 14usize;
    let inner = rect.shrink(2.5);
    let ch_h = (inner.height() - 2.0) * 0.5;
    let seg_w = (inner.width() - (segments as f32 - 1.0) * 1.5) / segments as f32;

    for (ch, level) in [peak_l, peak_r].iter().enumerate() {
        let db = if *level > 1e-5 {
            (20.0 * level.log10()).clamp(-48.0, 3.0)
        } else {
            -48.0
        };
        let norm = ((db + 48.0) / 51.0).clamp(0.0, 1.0);
        let lit_segs = (norm * segments as f32).round() as usize;
        let y0 = inner.top() + ch as f32 * (ch_h + 2.0);
        for s in 0..segments {
            let x0 = inner.left() + s as f32 * (seg_w + 1.5);
            let sr = Rect::from_min_size(Pos2::new(x0, y0), Vec2::new(seg_w.max(2.0), ch_h.max(2.0)));
            let col = if s < lit_segs {
                if s >= segments - 2 {
                    P5_RED
                } else if s >= segments - 5 {
                    P5_GOLD
                } else {
                    P5_CYAN
                }
            } else {
                Color32::from_rgb(30, 30, 38)
            };
            painter.rect_filled(sr, CornerRadius::ZERO, col);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw_osc_waveform_plot(
    painter: &egui::Painter,
    rect: Rect,
    mode: OscMode,
    wave: OscWaveform,
    ctrl_a: f32,
    ctrl_b: f32,
    wt_mix: f32,
    phase: f32,
) {
    painter.rect_filled(rect.translate(Vec2::new(2.0, 2.0)), CornerRadius::ZERO, P5_BLACK);
    painter.rect_filled(rect, CornerRadius::ZERO, P5_BG);

    // Subtle grid
    let mid_y = rect.center().y;
    painter.line_segment(
        [Pos2::new(rect.left() + 2.0, mid_y), Pos2::new(rect.right() - 2.0, mid_y)],
        Stroke::new(1.0_f32, Color32::from_rgb(36, 36, 48)),
    );
    painter.line_segment(
        [Pos2::new(rect.center().x, rect.top() + 2.0), Pos2::new(rect.center().x, rect.bottom() - 2.0)],
        Stroke::new(1.0_f32, Color32::from_rgb(36, 36, 48)),
    );

    let n = 64usize;
    let inner = rect.shrink(4.0);
    let mut pts = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / (n - 1) as f32;
        let ph = (t + phase) % 1.0;

        let base = match wave {
            OscWaveform::Sin => (ph * TAU).sin(),
            OscWaveform::Tri => 1.0 - 4.0 * (ph - 0.5).abs(),
            OscWaveform::Saw => 2.0 * ph - 1.0,
            OscWaveform::Square => if ph < 0.5 { 1.0 } else { -1.0 },
            OscWaveform::Pulse => if ph < (0.15 + ctrl_a * 0.7) { 1.0 } else { -1.0 },
            OscWaveform::SuperSaw => {
                ((2.0 * ph - 1.0) + 0.6 * (2.0 * ((ph * 1.03) % 1.0) - 1.0) + 0.6 * (2.0 * ((ph * 0.97) % 1.0) - 1.0)) * 0.45
            }
            OscWaveform::Organ => {
                ((ph * TAU).sin() + 0.6 * (ph * TAU * 2.0).sin() + 0.35 * (ph * TAU * 4.0).sin()) * 0.52
            }
            OscWaveform::Bell => (ph * TAU).sin() * 0.55 + (ph * TAU * 3.5).sin() * 0.45,
            OscWaveform::Formant | OscWaveform::Choir => {
                (ph * TAU).sin() * 0.5 + (ph * TAU * (3.0 + ctrl_a * 3.0)).sin() * 0.5
            }
            OscWaveform::Metallic => (ph * TAU * 2.0).sin() * (ph * TAU * 5.3).cos(),
            OscWaveform::Acid => ((2.0 * ph - 1.0) * 1.8).tanh(),
            OscWaveform::Bass | OscWaveform::Sub => (ph * TAU).sin() * 0.75 + (2.0 * ph - 1.0) * 0.25,
            OscWaveform::Strings => (2.0 * ph - 1.0) * 0.6 + (ph * TAU * 2.0).sin() * 0.4,
            OscWaveform::Hollow => (ph * TAU).sin() * 0.6 + (ph * TAU * 3.0).sin() * 0.4,
        };

        let y_val = match mode {
            OscMode::Classic => {
                let saw = 2.0 * ph - 1.0;
                let pw = if ph < (0.1 + ctrl_a * 0.8) { 1.0 } else { -1.0 };
                let classic = saw * (1.0 - ctrl_b) + pw * ctrl_b;
                classic * (1.0 - wt_mix) + base * wt_mix
            }
            OscMode::Noise => {
                let h = ((i as u32).wrapping_mul(2654435761) ^ 0x5A17_93F1) as f32 / u32::MAX as f32;
                (h * 2.0 - 1.0) * (0.4 + 0.6 * ctrl_a)
            }
            OscMode::FM | OscMode::HardFM => {
                let idx = (ctrl_b * 4.5).max(0.2);
                let ratio = 1.0 + (ctrl_a * 5.0).round();
                let fm = (ph * TAU + idx * (ph * TAU * ratio).sin()).sin();
                fm * (1.0 - wt_mix * 0.5) + base * (wt_mix * 0.5)
            }
            OscMode::AMSync => {
                let sync_ph = (ph * (1.0 + ctrl_a * 3.5)) % 1.0;
                let carrier = 2.0 * sync_ph - 1.0;
                let mod_sig = 0.5 + 0.5 * (ph * TAU).sin();
                (carrier * (1.0 - ctrl_b + ctrl_b * mod_sig)).clamp(-1.0, 1.0)
            }
            OscMode::SawPWM => {
                let split = (0.1 + ctrl_a * 0.8).clamp(0.1, 0.9);
                let s1 = 2.0 * ph - 1.0;
                let s2 = 2.0 * ((ph + split) % 1.0) - 1.0;
                (s1 - s2 * (0.4 + 0.6 * ctrl_b)).clamp(-1.0, 1.0)
            }
            OscMode::Vowel => {
                let f = 2.0 + ctrl_a * 6.0;
                ((ph * TAU).sin() * 0.4 + (ph * TAU * f).sin() * 0.6 * (1.0 - ph * 0.4)).clamp(-1.0, 1.0)
            }
        };

        let x = inner.left() + t * inner.width();
        let y = inner.center().y - y_val.clamp(-1.0, 1.0) * (inner.height() * 0.46);
        pts.push(Pos2::new(x, y));
    }

    painter.add(Shape::line(pts, Stroke::new(2.0_f32, P5_CYAN)));
    painter.rect_stroke(
        rect,
        CornerRadius::ZERO,
        Stroke::new(1.5_f32, P5_WHITE),
        StrokeKind::Inside,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn draw_filter_response_plot(
    painter: &egui::Painter,
    rect: Rect,
    f1_type: FilterType,
    f1_model: FilterModel,
    f1_cut: f32,
    f1_res: f32,
    f2_type: FilterType,
    f2_cut: f32,
    f2_res: f32,
    parallel: bool,
) {
    painter.rect_filled(rect.translate(Vec2::new(2.0, 2.0)), CornerRadius::ZERO, P5_BLACK);
    painter.rect_filled(rect, CornerRadius::ZERO, P5_BG);

    let inner = rect.shrink(4.0);
    // Horizontal 0dB line
    let zero_y = inner.top() + inner.height() * 0.38;
    painter.line_segment(
        [Pos2::new(inner.left(), zero_y), Pos2::new(inner.right(), zero_y)],
        Stroke::new(1.0_f32, Color32::from_rgb(38, 38, 50)),
    );

    let eval_mag_db = |ft: FilterType, cut_hz: f32, res: f32, freq: f32| -> f32 {
        let ratio = (freq / cut_hz.max(20.0)).max(1e-4);
        let log_r = ratio.log2();
        let q_bump = res * 13.0 * (-log_r * log_r * (4.0 + res * 8.0)).exp();
        let raw = match ft {
            FilterType::LP2 => -12.0 * log_r.max(0.0) + q_bump,
            FilterType::LP4 | FilterType::LR4 => -24.0 * log_r.max(0.0) + q_bump,
            FilterType::HP2 => -12.0 * (-log_r).max(0.0) + q_bump,
            FilterType::HP4 => -24.0 * (-log_r).max(0.0) + q_bump,
            FilterType::BP2 => -12.0 * log_r.abs() + q_bump * 0.8,
            FilterType::BP4 => -22.0 * log_r.abs() + q_bump * 0.8,
            FilterType::Peak => q_bump * 1.15,
            FilterType::Notch => -(18.0 + res * 10.0) * (-log_r * log_r * 10.0).exp(),
            FilterType::LpBp => -16.0 * log_r.max(0.0) - 6.0 * (-log_r).max(0.0) + q_bump,
            FilterType::HpBp => -16.0 * (-log_r).max(0.0) - 6.0 * log_r.max(0.0) + q_bump,
        };
        raw.clamp(-28.0, 14.0)
    };

    let n = 60usize;
    let mut pts1 = Vec::with_capacity(n);
    let mut pts2 = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / (n - 1) as f32;
        let freq = 20.0 * (1000.0_f32).powf(t);
        let db1 = eval_mag_db(f1_type, f1_cut, f1_res, freq);
        let db2 = eval_mag_db(f2_type, f2_cut, f2_res, freq);
        let x = inner.left() + t * inner.width();
        let y1 = (zero_y - db1 * (inner.height() / 44.0)).clamp(inner.top(), inner.bottom());
        let y2 = (zero_y - db2 * (inner.height() / 44.0)).clamp(inner.top(), inner.bottom());
        pts1.push(Pos2::new(x, y1));
        pts2.push(Pos2::new(x, y2));
    }

    painter.add(Shape::line(pts2, Stroke::new(1.8_f32, P5_RED)));
    painter.add(Shape::line(pts1, Stroke::new(2.0_f32, P5_CYAN)));

    // Compact badge inside plot
    let mode_tag = format!(
        "{} | {}",
        FilterModel::variants()[f1_model.to_index()],
        if parallel { "PARALLEL" } else { "SERIAL" }
    );
    painter.text(
        Pos2::new(rect.right() - 5.0, rect.top() + 4.0),
        egui::Align2::RIGHT_TOP,
        mode_tag,
        font_id(8.5, FONT_HEAVY),
        P5_GOLD,
    );

    painter.rect_stroke(
        rect,
        CornerRadius::ZERO,
        Stroke::new(1.5_f32, P5_WHITE),
        StrokeKind::Inside,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn draw_env_plot(
    painter: &egui::Painter,
    rect: Rect,
    att_norm: f32,
    dec_norm: f32,
    sus_norm: f32,
    slt_norm: f32,
    sll_norm: f32,
    rel_norm: f32,
    live_level: f32,
    accent: Color32,
) {
    painter.rect_filled(rect.translate(Vec2::new(2.0, 2.0)), CornerRadius::ZERO, P5_BLACK);
    painter.rect_filled(rect, CornerRadius::ZERO, P5_BG);

    let inner = rect.shrink(5.0);
    let w = inner.width();
    let h = inner.height();

    let x0 = inner.left();
    let x_att = x0 + (0.04 + att_norm * 0.22) * w;
    let x_dec = x_att + (0.06 + dec_norm * 0.22) * w;
    let x_slt = x_dec + (0.06 + slt_norm * 0.22) * w;
    let x_rel = (x_slt + (0.06 + rel_norm * 0.22) * w).min(inner.right());

    let y_bot = inner.bottom();
    let y_top = inner.top();
    let y_sus = y_bot - sus_norm.clamp(0.0, 1.0) * h;
    let y_sll = y_bot - sll_norm.clamp(0.0, 1.0) * h;

    let pts = vec![
        Pos2::new(x0, y_bot),
        Pos2::new(x_att, y_top),
        Pos2::new(x_dec, y_sus),
        Pos2::new(x_slt, y_sll),
        Pos2::new(x_rel, y_bot),
    ];

    // Translucent fill triangles under curve
    let fill_col = Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 36);
    for seg in pts.windows(2) {
        let poly = vec![
            seg[0],
            seg[1],
            Pos2::new(seg[1].x, y_bot),
            Pos2::new(seg[0].x, y_bot),
        ];
        painter.add(Shape::convex_polygon(poly, fill_col, Stroke::NONE));
    }

    painter.add(Shape::line(pts.clone(), Stroke::new(2.0_f32, accent)));
    for pt in &pts[1..4] {
        painter.circle_filled(*pt, 3.0, P5_WHITE);
    }

    // Live level meter bar on right edge
    if live_level > 0.005 {
        let lv_h = live_level.clamp(0.0, 1.0) * h;
        let bar = Rect::from_min_max(
            Pos2::new(inner.right() - 4.0, y_bot - lv_h),
            Pos2::new(inner.right(), y_bot),
        );
        painter.rect_filled(bar, CornerRadius::ZERO, P5_GOLD);
    }

    painter.rect_stroke(
        rect,
        CornerRadius::ZERO,
        Stroke::new(1.5_f32, P5_WHITE),
        StrokeKind::Inside,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn draw_lfo_plot(
    painter: &egui::Painter,
    rect: Rect,
    wave: LfoWaveform,
    morph: f32,
    sym: f32,
    phase: f32,
    unipolar: bool,
    live_phase: f32,
    accent: Color32,
) {
    painter.rect_filled(rect.translate(Vec2::new(2.0, 2.0)), CornerRadius::ZERO, P5_BLACK);
    painter.rect_filled(rect, CornerRadius::ZERO, P5_BG);

    let inner = rect.shrink(5.0);
    let mid_y = inner.center().y;
    painter.line_segment(
        [Pos2::new(inner.left(), mid_y), Pos2::new(inner.right(), mid_y)],
        Stroke::new(1.0_f32, Color32::from_rgb(38, 38, 50)),
    );

    let eval_lfo = |t_raw: f32| -> f32 {
        let mut t = (t_raw + phase) % 1.0;
        let s = sym.clamp(0.08, 0.92);
        t = if t < s {
            0.5 * (t / s)
        } else {
            0.5 + 0.5 * ((t - s) / (1.0 - s))
        };
        let raw = match wave {
            LfoWaveform::Sin => (t * TAU).sin(),
            LfoWaveform::Tri => 1.0 - 4.0 * (t - 0.5).abs(),
            LfoWaveform::SawUp => 2.0 * t - 1.0,
            LfoWaveform::SawDn => 1.0 - 2.0 * t,
            LfoWaveform::Square => if t < 0.5 { 1.0 } else { -1.0 },
            LfoWaveform::Snh => {
                let step = (t * 8.0).floor() as u32;
                let h = step.wrapping_mul(1103515245).wrapping_add(12345) & 0xFFFF;
                (h as f32 / 32768.0) - 1.0
            }
            LfoWaveform::SmoothRnd => (t * TAU).sin() * 0.6 + (t * TAU * 2.7).cos() * 0.4,
            LfoWaveform::Ramp => (2.0 * t - 1.0).powi(3),
        };
        let shaped = if morph > 0.01 {
            raw * (1.0 - morph) + (raw * 2.5).tanh() * morph
        } else {
            raw
        };
        if unipolar {
            (shaped * 0.5 + 0.5).clamp(0.0, 1.0) * 2.0 - 1.0
        } else {
            shaped.clamp(-1.0, 1.0)
        }
    };

    let n = 64usize;
    let mut pts = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / (n - 1) as f32;
        let v = eval_lfo(t);
        pts.push(Pos2::new(
            inner.left() + t * inner.width(),
            inner.center().y - v * (inner.height() * 0.44),
        ));
    }
    painter.add(Shape::line(pts, Stroke::new(2.0_f32, accent)));

    // Live playhead dot
    let lp = live_phase.rem_euclid(1.0);
    let lv = eval_lfo(lp);
    painter.circle_filled(
        Pos2::new(
            inner.left() + lp * inner.width(),
            inner.center().y - lv * (inner.height() * 0.44),
        ),
        3.5,
        P5_GOLD,
    );

    painter.rect_stroke(
        rect,
        CornerRadius::ZERO,
        Stroke::new(1.5_f32, P5_WHITE),
        StrokeKind::Inside,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn draw_eq_plot(
    painter: &egui::Painter,
    rect: Rect,
    eq_on: bool,
    low_f: f32,
    low_q: f32,
    low_g: f32,
    mid_f: f32,
    mid_q: f32,
    mid_g: f32,
    hi_f: f32,
    hi_q: f32,
    hi_g: f32,
    active_band: usize,
) {
    painter.rect_filled(rect.translate(Vec2::new(2.5, 2.5)), CornerRadius::ZERO, P5_BLACK);
    painter.rect_filled(rect, CornerRadius::ZERO, P5_BG);

    let inner = rect.shrink(6.0);
    let mid_y = inner.center().y;

    // Grid lines (Hz decades & dB)
    for &f in &[50.0_f32, 100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0, 10000.0] {
        let t = ((f / 20.0).ln() / (20000.0_f32 / 20.0).ln()).clamp(0.0, 1.0);
        let x = inner.left() + t * inner.width();
        painter.line_segment(
            [Pos2::new(x, inner.top()), Pos2::new(x, inner.bottom())],
            Stroke::new(1.0_f32, Color32::from_rgb(28, 28, 38)),
        );
    }
    for &db in &[-12.0_f32, -6.0, 0.0, 6.0, 12.0] {
        let y = mid_y - (db / 18.0) * (inner.height() * 0.5);
        painter.line_segment(
            [Pos2::new(inner.left(), y), Pos2::new(inner.right(), y)],
            Stroke::new(
                if db == 0.0 { 1.3_f32 } else { 0.8_f32 },
                if db == 0.0 { Color32::from_rgb(64, 64, 80) } else { Color32::from_rgb(28, 28, 38) },
            ),
        );
    }

    let eval_eq_db = |freq: f32| -> f32 {
        // Low shelf
        let lr = (freq / low_f.max(20.0)).log2();
        let low_resp = low_g / (1.0 + (2.0_f32).powf(lr * (1.2 + low_q * 0.8)));
        // Mid bell
        let mr = (freq / mid_f.max(20.0)).log2();
        let mid_resp = mid_g * (-mr * mr * (1.2 + mid_q * 1.4)).exp();
        // High shelf
        let hr = (freq / hi_f.max(20.0)).log2();
        let hi_resp = hi_g / (1.0 + (2.0_f32).powf(-hr * (1.2 + hi_q * 0.8)));
        (low_resp + mid_resp + hi_resp).clamp(-18.0, 18.0)
    };

    let n = 80usize;
    let mut pts = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / (n - 1) as f32;
        let freq = 20.0 * (1000.0_f32).powf(t);
        let db = if eq_on { eval_eq_db(freq) } else { 0.0 };
        let x = inner.left() + t * inner.width();
        let y = (mid_y - (db / 18.0) * (inner.height() * 0.48)).clamp(inner.top(), inner.bottom());
        pts.push(Pos2::new(x, y));
    }

    let curve_col = if eq_on { P5_CYAN } else { P5_DIM };
    let fill_col = if eq_on {
        Color32::from_rgba_unmultiplied(28, 232, 255, 34)
    } else {
        Color32::from_rgba_unmultiplied(100, 100, 120, 18)
    };

    for seg in pts.windows(2) {
        let poly = vec![
            seg[0],
            seg[1],
            Pos2::new(seg[1].x, mid_y),
            Pos2::new(seg[0].x, mid_y),
        ];
        painter.add(Shape::convex_polygon(poly, fill_col, Stroke::NONE));
    }
    painter.add(Shape::line(pts, Stroke::new(2.2_f32, curve_col)));

    // 3 band control node dots
    let bands = [
        (low_f, low_g, P5_GOLD, "L"),
        (mid_f, mid_g, P5_RED, "M"),
        (hi_f, hi_g, P5_MAGENTA, "H"),
    ];
    for (idx, &(f, g, col, tag)) in bands.iter().enumerate() {
        let t = ((f / 20.0).ln() / (20000.0_f32 / 20.0).ln()).clamp(0.0, 1.0);
        let x = inner.left() + t * inner.width();
        let y = (mid_y - (g / 18.0) * (inner.height() * 0.48)).clamp(inner.top() + 5.0, inner.bottom() - 5.0);
        let r = if idx == active_band { 6.5 } else { 4.5 };
        painter.circle_filled(Pos2::new(x, y), r, if eq_on { col } else { P5_DIM });
        painter.circle_stroke(Pos2::new(x, y), r, Stroke::new(1.5_f32, P5_WHITE));
        painter.text(
            Pos2::new(x, y - 10.0),
            egui::Align2::CENTER_BOTTOM,
            tag,
            font_id(8.5, FONT_HEAVY),
            if idx == active_band { P5_WHITE } else { P5_MUTED },
        );
    }

    // Axis captions
    for (lbl, x) in [("100Hz", inner.left() + inner.width() * 0.23), ("1kHz", inner.left() + inner.width() * 0.56), ("10kHz", inner.left() + inner.width() * 0.89)] {
        painter.text(
            Pos2::new(x, inner.bottom() - 1.0),
            egui::Align2::CENTER_BOTTOM,
            lbl,
            font_id(8.0, FONT_HEAVY),
            P5_DIM,
        );
    }

    painter.rect_stroke(
        rect,
        CornerRadius::ZERO,
        Stroke::new(1.8_f32, if eq_on { P5_WHITE } else { P5_DIM }),
        StrokeKind::Inside,
    );
}
