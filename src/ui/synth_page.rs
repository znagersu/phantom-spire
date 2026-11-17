use crate::params::{
    EnvParams, ExtraSynthState, LfoParams, OscParams, PhantomSpireParams, StepperParams,
};
use crate::ui::theme::{
    font_id, FONT_HEAVY, FONT_IMPACT, P5_BG, P5_BLACK, P5_CYAN, P5_DIM, P5_GOLD, P5_MAGENTA,
    P5_MUTED, P5_PANEL, P5_RED, P5_SURFACE, P5_WHITE,
};
use crate::ui::widgets::{
    draw_cutout_card, draw_env_plot, draw_filter_response_plot, draw_lfo_plot,
    draw_osc_waveform_plot, p5_button, p5_diamond_pill, p5_enum_dropdown, p5_int_stepper,
    p5_knob_sized, p5_slider, p5_toggle_pill, p5_v_fader, PopupOverlayState,
};
use crate::{SharedSynthState, SharedUiState};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
};
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ModBayASelection {
    Env1,
    Env2,
    Lfo1,
    Lfo2,
    Stp1,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ModBayBSelection {
    Env3,
    Env4,
    Lfo3,
    Lfo4,
    Stp2,
    Arp,
}

pub struct SynthPageState {
    pub active_osc: usize,
    pub bay_a: ModBayASelection,
    pub bay_b: ModBayBSelection,
}

impl Default for SynthPageState {
    fn default() -> Self {
        Self {
            active_osc: 0,
            bay_a: ModBayASelection::Env1,
            bay_b: ModBayBSelection::Env3,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw_synth_page(
    ui: &mut egui::Ui,
    width: f32,
    height: f32,
    params: &PhantomSpireParams,
    ui_state: &SharedUiState,
    shared_synth: &SharedSynthState,
    setter: &ParamSetter,
    popups: &mut PopupOverlayState,
    state: &mut SynthPageState,
    midi_learn: &mut Option<String>,
) {
    let top_h = 294.0_f32;
    let bot_h = (height - top_h - 8.0).max(220.0);
    let osc_w = 602.0_f32;
    let flt_w = (width - osc_w - 8.0).max(260.0);

    // ========================================================================
    // TOP ROW: 01 // OSCILLATORS + 02 // DUAL FILTER 1 & 2
    // ========================================================================
    ui.allocate_ui_with_layout(
        Vec2::new(width, top_h),
        egui::Layout::left_to_right(egui::Align::Min),
        |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

            // --- 01 // OSCILLATORS ---
            ui.allocate_ui_with_layout(
                Vec2::new(osc_w, top_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    draw_osc_card(ui, top_h, params, ui_state, setter, popups, state, midi_learn);
                },
            );

            // --- 02 // DUAL FILTER 1 & 2 ---
            ui.allocate_ui_with_layout(
                Vec2::new(flt_w, top_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    draw_filter_card(ui, top_h, params, ui_state, setter, popups, midi_learn);
                },
            );
        },
    );

    ui.add_space(8.0);

    // ========================================================================
    // BOTTOM ROW: 03 // MOD BAY A  +  04 // MOD BAY B
    // ========================================================================
    let bay_w = (width - 8.0) * 0.5;
    ui.allocate_ui_with_layout(
        Vec2::new(width, bot_h),
        egui::Layout::left_to_right(egui::Align::Min),
        |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

            ui.allocate_ui_with_layout(
                Vec2::new(bay_w, bot_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    draw_mod_bay_a(
                        ui,
                        bot_h,
                        params,
                        ui_state,
                        shared_synth,
                        setter,
                        popups,
                        state,
                        midi_learn,
                    );
                },
            );

            ui.allocate_ui_with_layout(
                Vec2::new(bay_w, bot_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    draw_mod_bay_b(
                        ui,
                        bot_h,
                        params,
                        ui_state,
                        shared_synth,
                        setter,
                        popups,
                        state,
                        midi_learn,
                    );
                },
            );
        },
    );
}

// ============================================================================
// 01 // OSCILLATORS Card
// ============================================================================

#[allow(clippy::too_many_arguments)]
fn draw_osc_card(
    ui: &mut egui::Ui,
    height: f32,
    params: &PhantomSpireParams,
    ui_state: &SharedUiState,
    setter: &ParamSetter,
    popups: &mut PopupOverlayState,
    state: &mut SynthPageState,
    midi_learn: &mut Option<String>,
) {
    let has_copy_buf = params
        .extra_state
        .lock()
        .map(|g| g.osc_copy_buffer.is_some())
        .unwrap_or(false);

    let active_osc = state.active_osc;
    let mut next_osc = active_osc;
    draw_cutout_card(
        ui,
        "01 // OSCILLATORS",
        P5_GOLD,
        P5_WHITE,
        height,
        |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if p5_button(ui, "PST", 36.0, 20.0, false, P5_CYAN, has_copy_buf) {
                paste_osc_from_buffer(params, active_osc, setter);
                ui_state.set_last_param_label(&format!("PASTED TO OSC {}", active_osc + 1));
            }
            if p5_button(ui, "CPY", 36.0, 20.0, false, P5_GOLD, true) {
                copy_osc_to_buffer(params, active_osc);
                ui_state.set_last_param_label(&format!("COPIED OSC {}", active_osc + 1));
            }
            ui.add_space(6.0);
            for i in (0..4).rev() {
                let is_sel = active_osc == i;
                let active_lvl = params.oscs[i].mix_level.value() > 0.01;
                let lbl = if active_lvl {
                    format!("•OSC{}", i + 1)
                } else {
                    format!("OSC{}", i + 1)
                };
                if p5_button(
                    ui,
                    &lbl,
                    50.0,
                    20.0,
                    is_sel,
                    if active_lvl { P5_RED } else { P5_CYAN },
                    true,
                ) {
                    next_osc = i;
                }
            }
        },
        |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(5.0, 4.0);

            // Top Master Mix Strip for all 4 Oscillators simultaneously
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let sw = (ui.available_width() - 18.0) / 4.0;
                let colors = [P5_RED, P5_GOLD, P5_CYAN, P5_MAGENTA];
                for i in 0..4 {
                    p5_slider(
                        ui,
                        &format!("OSC{} MIX", i + 1),
                        &params.oscs[i].mix_level,
                        setter,
                        ui_state,
                        colors[i],
                        sw,
                        18.0,
                        false,
                    );
                }
            });

            let osc = &params.oscs[active_osc];
            let osc_id = active_osc;
            let body_h = 226.0;
            let wave_w = 290.0;
            let uni_w = 144.0;
            let mix_w = (ui.available_width() - wave_w - uni_w - 10.0).max(130.0);

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;

                // --- SUBPANEL A: WAVE ---
                draw_sub_box(ui, "WAVE", P5_CYAN, wave_w, body_h, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        p5_enum_dropdown(
                            ui,
                            &format!("osc_{osc_id}_mode"),
                            &osc.mode,
                            setter,
                            ui_state,
                            popups,
                            135.0,
                            21.0,
                            P5_GOLD,
                        );
                        p5_enum_dropdown(
                            ui,
                            &format!("osc_{osc_id}_wave"),
                            &osc.waveform,
                            setter,
                            ui_state,
                            popups,
                            135.0,
                            21.0,
                            P5_CYAN,
                        );
                    });

                    ui.add_space(3.0);

                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        // Left knob col: NOTE, CTRL A, PHASE
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            p5_knob_sized(ui, "NOTE", &osc.note, setter, ui_state, P5_WHITE, 64.0, 29.0, true, midi_learn);
                            p5_knob_sized(ui, "CTRL A", &osc.ctrl_a, setter, ui_state, P5_CYAN, 64.0, 29.0, false, midi_learn);
                            p5_knob_sized(ui, "PHASE", &osc.phase, setter, ui_state, P5_MAGENTA, 64.0, 29.0, false, midi_learn);
                        });

                        // Center: Live Waveform Plot + OCTAVE stepper
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 5.0;
                            ui.add_space(4.0);
                            p5_int_stepper(
                                ui,
                                "OCT",
                                &osc.octave,
                                setter,
                                ui_state,
                                138.0,
                                22.0,
                                P5_GOLD,
                            );
                            let (plot_r, _) =
                                ui.allocate_exact_size(Vec2::new(138.0, 108.0), Sense::hover());
                            draw_osc_waveform_plot(
                                ui.painter(),
                                plot_r,
                                osc.mode.value(),
                                osc.waveform.value(),
                                osc.ctrl_a.value(),
                                osc.ctrl_b.value(),
                                osc.wt_mix.value(),
                                osc.phase.value(),
                            );
                            let (tag_r, _) =
                                ui.allocate_exact_size(Vec2::new(138.0, 16.0), Sense::hover());
                            ui.painter().text(
                                tag_r.center(),
                                egui::Align2::CENTER_CENTER,
                                format!("OSC {} ACTIVE ENGINE", osc_id + 1),
                                font_id(9.0, FONT_IMPACT),
                                P5_GOLD,
                            );
                        });

                        // Right knob col: FINE, CTRL B, WT MIX
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            p5_knob_sized(ui, "FINE", &osc.fine, setter, ui_state, P5_WHITE, 64.0, 29.0, true, midi_learn);
                            p5_knob_sized(ui, "CTRL B", &osc.ctrl_b, setter, ui_state, P5_GOLD, 64.0, 29.0, false, midi_learn);
                            p5_knob_sized(ui, "WT MIX", &osc.wt_mix, setter, ui_state, P5_RED, 64.0, 29.0, false, midi_learn);
                        });
                    });
                });

                // --- SUBPANEL B: UNISON ---
                draw_sub_box(ui, "UNISON", P5_GOLD, uni_w, body_h, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        p5_knob_sized(ui, "DETUNE", &osc.unison_detune, setter, ui_state, P5_GOLD, 63.0, 32.0, false, midi_learn);
                        p5_knob_sized(ui, "DENSITY", &osc.unison_density, setter, ui_state, P5_CYAN, 63.0, 32.0, false, midi_learn);
                    });
                    ui.add_space(4.0);
                    p5_int_stepper(
                        ui,
                        "VOICES",
                        &osc.unison_count,
                        setter,
                        ui_state,
                        130.0,
                        22.0,
                        P5_GOLD,
                    );
                    ui.add_space(3.0);
                    p5_enum_dropdown(
                        ui,
                        &format!("osc_{osc_id}_unison_mode"),
                        &osc.unison_mode,
                        setter,
                        ui_state,
                        popups,
                        130.0,
                        22.0,
                        P5_GOLD,
                    );
                    ui.add_space(5.0);

                    // Unison Detune Spread Visual Bar
                    let (viz_r, _) =
                        ui.allocate_exact_size(Vec2::new(130.0, 62.0), Sense::hover());
                    draw_unison_spread_viz(
                        ui.painter(),
                        viz_r,
                        osc.unison_count.value() as usize,
                        osc.unison_detune.value(),
                        osc.unison_density.value(),
                    );
                });

                // --- SUBPANEL C: MIX & PHASE ---
                draw_sub_box(ui, "MIX & PHASE", P5_RED, mix_w, body_h, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        p5_knob_sized(ui, "WIDE", &osc.wide, setter, ui_state, P5_CYAN, 64.0, 32.0, false, midi_learn);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 3.0;
                            p5_toggle_pill(ui, "ANA", &osc.ana, setter, ui_state, 60.0, 18.0, P5_GOLD);
                            p5_toggle_pill(ui, "INV", &osc.inv, setter, ui_state, 60.0, 18.0, P5_MAGENTA);
                            p5_toggle_pill(ui, "KEY", &osc.key, setter, ui_state, 60.0, 18.0, P5_CYAN);
                        });
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        p5_knob_sized(ui, "PAN", &osc.pan, setter, ui_state, P5_WHITE, 64.0, 32.0, true, midi_learn);
                        p5_knob_sized(ui, "FLT 1-2", &osc.filter_balance, setter, ui_state, P5_RED, 64.0, 32.0, false, midi_learn);
                    });
                    ui.add_space(6.0);
                    p5_slider(
                        ui,
                        "LEVEL",
                        &osc.mix_level,
                        setter,
                        ui_state,
                        P5_RED,
                        mix_w - 14.0,
                        22.0,
                        false,
                    );
                });
            });
        },
    );
    state.active_osc = next_osc;
}

fn draw_unison_spread_viz(
    painter: &egui::Painter,
    rect: Rect,
    voices: usize,
    detune: f32,
    density: f32,
) {
    painter.rect_filled(rect, CornerRadius::ZERO, P5_BG);
    painter.rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.2_f32, P5_DIM), StrokeKind::Inside);

    let cx = rect.center().x;
    let max_spread = rect.width() * 0.42 * (0.12 + 0.88 * detune);
    let v = voices.clamp(1, 9);
    for i in 0..v {
        let frac = if v == 1 {
            0.0
        } else {
            (i as f32 / (v - 1) as f32) * 2.0 - 1.0
        };
        let shaped = frac.signum() * frac.abs().powf(0.6 + (1.0 - density) * 1.2);
        let x = cx + shaped * max_spread;
        let h = rect.height() * (0.72 - frac.abs() * 0.25);
        let col = if i == v / 2 { P5_WHITE } else if i % 2 == 0 { P5_CYAN } else { P5_GOLD };
        painter.line_segment(
            [Pos2::new(x, rect.bottom() - 5.0), Pos2::new(x, rect.bottom() - 5.0 - h)],
            Stroke::new(3.0_f32, col),
        );
    }
    painter.text(
        Pos2::new(rect.left() + 4.0, rect.top() + 3.0),
        egui::Align2::LEFT_TOP,
        format!("{v}V SPREAD"),
        font_id(8.0, FONT_HEAVY),
        P5_MUTED,
    );
}

// ============================================================================
// 02 // DUAL FILTER 1 & 2 Card
// ============================================================================

#[allow(clippy::too_many_arguments)]
fn draw_filter_card(
    ui: &mut egui::Ui,
    height: f32,
    params: &PhantomSpireParams,
    ui_state: &SharedUiState,
    setter: &ParamSetter,
    popups: &mut PopupOverlayState,
    midi_learn: &mut Option<String>,
) {
    draw_cutout_card(
        ui,
        "02 // DUAL FILTER",
        P5_RED,
        P5_WHITE,
        height,
        |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            p5_toggle_pill(
                ui,
                "LINK",
                &params.filter_routing.link,
                setter,
                ui_state,
                44.0,
                20.0,
                P5_CYAN,
            );
            p5_toggle_pill(
                ui,
                "PAR",
                &params.filter_routing.parallel,
                setter,
                ui_state,
                42.0,
                20.0,
                P5_GOLD,
            );
        },
        |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 3.0);
            let avail_w = ui.available_width();

            // Live Dual Filter Frequency Response Plot
            let (plot_r, _) = ui.allocate_exact_size(Vec2::new(avail_w, 50.0), Sense::hover());
            draw_filter_response_plot(
                ui.painter(),
                plot_r,
                params.filters[0].filter_type.value(),
                params.filters[0].model.value(),
                params.filters[0].cutoff.value(),
                params.filters[0].resonance.value(),
                params.filters[1].filter_type.value(),
                params.filters[1].cutoff.value(),
                params.filters[1].resonance.value(),
                params.filter_routing.parallel.value(),
            );

            // Filter 1 & Filter 2 Rows
            let f_colors = [P5_CYAN, P5_RED];
            for fi in 0..2 {
                let flt = &params.filters[fi];
                let accent = f_colors[fi];
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let (tag_r, _) = ui.allocate_exact_size(Vec2::new(22.0, 20.0), Sense::hover());
                    ui.painter().rect_filled(tag_r, CornerRadius::ZERO, accent);
                    ui.painter().text(
                        tag_r.center(),
                        egui::Align2::CENTER_CENTER,
                        format!("F{}", fi + 1),
                        font_id(10.0, FONT_IMPACT),
                        if accent == P5_CYAN { P5_BLACK } else { P5_WHITE },
                    );
                    let dw = (avail_w - 30.0) * 0.5;
                    p5_enum_dropdown(
                        ui,
                        &format!("flt_{fi}_model"),
                        &flt.model,
                        setter,
                        ui_state,
                        popups,
                        dw,
                        20.0,
                        accent,
                    );
                    p5_enum_dropdown(
                        ui,
                        &format!("flt_{fi}_type"),
                        &flt.filter_type,
                        setter,
                        ui_state,
                        popups,
                        dw,
                        20.0,
                        accent,
                    );
                });

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    let kw = (avail_w - 9.0) / 4.0;
                    p5_knob_sized(ui, &format!("CUT {}", fi + 1), &flt.cutoff, setter, ui_state, accent, kw, 28.0, false, midi_learn);
                    p5_knob_sized(ui, &format!("RES {}", fi + 1), &flt.resonance, setter, ui_state, P5_RED, kw, 28.0, false, midi_learn);
                    p5_knob_sized(ui, "KEYTRK", &flt.keytrack, setter, ui_state, P5_GOLD, kw, 28.0, true, midi_learn);
                    p5_knob_sized(ui, "DRIVE", &flt.drive, setter, ui_state, P5_MAGENTA, kw, 28.0, false, midi_learn);
                });
            }

            // Filter Balance Bipolar Slider
            p5_slider(
                ui,
                "FILTER BALANCE",
                &params.filter_routing.filter_balance,
                setter,
                ui_state,
                P5_GOLD,
                avail_w,
                19.0,
                true,
            );
        },
    );
}

// ============================================================================
// 03 // MOD BAY A  (ENV 1, ENV 2, LFO 1, LFO 2, STP 1)
// ============================================================================

#[allow(clippy::too_many_arguments)]
fn draw_mod_bay_a(
    ui: &mut egui::Ui,
    height: f32,
    params: &PhantomSpireParams,
    ui_state: &SharedUiState,
    shared_synth: &SharedSynthState,
    setter: &ParamSetter,
    popups: &mut PopupOverlayState,
    state: &mut SynthPageState,
    midi_learn: &mut Option<String>,
) {
    let has_env_buf = params
        .extra_state
        .lock()
        .map(|g| g.env_copy_buffer.is_some())
        .unwrap_or(false);

    let bay_a = state.bay_a;
    let mut next_bay_a = bay_a;
    draw_cutout_card(
        ui,
        "03 // MOD BAY A",
        P5_CYAN,
        P5_WHITE,
        height,
        |ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            let cur_env = match bay_a {
                ModBayASelection::Env1 => Some(0),
                ModBayASelection::Env2 => Some(1),
                _ => None,
            };
            if let Some(eidx) = cur_env {
                if p5_button(ui, "PST", 30.0, 19.0, false, P5_CYAN, has_env_buf) {
                    paste_env_from_buffer(params, eidx, setter);
                }
                if p5_button(ui, "CPY", 30.0, 19.0, false, P5_GOLD, true) {
                    copy_env_to_buffer(params, eidx);
                }
            }
            for (sel, lbl, col) in [
                (ModBayASelection::Stp1, "STP1", P5_GOLD),
                (ModBayASelection::Lfo2, "LFO2", P5_MAGENTA),
                (ModBayASelection::Lfo1, "LFO1", P5_CYAN),
                (ModBayASelection::Env2, "ENV2", P5_RED),
                (ModBayASelection::Env1, "ENV1", P5_CYAN),
            ] {
                if p5_button(ui, lbl, 39.0, 19.0, bay_a == sel, col, true) {
                    next_bay_a = sel;
                }
            }
        },
        |ui| match bay_a {
            ModBayASelection::Env1 => draw_env_bay(
                ui,
                0,
                &params.envs[0],
                ui_state,
                setter,
                popups,
                P5_CYAN,
                midi_learn,
            ),
            ModBayASelection::Env2 => draw_env_bay(
                ui,
                1,
                &params.envs[1],
                ui_state,
                setter,
                popups,
                P5_RED,
                midi_learn,
            ),
            ModBayASelection::Lfo1 => draw_lfo_bay(
                ui,
                0,
                &params.lfos[0],
                ui_state,
                setter,
                popups,
                P5_CYAN,
                midi_learn,
            ),
            ModBayASelection::Lfo2 => draw_lfo_bay(
                ui,
                1,
                &params.lfos[1],
                ui_state,
                setter,
                popups,
                P5_MAGENTA,
                midi_learn,
            ),
            ModBayASelection::Stp1 => draw_compact_stepper_bay(
                ui,
                0,
                &params.steppers[0],
                &params.extra_state,
                shared_synth,
                ui_state,
                setter,
                popups,
                P5_GOLD,
            ),
        },
    );
    state.bay_a = next_bay_a;
}

// ============================================================================
// 04 // MOD BAY B  (ENV 3, ENV 4, LFO 3, LFO 4, STP 2, ARP)
// ============================================================================

#[allow(clippy::too_many_arguments)]
fn draw_mod_bay_b(
    ui: &mut egui::Ui,
    height: f32,
    params: &PhantomSpireParams,
    ui_state: &SharedUiState,
    shared_synth: &SharedSynthState,
    setter: &ParamSetter,
    popups: &mut PopupOverlayState,
    state: &mut SynthPageState,
    midi_learn: &mut Option<String>,
) {
    let has_env_buf = params
        .extra_state
        .lock()
        .map(|g| g.env_copy_buffer.is_some())
        .unwrap_or(false);

    let bay_b = state.bay_b;
    let mut next_bay_b = bay_b;
    draw_cutout_card(
        ui,
        "04 // MOD BAY B",
        P5_GOLD,
        P5_WHITE,
        height,
        |ui| {
            ui.spacing_mut().item_spacing.x = 2.5;
            let cur_env = match bay_b {
                ModBayBSelection::Env3 => Some(2),
                ModBayBSelection::Env4 => Some(3),
                _ => None,
            };
            if let Some(eidx) = cur_env {
                if p5_button(ui, "PST", 28.0, 19.0, false, P5_CYAN, has_env_buf) {
                    paste_env_from_buffer(params, eidx, setter);
                }
                if p5_button(ui, "CPY", 28.0, 19.0, false, P5_GOLD, true) {
                    copy_env_to_buffer(params, eidx);
                }
            }
            for (sel, lbl, col) in [
                (ModBayBSelection::Arp, "ARP", P5_CYAN),
                (ModBayBSelection::Stp2, "STP2", P5_GOLD),
                (ModBayBSelection::Lfo4, "LFO4", P5_MAGENTA),
                (ModBayBSelection::Lfo3, "LFO3", P5_CYAN),
                (ModBayBSelection::Env4, "ENV4", P5_RED),
                (ModBayBSelection::Env3, "ENV3", P5_GOLD),
            ] {
                if p5_button(ui, lbl, 35.0, 19.0, bay_b == sel, col, true) {
                    next_bay_b = sel;
                }
            }
        },
        |ui| match bay_b {
            ModBayBSelection::Env3 => draw_env_bay(
                ui,
                2,
                &params.envs[2],
                ui_state,
                setter,
                popups,
                P5_GOLD,
                midi_learn,
            ),
            ModBayBSelection::Env4 => draw_env_bay(
                ui,
                3,
                &params.envs[3],
                ui_state,
                setter,
                popups,
                P5_MAGENTA,
                midi_learn,
            ),
            ModBayBSelection::Lfo3 => draw_lfo_bay(
                ui,
                2,
                &params.lfos[2],
                ui_state,
                setter,
                popups,
                P5_CYAN,
                midi_learn,
            ),
            ModBayBSelection::Lfo4 => draw_lfo_bay(
                ui,
                3,
                &params.lfos[3],
                ui_state,
                setter,
                popups,
                P5_MAGENTA,
                midi_learn,
            ),
            ModBayBSelection::Stp2 => draw_compact_stepper_bay(
                ui,
                1,
                &params.steppers[1],
                &params.extra_state,
                shared_synth,
                ui_state,
                setter,
                popups,
                P5_CYAN,
            ),
            ModBayBSelection::Arp => draw_compact_arp_bay(
                ui,
                params,
                shared_synth,
                ui_state,
                setter,
                popups,
            ),
        },
    );
    state.bay_b = next_bay_b;
}

// ============================================================================
// Envelope, LFO, Compact Stepper & Compact Arp Bay Renderers
// ============================================================================

#[allow(clippy::too_many_arguments)]
fn draw_env_bay(
    ui: &mut egui::Ui,
    env_idx: usize,
    env: &EnvParams,
    ui_state: &SharedUiState,
    setter: &ParamSetter,
    popups: &mut PopupOverlayState,
    accent: Color32,
    midi_learn: &mut Option<String>,
) {
    let left_w = 248.0;
    let right_w = (ui.available_width() - left_w - 6.0).max(160.0);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;

        // Left: 6-stage Envelope Plot + 6 Vertical Faders
        ui.allocate_ui_with_layout(
            Vec2::new(left_w, 206.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                let (plot_r, _) = ui.allocate_exact_size(Vec2::new(left_w, 60.0), Sense::hover());
                draw_env_plot(
                    ui.painter(),
                    plot_r,
                    env.attack.unmodulated_normalized_value(),
                    env.decay.unmodulated_normalized_value(),
                    env.sustain.unmodulated_normalized_value(),
                    env.slt.unmodulated_normalized_value(),
                    env.sll.unmodulated_normalized_value(),
                    env.release.unmodulated_normalized_value(),
                    ui_state.get_env_level(env_idx),
                    accent,
                );

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let fw = (left_w - 10.0) / 6.0;
                    p5_v_fader(ui, "ATT", &env.attack, setter, ui_state, accent, fw, 110.0);
                    p5_v_fader(ui, "DEC", &env.decay, setter, ui_state, accent, fw, 110.0);
                    p5_v_fader(ui, "SUS", &env.sustain, setter, ui_state, P5_GOLD, fw, 110.0);
                    p5_v_fader(ui, "SLT", &env.slt, setter, ui_state, P5_CYAN, fw, 110.0);
                    p5_v_fader(ui, "SLL", &env.sll, setter, ui_state, P5_CYAN, fw, 110.0);
                    p5_v_fader(ui, "REL", &env.release, setter, ui_state, P5_RED, fw, 110.0);
                });
            },
        );

        // Right: VEL bar + 2 Direct Mod Slots
        ui.allocate_ui_with_layout(
            Vec2::new(right_w, 206.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                p5_slider(ui, "ENV VEL", &env.vel, setter, ui_state, P5_GOLD, right_w, 18.0, false);

                for si in 0..2 {
                    let slot = &env.mod_slots[si];
                    p5_enum_dropdown(
                        ui,
                        &format!("env_{env_idx}_slot_{si}"),
                        &slot.target,
                        setter,
                        ui_state,
                        popups,
                        right_w,
                        20.0,
                        accent,
                    );
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let kw = (right_w - 4.0) * 0.5;
                        p5_knob_sized(
                            ui,
                            &format!("AMT {}", si + 1),
                            &slot.amt,
                            setter,
                            ui_state,
                            P5_GOLD,
                            kw,
                            28.0,
                            true,
                            midi_learn,
                        );
                        p5_knob_sized(
                            ui,
                            &format!("VEL {}", si + 1),
                            &slot.vel,
                            setter,
                            ui_state,
                            P5_CYAN,
                            kw,
                            28.0,
                            false,
                            midi_learn,
                        );
                    });
                }
            },
        );
    });
}

#[allow(clippy::too_many_arguments)]
fn draw_lfo_bay(
    ui: &mut egui::Ui,
    lfo_idx: usize,
    lfo: &LfoParams,
    ui_state: &SharedUiState,
    setter: &ParamSetter,
    popups: &mut PopupOverlayState,
    accent: Color32,
    midi_learn: &mut Option<String>,
) {
    let left_w = 252.0;
    let right_w = (ui.available_width() - left_w - 6.0).max(156.0);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;

        ui.allocate_ui_with_layout(
            Vec2::new(left_w, 206.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                let (plot_r, _) = ui.allocate_exact_size(Vec2::new(left_w, 58.0), Sense::hover());
                draw_lfo_plot(
                    ui.painter(),
                    plot_r,
                    lfo.waveform.value(),
                    lfo.morph.value(),
                    lfo.sym.value(),
                    lfo.phase.value(),
                    lfo.unipolar.value(),
                    ui_state.get_lfo_phase(lfo_idx),
                    accent,
                );

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    p5_enum_dropdown(
                        ui,
                        &format!("lfo_{lfo_idx}_wave"),
                        &lfo.waveform,
                        setter,
                        ui_state,
                        popups,
                        108.0,
                        20.0,
                        accent,
                    );
                    p5_toggle_pill(ui, "SYNC", &lfo.sync, setter, ui_state, 44.0, 20.0, P5_GOLD);
                    p5_toggle_pill(ui, "UNI", &lfo.unipolar, setter, ui_state, 40.0, 20.0, P5_CYAN);
                    p5_toggle_pill(ui, "MONO", &lfo.mono, setter, ui_state, 48.0, 20.0, P5_RED);
                });

                if lfo.sync.value() {
                    p5_enum_dropdown(
                        ui,
                        &format!("lfo_{lfo_idx}_sdiv"),
                        &lfo.sync_div,
                        setter,
                        ui_state,
                        popups,
                        left_w,
                        20.0,
                        P5_GOLD,
                    );
                } else {
                    p5_slider(ui, "RATE", &lfo.rate, setter, ui_state, accent, left_w, 20.0, false);
                }

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    let kw = (left_w - 9.0) / 4.0;
                    p5_knob_sized(ui, "MORPH", &lfo.morph, setter, ui_state, P5_CYAN, kw, 30.0, false, midi_learn);
                    p5_knob_sized(ui, "SYM", &lfo.sym, setter, ui_state, P5_GOLD, kw, 30.0, false, midi_learn);
                    p5_knob_sized(ui, "PHASE", &lfo.phase, setter, ui_state, P5_MAGENTA, kw, 30.0, false, midi_learn);
                    p5_knob_sized(ui, "FADE", &lfo.fade, setter, ui_state, P5_WHITE, kw, 30.0, false, midi_learn);
                });
            },
        );

        ui.allocate_ui_with_layout(
            Vec2::new(right_w, 206.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.spacing_mut().item_spacing.y = 5.0;
                for si in 0..2 {
                    let slot = &lfo.mod_slots[si];
                    p5_enum_dropdown(
                        ui,
                        &format!("lfo_{lfo_idx}_slot_{si}"),
                        &slot.target,
                        setter,
                        ui_state,
                        popups,
                        right_w,
                        21.0,
                        accent,
                    );
                    ui.horizontal(|ui| {
                        ui.add_space((right_w - 76.0) * 0.5);
                        p5_knob_sized(
                            ui,
                            &format!("MOD AMT {}", si + 1),
                            &slot.amt,
                            setter,
                            ui_state,
                            P5_GOLD,
                            76.0,
                            36.0,
                            true,
                            midi_learn,
                        );
                    });
                }
            },
        );
    });
}

#[allow(clippy::too_many_arguments)]
fn draw_compact_stepper_bay(
    ui: &mut egui::Ui,
    stp_idx: usize,
    stp: &StepperParams,
    extra_state: &std::sync::Arc<std::sync::Mutex<ExtraSynthState>>,
    shared_synth: &SharedSynthState,
    ui_state: &SharedUiState,
    setter: &ParamSetter,
    popups: &mut PopupOverlayState,
    accent: Color32,
) {
    let w = ui.available_width();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        p5_toggle_pill(ui, "SYNC", &stp.sync, setter, ui_state, 48.0, 20.0, P5_GOLD);
        p5_toggle_pill(ui, "LOOP", &stp.loop_mode, setter, ui_state, 48.0, 20.0, P5_CYAN);
        if stp.sync.value() {
            p5_enum_dropdown(
                ui,
                &format!("bay_stp_{stp_idx}_div"),
                &stp.sync_div,
                setter,
                ui_state,
                popups,
                76.0,
                20.0,
                accent,
            );
        } else {
            p5_slider(ui, "HZ", &stp.rate, setter, ui_state, accent, 76.0, 20.0, false);
        }
        p5_int_stepper(ui, "N", &stp.steps_count, setter, ui_state, 66.0, 20.0, accent);
        let rem = (w - 254.0).max(120.0) * 0.5;
        p5_slider(ui, "GATE", &stp.gate, setter, ui_state, P5_CYAN, rem, 20.0, false);
        p5_slider(ui, "SMTH", &stp.smooth, setter, ui_state, P5_MAGENTA, rem, 20.0, false);
    });

    ui.add_space(5.0);

    // Interactive 16-bar stepper paint strip
    let active_step = ui_state.stp_steps[stp_idx].load(Ordering::Relaxed) as usize;
    let step_count = stp.steps_count.value().clamp(2, 16) as usize;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 168.0), Sense::click_and_drag());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::ZERO, P5_BG);

    let mut steps_copy = [0.5_f32; 16];
    if let Ok(extra) = extra_state.lock() {
        steps_copy = extra.stepper_steps[stp_idx];
    }

    if (resp.dragged() || resp.clicked()) && ui.input(|i| i.pointer.primary_down()) {
        if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
            let col = (((pos.x - rect.left()) / rect.width().max(1.0)) * 16.0).floor() as isize;
            if (0..16).contains(&col) {
                let val = (1.0 - (pos.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
                steps_copy[col as usize] = val;
                if let Ok(mut extra) = extra_state.lock() {
                    extra.stepper_steps[stp_idx][col as usize] = val;
                    shared_synth.sync_from_extra(&extra);
                }
                ui_state.set_last_param_label(&format!(
                    "STP{} STEP {}: {:.0}%",
                    stp_idx + 1,
                    col + 1,
                    val * 100.0
                ));
            }
        }
    }

    let bar_w = rect.width() / 16.0;
    for i in 0..16 {
        let x0 = rect.left() + i as f32 * bar_w;
        let col_r = Rect::from_min_size(Pos2::new(x0 + 1.5, rect.top() + 2.0), Vec2::new(bar_w - 3.0, rect.height() - 4.0));
        let in_len = i < step_count;
        let is_play = i == active_step;
        p.rect_filled(
            col_r,
            CornerRadius::ZERO,
            if is_play {
                Color32::from_rgb(38, 38, 52)
            } else if in_len {
                P5_PANEL
            } else {
                Color32::from_rgb(12, 12, 16)
            },
        );
        let fill_h = col_r.height() * steps_copy[i];
        let bar_r = Rect::from_min_max(
            Pos2::new(col_r.left(), col_r.bottom() - fill_h),
            col_r.max,
        );
        p.rect_filled(
            bar_r,
            CornerRadius::ZERO,
            if !in_len {
                P5_DIM
            } else if is_play {
                P5_WHITE
            } else {
                accent
            },
        );
    }

    p.rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.5_f32, P5_WHITE), StrokeKind::Inside);
}

#[allow(clippy::too_many_arguments)]
fn draw_compact_arp_bay(
    ui: &mut egui::Ui,
    params: &PhantomSpireParams,
    shared_synth: &SharedSynthState,
    ui_state: &SharedUiState,
    setter: &ParamSetter,
    popups: &mut PopupOverlayState,
) {
    let arp = &params.arp;
    let w = ui.available_width();

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        p5_diamond_pill(ui, "ARP ON", &arp.enabled, setter, ui_state, 74.0, 22.0, P5_CYAN, false);
        p5_toggle_pill(ui, "LATCH", &arp.latch, setter, ui_state, 52.0, 22.0, P5_GOLD);
        p5_toggle_pill(ui, "SYNC", &arp.sync, setter, ui_state, 46.0, 22.0, P5_RED);
        p5_enum_dropdown(ui, "bay_arp_mode", &arp.mode, setter, ui_state, popups, 84.0, 22.0, P5_CYAN);
        p5_int_stepper(ui, "OCT", &arp.octaves, setter, ui_state, 72.0, 22.0, P5_GOLD);
        p5_int_stepper(ui, "N", &arp.steps_count, setter, ui_state, 66.0, 22.0, P5_WHITE);
    });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if arp.sync.value() {
            p5_enum_dropdown(ui, "bay_arp_div", &arp.sync_div, setter, ui_state, popups, 82.0, 20.0, P5_GOLD);
        } else {
            p5_slider(ui, "RATE", &arp.rate, setter, ui_state, P5_GOLD, 82.0, 20.0, false);
        }
        let sw = (w - 94.0) / 3.0;
        p5_slider(ui, "GATE", &arp.gate, setter, ui_state, P5_CYAN, sw, 20.0, false);
        p5_slider(ui, "SWING", &arp.swing, setter, ui_state, P5_MAGENTA, sw, 20.0, false);
        p5_slider(ui, "TEMPO", &arp.internal_bpm, setter, ui_state, P5_RED, sw, 20.0, false);
    });

    ui.add_space(6.0);

    // 16 clickable step gate pills
    let active_step = ui_state.arp_step.load(Ordering::Relaxed) as usize;
    let step_count = arp.steps_count.value().clamp(1, 16) as usize;
    let mut mask = params
        .extra_state
        .lock()
        .map(|g| g.arp_steps_enabled)
        .unwrap_or(0xFFFF);

    let (grid_r, _) = ui.allocate_exact_size(Vec2::new(w, 136.0), Sense::hover());
    ui.painter().rect_filled(grid_r, CornerRadius::ZERO, P5_BG);
    ui.painter().rect_stroke(grid_r, CornerRadius::ZERO, Stroke::new(1.4_f32, P5_WHITE), StrokeKind::Inside);

    let mut sub = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(grid_r.shrink(6.0))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    sub.painter().text(
        Pos2::new(grid_r.left() + 8.0, grid_r.top() + 14.0),
        egui::Align2::LEFT_CENTER,
        "16-STEP ARP GATE MATRIX (CLICK STEPS TO TOGGLE — FULL EDITOR IN MOD & ARP TAB)",
        font_id(8.5, FONT_HEAVY),
        P5_MUTED,
    );
    sub.add_space(18.0);

    let cell_w = (grid_r.width() - 12.0 - 15.0 * 2.0) / 16.0;
    sub.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for i in 0..16 {
            let on = (mask & (1u16 << i)) != 0;
            let in_len = i < step_count;
            let is_play = i == active_step;
            let (cr, resp) = ui.allocate_exact_size(Vec2::new(cell_w, 96.0), Sense::click());
            if resp.clicked() {
                mask ^= 1u16 << i;
                if let Ok(mut extra) = params.extra_state.lock() {
                    extra.arp_steps_enabled = mask;
                    shared_synth.sync_from_extra(&extra);
                }
            }
            let cp = ui.painter();
            let fill = if !in_len {
                Color32::from_rgb(16, 16, 20)
            } else if is_play {
                P5_GOLD
            } else if on {
                P5_CYAN
            } else {
                P5_SURFACE
            };
            cp.rect(
                cr,
                CornerRadius::ZERO,
                fill,
                Stroke::new(1.2_f32, if is_play { P5_WHITE } else { P5_BLACK }),
                StrokeKind::Inside,
            );
            cp.text(
                Pos2::new(cr.center().x, cr.bottom() - 10.0),
                egui::Align2::CENTER_CENTER,
                format!("{}", i + 1),
                font_id(8.5, FONT_HEAVY),
                if on || is_play { P5_BLACK } else { P5_MUTED },
            );
        }
    });
}

fn draw_sub_box(
    ui: &mut egui::Ui,
    title: &str,
    accent: Color32,
    width: f32,
    height: f32,
    body: impl FnOnce(&mut egui::Ui),
) {
    ui.allocate_ui_with_layout(
        Vec2::new(width, height),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            egui::Frame::new()
                .fill(P5_PANEL)
                .stroke(Stroke::new(1.3_f32, Color32::from_rgb(66, 66, 82)))
                .inner_margin(egui::Margin::symmetric(5, 4))
                .show(ui, |ui| {
                    ui.set_min_width(width - 10.0);
                    ui.set_max_width(width - 10.0);
                    ui.set_min_height(height - 8.0);
                    ui.set_max_height(height - 8.0);
                    let (hr, _) = ui.allocate_exact_size(Vec2::new(width - 12.0, 14.0), Sense::hover());
                    ui.painter().text(
                        Pos2::new(hr.left() + 2.0, hr.center().y),
                        egui::Align2::LEFT_CENTER,
                        title,
                        font_id(9.5, FONT_IMPACT),
                        accent,
                    );
                    ui.add_space(2.0);
                    body(ui);
                });
        },
    );
}

// ============================================================================
// OSC & ENV Copy/Paste Buffer Helpers
// ============================================================================

fn copy_osc_to_buffer(params: &PhantomSpireParams, idx: usize) {
    let o: &OscParams = &params.oscs[idx];
    let mut map = BTreeMap::new();
    map.insert("mode".into(), serde_json::json!(o.mode.value().to_index()));
    map.insert("waveform".into(), serde_json::json!(o.waveform.value().to_index()));
    map.insert("wt_mix".into(), serde_json::json!(o.wt_mix.value()));
    map.insert("note".into(), serde_json::json!(o.note.value()));
    map.insert("fine".into(), serde_json::json!(o.fine.value()));
    map.insert("octave".into(), serde_json::json!(o.octave.value()));
    map.insert("ctrl_a".into(), serde_json::json!(o.ctrl_a.value()));
    map.insert("ctrl_b".into(), serde_json::json!(o.ctrl_b.value()));
    map.insert("phase".into(), serde_json::json!(o.phase.value()));
    map.insert("unison_count".into(), serde_json::json!(o.unison_count.value()));
    map.insert("unison_detune".into(), serde_json::json!(o.unison_detune.value()));
    map.insert("unison_density".into(), serde_json::json!(o.unison_density.value()));
    map.insert("unison_mode".into(), serde_json::json!(o.unison_mode.value().to_index()));
    map.insert("wide".into(), serde_json::json!(o.wide.value()));
    map.insert("pan".into(), serde_json::json!(o.pan.value()));
    map.insert("mix_level".into(), serde_json::json!(o.mix_level.value()));
    map.insert("filter_balance".into(), serde_json::json!(o.filter_balance.value()));
    map.insert("ana".into(), serde_json::json!(o.ana.value()));
    map.insert("inv".into(), serde_json::json!(o.inv.value()));
    map.insert("key".into(), serde_json::json!(o.key.value()));
    if let Ok(mut extra) = params.extra_state.lock() {
        extra.osc_copy_buffer = Some(map);
    }
}

fn paste_osc_from_buffer(params: &PhantomSpireParams, idx: usize, setter: &ParamSetter) {
    let Some(map) = params
        .extra_state
        .lock()
        .ok()
        .and_then(|g| g.osc_copy_buffer.clone())
    else {
        return;
    };
    let o = &params.oscs[idx];
    let set_f = |p: &FloatParam, key: &str| {
        if let Some(v) = map.get(key).and_then(|v| v.as_f64()) {
            setter.begin_set_parameter(p);
            setter.set_parameter(p, v as f32);
            setter.end_set_parameter(p);
        }
    };
    let set_i = |p: &IntParam, key: &str| {
        if let Some(v) = map.get(key).and_then(|v| v.as_i64()) {
            setter.begin_set_parameter(p);
            setter.set_parameter(p, v as i32);
            setter.end_set_parameter(p);
        }
    };
    let set_b = |p: &BoolParam, key: &str| {
        if let Some(v) = map.get(key).and_then(|v| v.as_bool()) {
            setter.begin_set_parameter(p);
            setter.set_parameter(p, v);
            setter.end_set_parameter(p);
        }
    };
    if let Some(i) = map.get("mode").and_then(|v| v.as_u64()) {
        setter.begin_set_parameter(&o.mode);
        setter.set_parameter(&o.mode, crate::params::OscMode::from_index(i as usize));
        setter.end_set_parameter(&o.mode);
    }
    if let Some(i) = map.get("waveform").and_then(|v| v.as_u64()) {
        setter.begin_set_parameter(&o.waveform);
        setter.set_parameter(&o.waveform, crate::params::OscWaveform::from_index(i as usize));
        setter.end_set_parameter(&o.waveform);
    }
    if let Some(i) = map.get("unison_mode").and_then(|v| v.as_u64()) {
        setter.begin_set_parameter(&o.unison_mode);
        setter.set_parameter(&o.unison_mode, crate::params::UnisonMode::from_index(i as usize));
        setter.end_set_parameter(&o.unison_mode);
    }
    set_f(&o.wt_mix, "wt_mix");
    set_i(&o.note, "note");
    set_f(&o.fine, "fine");
    set_i(&o.octave, "octave");
    set_f(&o.ctrl_a, "ctrl_a");
    set_f(&o.ctrl_b, "ctrl_b");
    set_f(&o.phase, "phase");
    set_i(&o.unison_count, "unison_count");
    set_f(&o.unison_detune, "unison_detune");
    set_f(&o.unison_density, "unison_density");
    set_f(&o.wide, "wide");
    set_f(&o.pan, "pan");
    set_f(&o.mix_level, "mix_level");
    set_f(&o.filter_balance, "filter_balance");
    set_b(&o.ana, "ana");
    set_b(&o.inv, "inv");
    set_b(&o.key, "key");
}

fn copy_env_to_buffer(params: &PhantomSpireParams, idx: usize) {
    let e = &params.envs[idx];
    let mut map = BTreeMap::new();
    map.insert("attack".into(), serde_json::json!(e.attack.value()));
    map.insert("decay".into(), serde_json::json!(e.decay.value()));
    map.insert("sustain".into(), serde_json::json!(e.sustain.value()));
    map.insert("slt".into(), serde_json::json!(e.slt.value()));
    map.insert("sll".into(), serde_json::json!(e.sll.value()));
    map.insert("release".into(), serde_json::json!(e.release.value()));
    map.insert("vel".into(), serde_json::json!(e.vel.value()));
    if let Ok(mut extra) = params.extra_state.lock() {
        extra.env_copy_buffer = Some(map);
    }
}

fn paste_env_from_buffer(params: &PhantomSpireParams, idx: usize, setter: &ParamSetter) {
    let Some(map) = params
        .extra_state
        .lock()
        .ok()
        .and_then(|g| g.env_copy_buffer.clone())
    else {
        return;
    };
    let e = &params.envs[idx];
    for (p, key) in [
        (&e.attack, "attack"),
        (&e.decay, "decay"),
        (&e.sustain, "sustain"),
        (&e.slt, "slt"),
        (&e.sll, "sll"),
        (&e.release, "release"),
        (&e.vel, "vel"),
    ] {
        if let Some(v) = map.get(key).and_then(|v| v.as_f64()) {
            setter.begin_set_parameter(p);
            setter.set_parameter(p, v as f32);
            setter.end_set_parameter(p);
        }
    }
}
