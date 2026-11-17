use crate::params::{ModSource, ModTarget, PhantomSpireParams};
use crate::ui::theme::{
    font_id, FONT_HEAVY, FONT_IMPACT, P5_BG, P5_BLACK, P5_CYAN, P5_DIM, P5_GOLD, P5_MAGENTA,
    P5_MUTED, P5_PANEL, P5_RED, P5_SURFACE, P5_WHITE,
};
use crate::ui::widgets::{
    draw_cutout_card, p5_button, p5_diamond_pill, p5_enum_dropdown, p5_int_stepper, p5_slider,
    p5_toggle_pill, PopupOverlayState,
};
use crate::{SharedSynthState, SharedUiState};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, Color32, CornerRadius, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2,
};
use std::sync::atomic::Ordering;

#[derive(Default)]
pub struct ModPageState {
    pub active_stepper: usize, // 0 = STP 1, 1 = STP 2
}

#[allow(clippy::too_many_arguments)]
pub fn draw_mod_page(
    ui: &mut egui::Ui,
    width: f32,
    height: f32,
    params: &PhantomSpireParams,
    ui_state: &SharedUiState,
    shared_synth: &SharedSynthState,
    setter: &ParamSetter,
    popups: &mut PopupOverlayState,
    state: &mut ModPageState,
) {
    let left_w = 446.0_f32;
    let right_w = (width - left_w - 8.0).max(400.0);

    ui.allocate_ui_with_layout(
        Vec2::new(width, height),
        egui::Layout::left_to_right(egui::Align::Min),
        |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

            // ================================================================
            // LEFT COLUMN: 01 // 15-SLOT MODULATION MATRIX
            // ================================================================
            ui.allocate_ui_with_layout(
                Vec2::new(left_w, height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    draw_mod_matrix_card(
                        ui,
                        height,
                        params,
                        ui_state,
                        shared_synth,
                        setter,
                        popups,
                    );
                },
            );

            // ================================================================
            // RIGHT COLUMN: 02 // STEPPER 1 & 2  +  03 // ARPEGGIATOR
            // ================================================================
            let stp_h = 258.0_f32;
            let arp_h = (height - stp_h - 8.0).max(240.0);
            ui.allocate_ui_with_layout(
                Vec2::new(right_w, height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    draw_stepper_editor_card(
                        ui,
                        stp_h,
                        params,
                        ui_state,
                        shared_synth,
                        setter,
                        popups,
                        state,
                    );
                    draw_arp_editor_card(ui, arp_h, params, ui_state, shared_synth, setter, popups);
                },
            );
        },
    );
}

// ============================================================================
// 01 // 15-SLOT MOD MATRIX
// ============================================================================

#[allow(clippy::too_many_arguments)]
fn draw_mod_matrix_card(
    ui: &mut egui::Ui,
    height: f32,
    params: &PhantomSpireParams,
    ui_state: &SharedUiState,
    shared_synth: &SharedSynthState,
    setter: &ParamSetter,
    popups: &mut PopupOverlayState,
) {
    let active_count = params
        .mod_slots
        .iter()
        .filter(|s| s.source.value() != ModSource::Off && s.target.value() != ModTarget::Off)
        .count();

    draw_cutout_card(
        ui,
        "01 // MOD MATRIX",
        P5_MAGENTA,
        P5_WHITE,
        height,
        |ui| {
            let (badge_r, _) = ui.allocate_exact_size(Vec2::new(98.0, 20.0), Sense::hover());
            ui.painter().rect_filled(badge_r, CornerRadius::ZERO, P5_PANEL);
            ui.painter().rect_stroke(
                badge_r,
                CornerRadius::ZERO,
                Stroke::new(1.2_f32, P5_MAGENTA),
                StrokeKind::Inside,
            );
            ui.painter().text(
                badge_r.center(),
                egui::Align2::CENTER_CENTER,
                format!("{active_count}/15 ACTIVE"),
                font_id(9.5, FONT_HEAVY),
                P5_GOLD,
            );
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            let avail_w = ui.available_width();

            // Column headers
            let (hdr_r, _) = ui.allocate_exact_size(Vec2::new(avail_w, 14.0), Sense::hover());
            let hp = ui.painter();
            hp.text(Pos2::new(hdr_r.left() + 4.0, hdr_r.center().y), egui::Align2::LEFT_CENTER, "#", font_id(8.5, FONT_HEAVY), P5_MUTED);
            hp.text(Pos2::new(hdr_r.left() + 28.0, hdr_r.center().y), egui::Align2::LEFT_CENTER, "SOURCE", font_id(8.5, FONT_HEAVY), P5_CYAN);
            hp.text(Pos2::new(hdr_r.left() + 150.0, hdr_r.center().y), egui::Align2::LEFT_CENTER, "TARGET", font_id(8.5, FONT_HEAVY), P5_GOLD);
            hp.text(Pos2::new(hdr_r.right() - 4.0, hdr_r.center().y), egui::Align2::RIGHT_CENTER, "AMOUNT (BIPOLAR)", font_id(8.5, FONT_HEAVY), P5_MAGENTA);

            // All 15 slots visible simultaneously (15 * 32px = 480px)
            let row_h = 28.5_f32;
            let src_w = 108.0_f32;
            let tgt_w = 136.0_f32;
            let amt_w = (avail_w - 22.0 - src_w - 14.0 - tgt_w - 16.0).max(100.0);

            for i in 0..15 {
                let slot = &params.mod_slots[i];
                let is_active =
                    slot.source.value() != ModSource::Off && slot.target.value() != ModTarget::Off;

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;

                    // Slot index badge
                    let (idx_r, _) = ui.allocate_exact_size(Vec2::new(22.0, row_h), Sense::hover());
                    ui.painter().rect_filled(
                        idx_r,
                        CornerRadius::ZERO,
                        if is_active { P5_RED } else { P5_SURFACE },
                    );
                    ui.painter().text(
                        idx_r.center(),
                        egui::Align2::CENTER_CENTER,
                        format!("{:02}", i + 1),
                        font_id(9.5, FONT_IMPACT),
                        if is_active { P5_WHITE } else { P5_MUTED },
                    );

                    // Source custom P5 dropdown
                    if p5_enum_dropdown(
                        ui,
                        &format!("mtrx_src_{i}"),
                        &slot.source,
                        setter,
                        ui_state,
                        popups,
                        src_w,
                        row_h,
                        P5_CYAN,
                    ) {
                        if let Ok(mut extra) = params.extra_state.lock() {
                            extra.mod_sources[i] = slot.source.value().to_index() as u8;
                            shared_synth.sync_from_extra(&extra);
                        }
                    }

                    // Arrow glyph
                    let (ar, _) = ui.allocate_exact_size(Vec2::new(10.0, row_h), Sense::hover());
                    ui.painter().text(
                        ar.center(),
                        egui::Align2::CENTER_CENTER,
                        "→",
                        font_id(11.0, FONT_HEAVY),
                        if is_active { P5_GOLD } else { P5_DIM },
                    );

                    // Target custom P5 dropdown
                    if p5_enum_dropdown(
                        ui,
                        &format!("mtrx_tgt_{i}"),
                        &slot.target,
                        setter,
                        ui_state,
                        popups,
                        tgt_w,
                        row_h,
                        P5_GOLD,
                    ) {
                        if let Ok(mut extra) = params.extra_state.lock() {
                            extra.mod_targets[i] = slot.target.value().to_index() as u8;
                            shared_synth.sync_from_extra(&extra);
                        }
                    }

                    // Bipolar Amount Slider
                    p5_slider(
                        ui,
                        "",
                        &slot.amount,
                        setter,
                        ui_state,
                        P5_RED,
                        amt_w,
                        row_h,
                        true,
                    );
                });
            }
        },
    );
}

// ============================================================================
// 02 // STEPPER 1 & 2 (Full 16-Step Editor + TIE Diamonds)
// ============================================================================

#[allow(clippy::too_many_arguments)]
fn draw_stepper_editor_card(
    ui: &mut egui::Ui,
    height: f32,
    params: &PhantomSpireParams,
    ui_state: &SharedUiState,
    shared_synth: &SharedSynthState,
    setter: &ParamSetter,
    popups: &mut PopupOverlayState,
    state: &mut ModPageState,
) {
    let idx = state.active_stepper.min(1);
    let stp = &params.steppers[idx];

    draw_cutout_card(
        ui,
        "02 // STEPPER 1 & 2",
        P5_GOLD,
        P5_WHITE,
        height,
        |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if p5_button(ui, "CLR", 36.0, 20.0, false, P5_RED, true) {
                if let Ok(mut extra) = params.extra_state.lock() {
                    extra.stepper_steps[idx] = [0.5; 16];
                    extra.stepper_ties[idx] = 0;
                    shared_synth.sync_from_extra(&extra);
                }
                ui_state.set_last_param_label(&format!("STP{} CLEARED", idx + 1));
            }
            if p5_button(ui, "RND", 36.0, 20.0, false, P5_MAGENTA, true) {
                if let Ok(mut extra) = params.extra_state.lock() {
                    let mut seed = shared_synth.revision.load(Ordering::Relaxed) as u32 ^ 0xA511_E9B3;
                    for s in 0..16 {
                        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                        extra.stepper_steps[idx][s] = ((seed >> 16) as f32 / 65535.0).clamp(0.05, 1.0);
                    }
                    shared_synth.sync_from_extra(&extra);
                }
                ui_state.set_last_param_label(&format!("STP{} RANDOMIZED", idx + 1));
            }
            ui.add_space(6.0);
            if p5_button(ui, "STP 2", 48.0, 20.0, idx == 1, P5_CYAN, true) {
                state.active_stepper = 1;
            }
            if p5_button(ui, "STP 1", 48.0, 20.0, idx == 0, P5_GOLD, true) {
                state.active_stepper = 0;
            }
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = 5.0;
            let avail_w = ui.available_width();

            // Controls bar
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                p5_toggle_pill(ui, "SYNC", &stp.sync, setter, ui_state, 46.0, 21.0, P5_GOLD);
                p5_toggle_pill(ui, "LOOP", &stp.loop_mode, setter, ui_state, 46.0, 21.0, P5_CYAN);
                if stp.sync.value() {
                    p5_enum_dropdown(ui, &format!("mod_stp_{idx}_div"), &stp.sync_div, setter, ui_state, popups, 70.0, 21.0, P5_GOLD);
                } else {
                    p5_slider(ui, "HZ", &stp.rate, setter, ui_state, P5_GOLD, 70.0, 21.0, false);
                }
                p5_int_stepper(ui, "LEN", &stp.steps_count, setter, ui_state, 74.0, 21.0, P5_WHITE);
                let rem = (avail_w - 252.0).max(120.0) * 0.5;
                p5_slider(ui, "GATE", &stp.gate, setter, ui_state, P5_CYAN, rem, 21.0, false);
                p5_slider(ui, "SMTH", &stp.smooth, setter, ui_state, P5_MAGENTA, rem, 21.0, false);
            });

            // 16-Bar Drag-to-Paint Stepper Canvas
            let active_step = ui_state.stp_steps[idx].load(Ordering::Relaxed) as usize;
            let step_count = stp.steps_count.value().clamp(2, 16) as usize;
            let (mut steps, mut ties) = params
                .extra_state
                .lock()
                .map(|g| (g.stepper_steps[idx], g.stepper_ties[idx]))
                .unwrap_or(([0.5; 16], 0));

            let (rect, resp) = ui.allocate_exact_size(Vec2::new(avail_w, 160.0), Sense::click_and_drag());
            let p = ui.painter();
            p.rect_filled(rect, CornerRadius::ZERO, P5_BG);

            // 50% centerline
            p.line_segment(
                [Pos2::new(rect.left(), rect.center().y), Pos2::new(rect.right(), rect.center().y)],
                Stroke::new(1.0_f32, Color32::from_rgb(38, 38, 52)),
            );

            if (resp.dragged() || resp.clicked()) && ui.input(|i| i.pointer.primary_down()) {
                if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                    let col = (((pos.x - rect.left()) / rect.width().max(1.0)) * 16.0).floor() as isize;
                    if (0..16).contains(&col) {
                        let val = (1.0 - (pos.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
                        steps[col as usize] = val;
                        if let Ok(mut extra) = params.extra_state.lock() {
                            extra.stepper_steps[idx][col as usize] = val;
                            shared_synth.sync_from_extra(&extra);
                        }
                        ui_state.set_last_param_label(&format!(
                            "STP{} STEP {}: {:.0}%",
                            idx + 1,
                            col + 1,
                            val * 100.0
                        ));
                    }
                }
            }

            let accent = if idx == 0 { P5_GOLD } else { P5_CYAN };
            let col_w = rect.width() / 16.0;
            for i in 0..16 {
                let x0 = rect.left() + i as f32 * col_w;
                let cell = Rect::from_min_size(
                    Pos2::new(x0 + 2.0, rect.top() + 3.0),
                    Vec2::new(col_w - 4.0, rect.height() - 6.0),
                );
                let in_len = i < step_count;
                let is_play = i == active_step;
                p.rect_filled(
                    cell,
                    CornerRadius::ZERO,
                    if is_play { Color32::from_rgb(42, 42, 58) } else { P5_PANEL },
                );
                let fh = cell.height() * steps[i];
                let fill = Rect::from_min_max(
                    Pos2::new(cell.left(), cell.bottom() - fh),
                    cell.max,
                );
                p.rect_filled(
                    fill,
                    CornerRadius::ZERO,
                    if !in_len {
                        P5_DIM
                    } else if is_play {
                        P5_WHITE
                    } else {
                        accent
                    },
                );
                p.text(
                    Pos2::new(cell.center().x, cell.top() + 8.0),
                    egui::Align2::CENTER_CENTER,
                    format!("{}", i + 1),
                    font_id(8.0, FONT_HEAVY),
                    if in_len { P5_MUTED } else { Color32::from_rgb(56, 56, 68) },
                );
            }
            p.rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.5_f32, P5_WHITE), StrokeKind::Inside);

            // TIE Diamond Toggle Row
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for i in 0..16 {
                    let tied = (ties & (1u16 << i)) != 0;
                    let (tr, tresp) = ui.allocate_exact_size(Vec2::new(col_w, 18.0), Sense::click());
                    if tresp.clicked() {
                        ties ^= 1u16 << i;
                        if let Ok(mut extra) = params.extra_state.lock() {
                            extra.stepper_ties[idx] = ties;
                            shared_synth.sync_from_extra(&extra);
                        }
                    }
                    let c = tr.center();
                    let dia = vec![
                        Pos2::new(c.x, c.y - 5.0),
                        Pos2::new(c.x + 5.0, c.y),
                        Pos2::new(c.x, c.y + 5.0),
                        Pos2::new(c.x - 5.0, c.y),
                    ];
                    ui.painter().add(Shape::convex_polygon(
                        dia,
                        if tied { P5_RED } else { P5_SURFACE },
                        Stroke::new(1.1_f32, if tied { P5_WHITE } else { P5_DIM }),
                    ));
                }
            });
        },
    );
}

// ============================================================================
// 03 // ARPEGGIATOR (Full 16-Step Gate / Semitone Offset / Velocity Editor)
// ============================================================================

#[allow(clippy::too_many_arguments)]
fn draw_arp_editor_card(
    ui: &mut egui::Ui,
    height: f32,
    params: &PhantomSpireParams,
    ui_state: &SharedUiState,
    shared_synth: &SharedSynthState,
    setter: &ParamSetter,
    popups: &mut PopupOverlayState,
) {
    let arp = &params.arp;

    draw_cutout_card(
        ui,
        "03 // ARPEGGIATOR",
        P5_CYAN,
        if arp.enabled.value() { P5_WHITE } else { P5_MUTED },
        height,
        |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            p5_diamond_pill(ui, "ARP ON", &arp.enabled, setter, ui_state, 74.0, 20.0, P5_CYAN, false);
            p5_toggle_pill(ui, "LATCH", &arp.latch, setter, ui_state, 50.0, 20.0, P5_GOLD);
            p5_toggle_pill(ui, "SYNC", &arp.sync, setter, ui_state, 44.0, 20.0, P5_RED);
            if p5_button(ui, "RST", 34.0, 20.0, false, P5_WHITE, true) {
                if let Ok(mut extra) = params.extra_state.lock() {
                    extra.arp_steps_enabled = 0xFFFF;
                    extra.arp_step_offsets = [0; 16];
                    extra.arp_step_velocities = [100; 16];
                    shared_synth.sync_from_extra(&extra);
                }
                ui_state.set_last_param_label("ARP PATTERN RESET");
            }
        },
        |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let avail_w = ui.available_width();

            // Row 1: Mode, Vel Mode, Octaves, Steps
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                p5_enum_dropdown(ui, "mod_arp_mode", &arp.mode, setter, ui_state, popups, 114.0, 21.0, P5_CYAN);
                p5_enum_dropdown(ui, "mod_arp_vel", &arp.vel_mode, setter, ui_state, popups, 108.0, 21.0, P5_GOLD);
                p5_int_stepper(ui, "OCT", &arp.octaves, setter, ui_state, 86.0, 21.0, P5_GOLD);
                let rem = (avail_w - 320.0).max(80.0);
                p5_int_stepper(ui, "STEPS", &arp.steps_count, setter, ui_state, rem, 21.0, P5_WHITE);
            });

            // Row 2: Rate/SyncDiv, Gate, Swing, BPM
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if arp.sync.value() {
                    p5_enum_dropdown(ui, "mod_arp_div", &arp.sync_div, setter, ui_state, popups, 82.0, 21.0, P5_GOLD);
                } else {
                    p5_slider(ui, "RATE", &arp.rate, setter, ui_state, P5_GOLD, 82.0, 21.0, false);
                }
                let sw = (avail_w - 94.0) / 3.0;
                p5_slider(ui, "GATE", &arp.gate, setter, ui_state, P5_CYAN, sw, 21.0, false);
                p5_slider(ui, "SWING", &arp.swing, setter, ui_state, P5_MAGENTA, sw, 21.0, false);
                p5_slider(ui, "TEMPO", &arp.internal_bpm, setter, ui_state, P5_RED, sw, 21.0, false);
            });

            let active_step = ui_state.arp_step.load(Ordering::Relaxed) as usize;
            let step_count = arp.steps_count.value().clamp(1, 16) as usize;
            let (mut gate_mask, mut offsets, mut vels) = params
                .extra_state
                .lock()
                .map(|g| (g.arp_steps_enabled, g.arp_step_offsets, g.arp_step_velocities))
                .unwrap_or((0xFFFF, [0; 16], [100; 16]));

            let col_w = avail_w / 16.0;

            // Strip 1: 16 Gate Toggle Buttons
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for i in 0..16 {
                    let on = (gate_mask & (1u16 << i)) != 0;
                    let in_len = i < step_count;
                    let is_play = i == active_step;
                    let (gr, gresp) = ui.allocate_exact_size(Vec2::new(col_w, 24.0), Sense::click());
                    if gresp.clicked() {
                        gate_mask ^= 1u16 << i;
                        if let Ok(mut extra) = params.extra_state.lock() {
                            extra.arp_steps_enabled = gate_mask;
                            shared_synth.sync_from_extra(&extra);
                        }
                    }
                    let pill = gr.shrink2(Vec2::new(1.5, 1.0));
                    let fill = if !in_len {
                        P5_BG
                    } else if is_play {
                        P5_GOLD
                    } else if on {
                        P5_CYAN
                    } else {
                        P5_SURFACE
                    };
                    ui.painter().rect(
                        pill,
                        CornerRadius::ZERO,
                        fill,
                        Stroke::new(1.1_f32, if is_play { P5_WHITE } else { P5_BLACK }),
                        StrokeKind::Inside,
                    );
                    ui.painter().text(
                        pill.center(),
                        egui::Align2::CENTER_CENTER,
                        format!("{}", i + 1),
                        font_id(8.5, FONT_IMPACT),
                        if on || is_play { P5_BLACK } else { P5_MUTED },
                    );
                }
            });

            // Strip 2: 16 Semitone Offset Bars (-12 .. +12 st)
            let (off_r, off_resp) =
                ui.allocate_exact_size(Vec2::new(avail_w, 72.0), Sense::click_and_drag());
            {
                let p = ui.painter();
                p.rect_filled(off_r, CornerRadius::ZERO, P5_BG);
                p.line_segment(
                    [Pos2::new(off_r.left(), off_r.center().y), Pos2::new(off_r.right(), off_r.center().y)],
                    Stroke::new(1.0_f32, P5_DIM),
                );
                if (off_resp.dragged() || off_resp.clicked()) && ui.input(|i| i.pointer.primary_down()) {
                    if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                        let col = (((pos.x - off_r.left()) / off_r.width().max(1.0)) * 16.0).floor() as isize;
                        if (0..16).contains(&col) {
                            let norm = (1.0 - (pos.y - off_r.top()) / off_r.height().max(1.0)).clamp(0.0, 1.0);
                            let st = ((norm * 24.0).round() as i32 - 12).clamp(-12, 12) as i8;
                            offsets[col as usize] = st;
                            if let Ok(mut extra) = params.extra_state.lock() {
                                extra.arp_step_offsets[col as usize] = st;
                                shared_synth.sync_from_extra(&extra);
                            }
                            ui_state.set_last_param_label(&format!("ARP STEP {} OFFSET: {:+} st", col + 1, st));
                        }
                    }
                }
                if off_resp.double_clicked() {
                    if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                        let col = (((pos.x - off_r.left()) / off_r.width().max(1.0)) * 16.0).floor() as isize;
                        if (0..16).contains(&col) {
                            offsets[col as usize] = 0;
                            if let Ok(mut extra) = params.extra_state.lock() {
                                extra.arp_step_offsets[col as usize] = 0;
                                shared_synth.sync_from_extra(&extra);
                            }
                        }
                    }
                }

                for i in 0..16 {
                    let x0 = off_r.left() + i as f32 * col_w;
                    let mid_y = off_r.center().y;
                    let frac = offsets[i] as f32 / 12.0;
                    let tip_y = mid_y - frac * (off_r.height() * 0.44);
                    let bar = Rect::from_min_max(
                        Pos2::new(x0 + 2.5, mid_y.min(tip_y)),
                        Pos2::new(x0 + col_w - 2.5, mid_y.max(tip_y)),
                    );
                    let in_len = i < step_count;
                    let col = if !in_len {
                        P5_DIM
                    } else if offsets[i] >= 0 {
                        P5_GOLD
                    } else {
                        P5_MAGENTA
                    };
                    p.rect_filled(bar, CornerRadius::ZERO, col);
                    if offsets[i] != 0 {
                        p.text(
                            Pos2::new(x0 + col_w * 0.5, off_r.top() + 8.0),
                            egui::Align2::CENTER_CENTER,
                            format!("{:+}", offsets[i]),
                            font_id(7.5, FONT_HEAVY),
                            P5_WHITE,
                        );
                    }
                }
                p.text(
                    Pos2::new(off_r.left() + 4.0, off_r.bottom() - 3.0),
                    egui::Align2::LEFT_BOTTOM,
                    "PITCH OFFSET (-12..+12 ST)",
                    font_id(7.5, FONT_HEAVY),
                    P5_MUTED,
                );
                p.rect_stroke(off_r, CornerRadius::ZERO, Stroke::new(1.3_f32, P5_WHITE), StrokeKind::Inside);
            }

            // Strip 3: 16 Velocity Bars (1..127)
            let (vel_r, vel_resp) =
                ui.allocate_exact_size(Vec2::new(avail_w, 66.0), Sense::click_and_drag());
            {
                let p = ui.painter();
                p.rect_filled(vel_r, CornerRadius::ZERO, P5_BG);
                if (vel_resp.dragged() || vel_resp.clicked()) && ui.input(|i| i.pointer.primary_down()) {
                    if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                        let col = (((pos.x - vel_r.left()) / vel_r.width().max(1.0)) * 16.0).floor() as isize;
                        if (0..16).contains(&col) {
                            let norm = (1.0 - (pos.y - vel_r.top()) / vel_r.height().max(1.0)).clamp(0.02, 1.0);
                            let v = (norm * 127.0).round().clamp(1.0, 127.0) as u8;
                            vels[col as usize] = v;
                            if let Ok(mut extra) = params.extra_state.lock() {
                                extra.arp_step_velocities[col as usize] = v;
                                shared_synth.sync_from_extra(&extra);
                            }
                            ui_state.set_last_param_label(&format!("ARP STEP {} VEL: {}", col + 1, v));
                        }
                    }
                }

                for i in 0..16 {
                    let x0 = vel_r.left() + i as f32 * col_w;
                    let frac = vels[i] as f32 / 127.0;
                    let h = (vel_r.height() - 6.0) * frac;
                    let bar = Rect::from_min_max(
                        Pos2::new(x0 + 2.5, vel_r.bottom() - 3.0 - h),
                        Pos2::new(x0 + col_w - 2.5, vel_r.bottom() - 3.0),
                    );
                    let in_len = i < step_count;
                    let on = (gate_mask & (1u16 << i)) != 0;
                    let col = if !in_len || !on {
                        P5_DIM
                    } else if i == active_step {
                        P5_WHITE
                    } else {
                        P5_RED
                    };
                    p.rect_filled(bar, CornerRadius::ZERO, col);
                }
                p.text(
                    Pos2::new(vel_r.left() + 4.0, vel_r.top() + 3.0),
                    egui::Align2::LEFT_TOP,
                    "STEP VELOCITY (1..127)",
                    font_id(7.5, FONT_HEAVY),
                    P5_MUTED,
                );
                p.rect_stroke(vel_r, CornerRadius::ZERO, Stroke::new(1.3_f32, P5_WHITE), StrokeKind::Inside);
            }
        },
    );
}
