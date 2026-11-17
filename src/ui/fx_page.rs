use crate::params::PhantomSpireParams;
use crate::ui::theme::{
    font_id, FONT_HEAVY, FONT_IMPACT, P5_CYAN, P5_GOLD, P5_MAGENTA, P5_MUTED, P5_RED, P5_WHITE,
};
use crate::ui::widgets::{
    draw_cutout_card, draw_eq_plot, draw_stereo_meter, p5_button, p5_diamond_pill,
    p5_enum_dropdown, p5_int_stepper, p5_knob_sized, p5_toggle_pill, PopupOverlayState,
};
use crate::SharedUiState;
use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, Pos2, Sense, Vec2};

#[derive(Default)]
pub struct FxPageState {
    pub active_eq_band: usize, // 0 = Low, 1 = Mid, 2 = High
}

#[allow(clippy::too_many_arguments)]
pub fn draw_fx_page(
    ui: &mut egui::Ui,
    width: f32,
    height: f32,
    params: &PhantomSpireParams,
    ui_state: &SharedUiState,
    setter: &ParamSetter,
    popups: &mut PopupOverlayState,
    state: &mut FxPageState,
    midi_learn: &mut Option<String>,
) {
    let row1_h = 166.0_f32;
    let row2_h = 154.0_f32;
    let row3_h = (height - row1_h - row2_h - 14.0).max(200.0);

    // ========================================================================
    // ROW 1: SHP | PHS | CHR (All visible simultaneously w/ MUTE pills!)
    // ========================================================================
    let col3_w = (width - 16.0) / 3.0;
    ui.allocate_ui_with_layout(
        Vec2::new(width, row1_h),
        egui::Layout::left_to_right(egui::Align::Min),
        |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

            // --- 01 // SHAPER (SHP) ---
            ui.allocate_ui_with_layout(
                Vec2::new(col3_w, row1_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let shp = &params.fx.shaper;
                    draw_cutout_card(
                        ui,
                        "01 // SHP",
                        P5_RED,
                        if shp.mute.value() { P5_MUTED } else { P5_WHITE },
                        row1_h,
                        |ui| {
                            p5_diamond_pill(ui, "MUTE", &shp.mute, setter, ui_state, 66.0, 20.0, P5_RED, false);
                            p5_toggle_pill(ui, "HQ", &shp.hq, setter, ui_state, 30.0, 20.0, P5_CYAN);
                            p5_toggle_pill(ui, "BND", &shp.band, setter, ui_state, 36.0, 20.0, P5_GOLD);
                        },
                        |ui| {
                            ui.spacing_mut().item_spacing.y = 3.0;
                            p5_enum_dropdown(
                                ui,
                                "fx_shp_mode",
                                &shp.mode,
                                setter,
                                ui_state,
                                popups,
                                ui.available_width(),
                                20.0,
                                P5_RED,
                            );
                            let kw = (ui.available_width() - 8.0) / 3.0;
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                p5_knob_sized(ui, "DRIVE", &shp.drive, setter, ui_state, P5_RED, kw, 25.0, false, midi_learn);
                                p5_knob_sized(ui, "BIT", &shp.bit, setter, ui_state, P5_GOLD, kw, 25.0, false, midi_learn);
                                p5_knob_sized(ui, "S.RATE", &shp.s_rate, setter, ui_state, P5_CYAN, kw, 25.0, false, midi_learn);
                            });
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                p5_knob_sized(ui, "LO CUT", &shp.low_cut, setter, ui_state, P5_WHITE, kw, 25.0, false, midi_learn);
                                p5_knob_sized(ui, "HI CUT", &shp.hi_cut, setter, ui_state, P5_WHITE, kw, 25.0, false, midi_learn);
                                p5_knob_sized(ui, "DRY/WET", &shp.dry_wet, setter, ui_state, P5_GOLD, kw, 25.0, false, midi_learn);
                            });
                        },
                    );
                },
            );

            // --- 02 // PHASER (PHS) ---
            ui.allocate_ui_with_layout(
                Vec2::new(col3_w, row1_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let phs = &params.fx.phaser;
                    draw_cutout_card(
                        ui,
                        "02 // PHS",
                        P5_MAGENTA,
                        if phs.mute.value() { P5_MUTED } else { P5_WHITE },
                        row1_h,
                        |ui| {
                            p5_diamond_pill(ui, "MUTE", &phs.mute, setter, ui_state, 66.0, 20.0, P5_RED, false);
                        },
                        |ui| {
                            ui.spacing_mut().item_spacing.y = 3.0;
                            p5_int_stepper(
                                ui,
                                "STAGES",
                                &phs.stages,
                                setter,
                                ui_state,
                                ui.available_width(),
                                20.0,
                                P5_MAGENTA,
                            );
                            let kw = (ui.available_width() - 8.0) / 3.0;
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                p5_knob_sized(ui, "RATE", &phs.rate, setter, ui_state, P5_MAGENTA, kw, 25.0, false, midi_learn);
                                p5_knob_sized(ui, "FEEDBK", &phs.feedback, setter, ui_state, P5_RED, kw, 25.0, true, midi_learn);
                                p5_knob_sized(ui, "FREQ", &phs.freq, setter, ui_state, P5_CYAN, kw, 25.0, false, midi_learn);
                            });
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                let kw2 = (ui.available_width() - 4.0) * 0.5;
                                p5_knob_sized(ui, "SPREAD", &phs.spread, setter, ui_state, P5_WHITE, kw2, 25.0, false, midi_learn);
                                p5_knob_sized(ui, "DRY/WET", &phs.dry_wet, setter, ui_state, P5_GOLD, kw2, 25.0, false, midi_learn);
                            });
                        },
                    );
                },
            );

            // --- 03 // CHORUS (CHR) ---
            ui.allocate_ui_with_layout(
                Vec2::new(col3_w, row1_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let chr = &params.fx.chorus;
                    draw_cutout_card(
                        ui,
                        "03 // CHR",
                        P5_CYAN,
                        if chr.mute.value() { P5_MUTED } else { P5_WHITE },
                        row1_h,
                        |ui| {
                            p5_diamond_pill(ui, "MUTE", &chr.mute, setter, ui_state, 66.0, 20.0, P5_RED, false);
                        },
                        |ui| {
                            ui.spacing_mut().item_spacing.y = 3.0;
                            p5_enum_dropdown(
                                ui,
                                "fx_chr_mode",
                                &chr.mode,
                                setter,
                                ui_state,
                                popups,
                                ui.available_width(),
                                20.0,
                                P5_CYAN,
                            );
                            let kw4 = (ui.available_width() - 9.0) / 4.0;
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 3.0;
                                p5_knob_sized(ui, "RATE", &chr.rate, setter, ui_state, P5_CYAN, kw4, 25.0, false, midi_learn);
                                p5_knob_sized(ui, "DELAY", &chr.delay, setter, ui_state, P5_WHITE, kw4, 25.0, false, midi_learn);
                                p5_knob_sized(ui, "DEPTH", &chr.depth, setter, ui_state, P5_GOLD, kw4, 25.0, false, midi_learn);
                                p5_knob_sized(ui, "FDBK", &chr.feedback, setter, ui_state, P5_RED, kw4, 25.0, true, midi_learn);
                            });
                            let kw3 = (ui.available_width() - 8.0) / 3.0;
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                p5_knob_sized(ui, "LO CUT", &chr.low_cut, setter, ui_state, P5_WHITE, kw3, 25.0, false, midi_learn);
                                p5_knob_sized(ui, "HI CUT", &chr.hi_cut, setter, ui_state, P5_WHITE, kw3, 25.0, false, midi_learn);
                                p5_knob_sized(ui, "DRY/WET", &chr.dry_wet, setter, ui_state, P5_GOLD, kw3, 25.0, false, midi_learn);
                            });
                        },
                    );
                },
            );
        },
    );

    ui.add_space(6.0);

    // ========================================================================
    // ROW 2: DEL | REV
    // ========================================================================
    let del_w = 486.0_f32;
    let rev_w = (width - del_w - 8.0).max(380.0);
    ui.allocate_ui_with_layout(
        Vec2::new(width, row2_h),
        egui::Layout::left_to_right(egui::Align::Min),
        |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

            // --- 04 // DELAY (DEL) ---
            ui.allocate_ui_with_layout(
                Vec2::new(del_w, row2_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let del = &params.fx.delay;
                    draw_cutout_card(
                        ui,
                        "04 // DEL",
                        P5_GOLD,
                        if del.mute.value() { P5_MUTED } else { P5_WHITE },
                        row2_h,
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            p5_diamond_pill(ui, "MUTE", &del.mute, setter, ui_state, 66.0, 20.0, P5_RED, false);
                            p5_toggle_pill(ui, "P.PONG", &del.ping_pong, setter, ui_state, 56.0, 20.0, P5_CYAN);
                            p5_toggle_pill(ui, "SYNC", &del.sync, setter, ui_state, 46.0, 20.0, P5_GOLD);
                        },
                        |ui| {
                            ui.spacing_mut().item_spacing.y = 4.0;
                            let avail_w = ui.available_width();
                            if del.sync.value() {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 6.0;
                                    let dw = (avail_w - 6.0) * 0.5;
                                    p5_enum_dropdown(ui, "fx_del_div_l", &del.sync_div_l, setter, ui_state, popups, dw, 20.0, P5_GOLD);
                                    p5_enum_dropdown(ui, "fx_del_div_r", &del.sync_div_r, setter, ui_state, popups, dw, 20.0, P5_GOLD);
                                });
                                ui.add_space(4.0);
                                let kw = (avail_w - 20.0) / 6.0;
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    p5_knob_sized(ui, "FDBK", &del.feedback, setter, ui_state, P5_RED, kw, 32.0, false, midi_learn);
                                    p5_knob_sized(ui, "RATE", &del.rate, setter, ui_state, P5_CYAN, kw, 32.0, false, midi_learn);
                                    p5_knob_sized(ui, "DEPTH", &del.depth, setter, ui_state, P5_MAGENTA, kw, 32.0, false, midi_learn);
                                    p5_knob_sized(ui, "LO CUT", &del.low_cut, setter, ui_state, P5_WHITE, kw, 32.0, false, midi_learn);
                                    p5_knob_sized(ui, "HI CUT", &del.hi_cut, setter, ui_state, P5_WHITE, kw, 32.0, false, midi_learn);
                                    p5_knob_sized(ui, "DRY/WET", &del.dry_wet, setter, ui_state, P5_GOLD, kw, 32.0, false, midi_learn);
                                });
                            } else {
                                let kw = (avail_w - 21.0) / 8.0;
                                ui.add_space(16.0);
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 3.0;
                                    p5_knob_sized(ui, "TIME L", &del.time_l, setter, ui_state, P5_GOLD, kw, 28.0, false, midi_learn);
                                    p5_knob_sized(ui, "TIME R", &del.time_r, setter, ui_state, P5_GOLD, kw, 28.0, false, midi_learn);
                                    p5_knob_sized(ui, "FDBK", &del.feedback, setter, ui_state, P5_RED, kw, 28.0, false, midi_learn);
                                    p5_knob_sized(ui, "RATE", &del.rate, setter, ui_state, P5_CYAN, kw, 28.0, false, midi_learn);
                                    p5_knob_sized(ui, "DEPTH", &del.depth, setter, ui_state, P5_MAGENTA, kw, 28.0, false, midi_learn);
                                    p5_knob_sized(ui, "LO CUT", &del.low_cut, setter, ui_state, P5_WHITE, kw, 28.0, false, midi_learn);
                                    p5_knob_sized(ui, "HI CUT", &del.hi_cut, setter, ui_state, P5_WHITE, kw, 28.0, false, midi_learn);
                                    p5_knob_sized(ui, "MIX", &del.dry_wet, setter, ui_state, P5_GOLD, kw, 28.0, false, midi_learn);
                                });
                            }
                        },
                    );
                },
            );

            // --- 05 // REVERB (REV) ---
            ui.allocate_ui_with_layout(
                Vec2::new(rev_w, row2_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let rev = &params.fx.reverb;
                    draw_cutout_card(
                        ui,
                        "05 // REV",
                        P5_CYAN,
                        if rev.mute.value() { P5_MUTED } else { P5_WHITE },
                        row2_h,
                        |ui| {
                            p5_diamond_pill(ui, "MUTE", &rev.mute, setter, ui_state, 66.0, 20.0, P5_RED, false);
                        },
                        |ui| {
                            ui.spacing_mut().item_spacing.y = 4.0;
                            let avail_w = ui.available_width();
                            p5_enum_dropdown(ui, "fx_rev_mode", &rev.mode, setter, ui_state, popups, avail_w, 20.0, P5_CYAN);
                            ui.add_space(4.0);
                            let kw = (avail_w - 15.0) / 6.0;
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 3.0;
                                p5_knob_sized(ui, "PREDLY", &rev.predelay, setter, ui_state, P5_WHITE, kw, 32.0, false, midi_learn);
                                p5_knob_sized(ui, "DECAY", &rev.decay, setter, ui_state, P5_CYAN, kw, 32.0, false, midi_learn);
                                p5_knob_sized(ui, "DAMP", &rev.damp, setter, ui_state, P5_MAGENTA, kw, 32.0, false, midi_learn);
                                p5_knob_sized(ui, "COLOR", &rev.color, setter, ui_state, P5_RED, kw, 32.0, true, midi_learn);
                                p5_knob_sized(ui, "WIDE", &rev.wide, setter, ui_state, P5_WHITE, kw, 32.0, false, midi_learn);
                                p5_knob_sized(ui, "DRY/WET", &rev.dry_wet, setter, ui_state, P5_GOLD, kw, 32.0, false, midi_learn);
                            });
                        },
                    );
                },
            );
        },
    );

    ui.add_space(8.0);

    // ========================================================================
    // ROW 3: MASTER DYNAMICS & CHARACTER  +  3-BAND MASTER EQ
    // ========================================================================
    let master_w = 342.0_f32;
    let eq_w = (width - master_w - 8.0).max(480.0);

    ui.allocate_ui_with_layout(
        Vec2::new(width, row3_h),
        egui::Layout::left_to_right(egui::Align::Min),
        |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

            // --- 06 // MASTER ---
            ui.allocate_ui_with_layout(
                Vec2::new(master_w, row3_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let m = &params.master;
                    draw_cutout_card(
                        ui,
                        "06 // MASTER",
                        P5_GOLD,
                        P5_WHITE,
                        row3_h,
                        |ui| {
                            p5_diamond_pill(ui, "LIMIT", &m.soft_limit, setter, ui_state, 68.0, 20.0, P5_CYAN, false);
                        },
                        |ui| {
                            let avail_w = ui.available_width();
                            ui.add_space(4.0);
                            let kw = (avail_w - 8.0) / 3.0;
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                p5_knob_sized(ui, "X-COMP", &m.x_comp, setter, ui_state, P5_GOLD, kw, 44.0, false, midi_learn);
                                p5_knob_sized(ui, "VELOCITY", &m.velocity, setter, ui_state, P5_CYAN, kw, 44.0, false, midi_learn);
                                p5_knob_sized(ui, "VOLUME", &m.volume, setter, ui_state, P5_RED, kw, 44.0, false, midi_learn);
                            });

                            ui.add_space(10.0);
                            let pw = (avail_w - 8.0) / 3.0;
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                p5_diamond_pill(ui, "WARM", &m.warm, setter, ui_state, pw, 26.0, P5_RED, false);
                                p5_diamond_pill(ui, "SOFT", &m.soft, setter, ui_state, pw, 26.0, P5_GOLD, false);
                                p5_diamond_pill(ui, "BOOST", &m.boost, setter, ui_state, pw, 26.0, P5_MAGENTA, false);
                            });

                            ui.add_space(10.0);
                            let (peak_l, peak_r) = ui_state.get_peaks();
                            let (mr, _) = ui.allocate_exact_size(Vec2::new(avail_w, 26.0), Sense::hover());
                            draw_stereo_meter(ui.painter(), mr, peak_l, peak_r);
                        },
                    );
                },
            );

            // --- 07 // 3-BAND MASTER EQ ---
            ui.allocate_ui_with_layout(
                Vec2::new(eq_w, row3_h),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let m = &params.master;
                    let active_band = state.active_eq_band;
                    let mut next_band = active_band;
                    draw_cutout_card(
                        ui,
                        "07 // 3-BAND EQ",
                        P5_CYAN,
                        if m.eq_on.value() { P5_WHITE } else { P5_MUTED },
                        row3_h,
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            p5_diamond_pill(ui, "EQ ON", &m.eq_on, setter, ui_state, 68.0, 20.0, P5_GOLD, false);
                            for (bidx, lbl, col) in [
                                (2usize, "HIGH", P5_MAGENTA),
                                (1usize, "MID", P5_RED),
                                (0usize, "LOW", P5_GOLD),
                            ] {
                                if p5_button(ui, lbl, 52.0, 20.0, active_band == bidx, col, true) {
                                    next_band = bidx;
                                }
                            }
                        },
                        |ui| {
                            let avail_w = ui.available_width();
                            let plot_w = 308.0_f32;
                            let right_w = (avail_w - plot_w - 8.0).max(190.0);

                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;

                                let (plot_r, _) =
                                    ui.allocate_exact_size(Vec2::new(plot_w, 168.0), Sense::hover());
                                draw_eq_plot(
                                    ui.painter(),
                                    plot_r,
                                    m.eq_on.value(),
                                    m.eq_low_freq.value(),
                                    m.eq_low_q.value(),
                                    m.eq_low_gain.value(),
                                    m.eq_mid_freq.value(),
                                    m.eq_mid_q.value(),
                                    m.eq_mid_gain.value(),
                                    m.eq_high_freq.value(),
                                    m.eq_high_q.value(),
                                    m.eq_high_gain.value(),
                                    active_band,
                                );

                                ui.allocate_ui_with_layout(
                                    Vec2::new(right_w, 168.0),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        let (band_title, band_col, p_freq, p_q, p_gain) =
                                            match active_band {
                                                0 => ("LOW SHELF BAND", P5_GOLD, &m.eq_low_freq, &m.eq_low_q, &m.eq_low_gain),
                                                1 => ("MID PEAK BAND", P5_RED, &m.eq_mid_freq, &m.eq_mid_q, &m.eq_mid_gain),
                                                _ => ("HIGH SHELF BAND", P5_MAGENTA, &m.eq_high_freq, &m.eq_high_q, &m.eq_high_gain),
                                            };

                                        let (br, _) = ui.allocate_exact_size(
                                            Vec2::new(right_w, 18.0),
                                            Sense::hover(),
                                        );
                                        ui.painter().text(
                                            Pos2::new(br.left() + 2.0, br.center().y),
                                            egui::Align2::LEFT_CENTER,
                                            band_title,
                                            font_id(11.0, FONT_IMPACT),
                                            band_col,
                                        );

                                        ui.add_space(6.0);
                                        let kw = (right_w - 8.0) / 3.0;
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 4.0;
                                            p5_knob_sized(ui, "FREQ", p_freq, setter, ui_state, P5_CYAN, kw, 42.0, false, midi_learn);
                                            p5_knob_sized(ui, "Q", p_q, setter, ui_state, P5_GOLD, kw, 42.0, false, midi_learn);
                                            p5_knob_sized(ui, "LEVEL", p_gain, setter, ui_state, band_col, kw, 42.0, true, midi_learn);
                                        });

                                        ui.add_space(12.0);
                                        let (sr, _) = ui.allocate_exact_size(
                                            Vec2::new(right_w, 42.0),
                                            Sense::hover(),
                                        );
                                        let sp = ui.painter();
                                        sp.text(
                                            Pos2::new(sr.left() + 4.0, sr.top() + 8.0),
                                            egui::Align2::LEFT_CENTER,
                                            format!(
                                                "LOW: {:+.1} dB   MID: {:+.1} dB   HI: {:+.1} dB",
                                                m.eq_low_gain.value(),
                                                m.eq_mid_gain.value(),
                                                m.eq_high_gain.value()
                                            ),
                                            font_id(9.5, FONT_HEAVY),
                                            P5_WHITE,
                                        );
                                        sp.text(
                                            Pos2::new(sr.left() + 4.0, sr.top() + 26.0),
                                            egui::Align2::LEFT_CENTER,
                                            "SELECT BAND TO SCULPT MASTER EQ",
                                            font_id(8.5, FONT_HEAVY),
                                            P5_MUTED,
                                        );
                                    },
                                );
                            });
                        },
                    );
                    state.active_eq_band = next_band;
                },
            );
        },
    );
}
