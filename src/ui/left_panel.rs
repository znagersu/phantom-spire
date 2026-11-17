use crate::params::PhantomSpireParams;
use crate::presets::PresetManager;
use crate::ui::preset_browser::PresetBrowserState;
use crate::ui::theme::{
    draw_crimson_command_bg, draw_p5_3d_text, draw_p5_icon_badge, draw_ransom_logo, font_id,
    icon_for_category, FONT_HEAVY, FONT_IMPACT, P5_BLACK, P5_CARD, P5_CYAN, P5_GOLD, P5_MAGENTA,
    P5_MUTED, P5_RED, P5_RED_DEEP, P5_WHITE,
};
use crate::ui::widgets::{
    p5_button, p5_enum_dropdown, p5_int_stepper, p5_knob_sized, p5_toggle_pill, p5_wheel,
    PopupOverlayState,
};
use crate::updater::{UpdateStatus, UpdaterHandle};
use crate::{SharedSynthState, SharedUiState};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, Color32, CornerRadius, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2,
};
use std::sync::{Arc, Mutex};

#[allow(clippy::too_many_arguments)]
pub fn draw_left_command_zone(
    ui: &mut egui::Ui,
    width: f32,
    height: f32,
    params: &PhantomSpireParams,
    ui_state: &SharedUiState,
    shared_synth: &SharedSynthState,
    presets: &Arc<Mutex<PresetManager>>,
    updater: &UpdaterHandle,
    up_status: &UpdateStatus,
    setter: &ParamSetter,
    popups: &mut PopupOverlayState,
    browser: &mut PresetBrowserState,
    midi_learn: &mut Option<String>,
) {
    let (zone_rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter_at(zone_rect);

    draw_crimson_command_bg(&painter, zone_rect);
    draw_ransom_logo(
        &painter,
        Pos2::new(zone_rect.left() + 9.0, zone_rect.top() + 9.0),
        width - 18.0,
    );

    let content_rect = zone_rect.shrink2(Vec2::new(10.0, 10.0));
    let mut child_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(content_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child_ui.spacing_mut().item_spacing = Vec2::new(4.0, 5.0);

    // Space below ransom logo
    child_ui.add_space(46.0);

    // Section 1: "Sound Command" Preset Browser
    let (hdr_r, _) =
        child_ui.allocate_exact_size(Vec2::new(content_rect.width(), 22.0), Sense::hover());
    draw_p5_3d_text(
        child_ui.painter(),
        Pos2::new(hdr_r.left() + 3.0, hdr_r.center().y),
        egui::Align2::LEFT_CENTER,
        "Sound Command",
        14.5,
        P5_WHITE,
        P5_WHITE,
        FONT_IMPACT,
    );

    // Tilted White Active Preset Card (Exact Phantom Voicer signature card!)
    let (
        preset_name,
        preset_cat,
        preset_is_fac,
        preset_is_fav,
        preset_idx,
        preset_total,
        filter_tag,
        can_undo,
        can_redo,
    ) = {
        let mgr = presets.lock().unwrap();
        let cur = mgr.current_preset();
        let filt_tag = mgr
            .active_category()
            .map(|c| format!("FLT:{}", c.to_uppercase()))
            .unwrap_or_else(|| {
                if cur.is_factory {
                    String::from("FAC")
                } else {
                    String::from("USR")
                }
            });
        (
            cur.name.clone(),
            cur.category.clone(),
            cur.is_factory,
            cur.is_favorite,
            mgr.current_index() + 1,
            mgr.entries().len(),
            filt_tag,
            mgr.can_undo(),
            mgr.can_redo(),
        )
    };
    let _ = preset_is_fac;

    let (card_r, card_resp) =
        child_ui.allocate_exact_size(Vec2::new(content_rect.width(), 44.0), Sense::click());
    if card_resp.clicked() {
        browser.open = !browser.open;
    }
    {
        let p = child_ui.painter();
        let pill = card_r.shrink2(Vec2::new(2.0, 2.0));
        p.rect_filled(pill.translate(Vec2::new(3.5, 3.5)), CornerRadius::ZERO, P5_BLACK);
        p.rect(
            pill,
            CornerRadius::ZERO,
            P5_WHITE,
            Stroke::new(2.4_f32, P5_BLACK),
            StrokeKind::Inside,
        );

        draw_p5_icon_badge(
            p,
            Pos2::new(pill.left() + 19.0, pill.center().y),
            11.0,
            icon_for_category(&preset_cat),
        );

        // Clip preset name so it never overflows the cyan wedge
        let name_clip = Rect::from_min_max(
            Pos2::new(pill.left() + 35.0, pill.top() + 2.0),
            Pos2::new(pill.right() - 24.0, pill.bottom() - 2.0),
        );
        let np = child_ui.painter_at(name_clip);
        let title_str = if preset_is_fav {
            format!("★ {preset_name}")
        } else {
            preset_name
        };
        np.text(
            Pos2::new(pill.left() + 37.0, pill.top() + 14.0),
            egui::Align2::LEFT_CENTER,
            title_str,
            font_id(12.5, FONT_IMPACT),
            P5_BLACK,
        );
        np.text(
            Pos2::new(pill.left() + 37.0, pill.bottom() - 10.0),
            egui::Align2::LEFT_CENTER,
            format!(
                "[{}] {} {:02}/{:02}",
                preset_cat.to_uppercase(),
                filter_tag,
                preset_idx,
                preset_total
            ),
            font_id(8.5, FONT_HEAVY),
            P5_RED_DEEP,
        );

        // Cyan P5 selection wedge
        let cy = pill.center().y;
        let rx = pill.right() - 7.0;
        let wedge = vec![
            Pos2::new(rx - 12.0, cy - 7.5),
            Pos2::new(rx, cy),
            Pos2::new(rx - 12.0, cy + 7.5),
        ];
        p.add(Shape::convex_polygon(
            wedge,
            if browser.open { P5_GOLD } else { P5_CYAN },
            Stroke::new(1.4_f32, P5_BLACK),
        ));
    }

    // Preset Action Strip: < | > | MENU | INIT | UNDO | REDO
    child_ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        if p5_button(ui, "<", 24.0, 22.0, false, P5_CYAN, true) {
            let mut mgr = presets.lock().unwrap();
            let idx = mgr.select_prev();
            let _ = mgr.apply_to_gui(idx, params, setter, shared_synth);
            ui_state.set_last_param_label(&format!("PRESET: {}", mgr.current_preset().name));
        }
        if p5_button(ui, ">", 24.0, 22.0, false, P5_CYAN, true) {
            let mut mgr = presets.lock().unwrap();
            let idx = mgr.select_next();
            let _ = mgr.apply_to_gui(idx, params, setter, shared_synth);
            ui_state.set_last_param_label(&format!("PRESET: {}", mgr.current_preset().name));
        }
        if p5_button(ui, "MENU", 48.0, 22.0, browser.open, P5_GOLD, true) {
            browser.open = !browser.open;
        }
        if p5_button(ui, "INIT", 42.0, 22.0, false, P5_RED, true) {
            let mut mgr = presets.lock().unwrap();
            let _ = mgr.apply_to_gui(0, params, setter, shared_synth);
            ui_state.set_last_param_label("PRESET: Init");
        }
        if p5_button(ui, "UNDO", 48.0, 22.0, false, P5_WHITE, can_undo) {
            let mut mgr = presets.lock().unwrap();
            if mgr.undo_gui(params, setter, shared_synth) {
                ui_state.set_last_param_label("UNDO APPLIED");
            }
        }
        if p5_button(ui, "REDO", 48.0, 22.0, false, P5_WHITE, can_redo) {
            let mut mgr = presets.lock().unwrap();
            if mgr.redo_gui(params, setter, shared_synth) {
                ui_state.set_last_param_label("REDO APPLIED");
            }
        }
    });

    child_ui.add_space(4.0);

    // Section 2: MACROS 1-4 Obsidian Cutout Card
    draw_obsidian_subcard(&mut child_ui, "MACROS 1-4", P5_GOLD, 152.0, |ui| {
        let kw = (ui.available_width() - 6.0) * 0.5;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            p5_knob_sized(
                ui,
                "MACRO 1",
                &params.global.macros[0].value,
                setter,
                ui_state,
                P5_CYAN,
                kw,
                32.0,
                false,
                midi_learn,
            );
            p5_knob_sized(
                ui,
                "MACRO 2",
                &params.global.macros[1].value,
                setter,
                ui_state,
                P5_GOLD,
                kw,
                32.0,
                false,
                midi_learn,
            );
        });
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            p5_knob_sized(
                ui,
                "MACRO 3",
                &params.global.macros[2].value,
                setter,
                ui_state,
                P5_MAGENTA,
                kw,
                32.0,
                false,
                midi_learn,
            );
            p5_knob_sized(
                ui,
                "MACRO 4",
                &params.global.macros[3].value,
                setter,
                ui_state,
                P5_WHITE,
                kw,
                32.0,
                false,
                midi_learn,
            );
        });
    });

    child_ui.add_space(4.0);

    // Section 3: PERFORMANCE, TUNING, WHEELS & GLIDE Obsidian Cutout Card
    draw_obsidian_subcard(&mut child_ui, "PITCH / GLIDE / TUNE", P5_CYAN, 206.0, |ui| {
        // Microtuning + Transpose row
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if p5_enum_dropdown(
                ui,
                "left_microtuning",
                &params.global.microtuning,
                setter,
                ui_state,
                popups,
                132.0,
                22.0,
                P5_CYAN,
            ) {
                let idx = params.global.microtuning.value().to_index() as u8;
                if let Ok(mut extra) = params.extra_state.lock() {
                    extra.microtuning_preset = idx;
                    shared_synth.sync_from_extra(&extra);
                }
            }
            p5_int_stepper(
                ui,
                "TR",
                &params.global.transpose,
                setter,
                ui_state,
                96.0,
                22.0,
                P5_GOLD,
            );
        });

        ui.add_space(5.0);

        // Lower split: Left Pitch/Mod Wheels | Right Glide, Log, Bender Up/Dn, Drift
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            p5_wheel(
                ui,
                "PITCH",
                &params.global.pitch_wheel,
                setter,
                ui_state,
                34.0,
                122.0,
                P5_RED,
                true,
            );
            p5_wheel(
                ui,
                "MOD",
                &params.global.mod_wheel,
                setter,
                ui_state,
                34.0,
                122.0,
                P5_CYAN,
                false,
            );

            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    p5_knob_sized(
                        ui,
                        "GLIDE",
                        &params.global.glide,
                        setter,
                        ui_state,
                        P5_CYAN,
                        68.0,
                        34.0,
                        false,
                        midi_learn,
                    );
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 5.0;
                        ui.add_space(4.0);
                        p5_toggle_pill(
                            ui,
                            "LOG",
                            &params.global.glide_log,
                            setter,
                            ui_state,
                            72.0,
                            23.0,
                            P5_GOLD,
                        );
                        p5_toggle_pill(
                            ui,
                            "DRIFT",
                            &params.global.drift,
                            setter,
                            ui_state,
                            72.0,
                            23.0,
                            P5_RED,
                        );
                    });
                });

                p5_int_stepper(
                    ui,
                    "BEND +",
                    &params.global.bender_up,
                    setter,
                    ui_state,
                    144.0,
                    22.0,
                    P5_CYAN,
                );
                p5_int_stepper(
                    ui,
                    "BEND -",
                    &params.global.bender_down,
                    setter,
                    ui_state,
                    144.0,
                    22.0,
                    P5_MAGENTA,
                );
            });
        });
    });

    // Bottom Action Bar (Exact Phantom Voicer bottom-left bar: Audio/Panic + Updater/CLEANSE + Cyan/Magenta bars)
    child_ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;

            // Left: Audio On / Panic button
            let (ab_rect, ab_resp) = ui.allocate_exact_size(Vec2::new(66.0, 28.0), Sense::click());
            if ab_resp.clicked() {
                ui_state.push_audition_all_off();
                ui_state.set_last_param_label("PANIC: ALL NOTES OFF");
            }
            ab_resp
                .clone()
                .on_hover_text("Audio Engine Active — Click for MIDI Panic (All Notes Off)");
            draw_p5_3d_text(
                ui.painter(),
                Pos2::new(ab_rect.left() + 2.0, ab_rect.center().y),
                egui::Align2::LEFT_CENTER,
                "Audio On",
                12.5,
                if ab_resp.hovered() { P5_GOLD } else { P5_CYAN },
                P5_WHITE,
                FONT_IMPACT,
            );

            // Middle: GitHub Auto-Updater + CLEANSE status pill
            #[derive(Clone, Copy)]
            enum UpAction {
                None,
                Recheck,
                Install,
                Cleanse,
            }

            let cur_ver = env!("CARGO_PKG_VERSION");
            let (up_label, up_fg, up_rim, action, hover_tip): (
                String,
                Color32,
                Color32,
                UpAction,
                Option<String>,
            ) = match up_status {
                UpdateStatus::Idle => (
                    format!("v{cur_ver} CLEANSE"),
                    P5_WHITE,
                    P5_WHITE,
                    UpAction::Recheck,
                    Some("Click to check GitHub for updates & scan stale copies".to_string()),
                ),
                UpdateStatus::Checking => (
                    "Checking...".to_string(),
                    P5_MUTED,
                    P5_WHITE,
                    UpAction::None,
                    Some("Checking GitHub releases...".to_string()),
                ),
                UpdateStatus::UpToDate { stale_copies } if *stale_copies > 0 => (
                    format!("CLEANSE ({stale_copies} OLD)!"),
                    P5_GOLD,
                    P5_RED,
                    UpAction::Cleanse,
                    Some(format!(
                        "Found {stale_copies} old/duplicate copy(s) — click to cleanse!"
                    )),
                ),
                UpdateStatus::UpToDate { .. } => (
                    format!("v{cur_ver} CLEAN"),
                    P5_WHITE,
                    P5_WHITE,
                    UpAction::Recheck,
                    Some("Up to date & clean — click to re-check GitHub".to_string()),
                ),
                UpdateStatus::Available {
                    version,
                    stale_copies,
                    ..
                } => (
                    if *stale_copies > 0 {
                        format!("UPD v{version}+CLN!")
                    } else {
                        format!("UPDATE v{version}!")
                    },
                    P5_GOLD,
                    P5_GOLD,
                    UpAction::Install,
                    Some("Click to download, verify SHA-256, update & cleanse".to_string()),
                ),
                UpdateStatus::Installing(msg) => (
                    format!("Working: {msg}"),
                    P5_CYAN,
                    P5_CYAN,
                    UpAction::None,
                    Some(msg.clone()),
                ),
                UpdateStatus::ReadyToRestart { version, message } => (
                    format!("RESTART v{version}"),
                    P5_CYAN,
                    P5_CYAN,
                    UpAction::None,
                    Some(message.clone()),
                ),
                UpdateStatus::Error(err) => (
                    format!("v{cur_ver} (Retry)"),
                    P5_MUTED,
                    P5_WHITE,
                    UpAction::Recheck,
                    Some(err.clone()),
                ),
            };

            let (up_rect, up_resp) = ui.allocate_exact_size(Vec2::new(138.0, 28.0), Sense::click());
            if let Some(tip) = hover_tip {
                up_resp.clone().on_hover_text(tip);
            }
            if up_resp.clicked() {
                match action {
                    UpAction::Install => {
                        if let UpdateStatus::Available {
                            version, zip_url, ..
                        } = up_status
                        {
                            updater.trigger_update_async(version.clone(), zip_url.clone());
                        }
                    }
                    UpAction::Cleanse => updater.cleanse_async(),
                    UpAction::Recheck => updater.check_async(),
                    UpAction::None => {}
                }
            }
            draw_p5_3d_text(
                ui.painter(),
                Pos2::new(up_rect.left() + 2.0, up_rect.center().y),
                egui::Align2::LEFT_CENTER,
                &up_label,
                10.5,
                up_fg,
                if up_resp.hovered() { P5_RED } else { up_rim },
                FONT_IMPACT,
            );

            // Right: Signature Phantom Voicer twin Cyan & Magenta bars
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (bar_r, _) = ui.allocate_exact_size(Vec2::new(36.0, 14.0), Sense::hover());
                let bp = ui.painter_at(bar_r);
                bp.rect_filled(
                    Rect::from_min_size(Pos2::new(bar_r.min.x, bar_r.center().y - 2.5), Vec2::new(15.0, 5.0)),
                    CornerRadius::ZERO,
                    P5_CYAN,
                );
                bp.rect_filled(
                    Rect::from_min_size(Pos2::new(bar_r.min.x + 18.0, bar_r.center().y - 2.5), Vec2::new(15.0, 5.0)),
                    CornerRadius::ZERO,
                    P5_MAGENTA,
                );
            });
        });
    });
}

fn draw_obsidian_subcard(
    ui: &mut egui::Ui,
    title: &str,
    accent: Color32,
    height: f32,
    body: impl FnOnce(&mut egui::Ui),
) {
    let start = ui.cursor().min;
    let w = ui.available_width();
    ui.painter().rect_filled(
        Rect::from_min_size(start + Vec2::new(3.0, 3.0), Vec2::new(w, height)),
        CornerRadius::ZERO,
        P5_BLACK,
    );
    egui::Frame::new()
        .fill(P5_CARD)
        .stroke(Stroke::new(2.0_f32, P5_WHITE))
        .inner_margin(egui::Margin::symmetric(7, 5))
        .show(ui, |ui| {
            ui.set_min_height((height - 10.0).max(20.0));
            ui.set_max_height((height - 10.0).max(20.0));
            let (tr, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 16.0), Sense::hover());
            ui.painter().text(
                Pos2::new(tr.left() + 2.0, tr.center().y),
                egui::Align2::LEFT_CENTER,
                title,
                font_id(11.0, FONT_IMPACT),
                accent,
            );
            ui.painter().line_segment(
                [Pos2::new(tr.left(), tr.bottom()), Pos2::new(tr.right(), tr.bottom())],
                Stroke::new(1.0_f32, Color32::from_rgb(44, 44, 56)),
            );
            ui.add_space(3.0);
            body(ui);
        });
}
