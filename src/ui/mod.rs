pub mod fx_page;
pub mod left_panel;
pub mod mod_page;
pub mod piano_strip;
pub mod preset_browser;
pub mod synth_page;
pub mod theme;
pub mod widgets;

use crate::params::PhantomSpireParams;
use crate::presets::PresetManager;
use crate::ui::fx_page::{draw_fx_page, FxPageState};
use crate::ui::left_panel::draw_left_command_zone;
use crate::ui::mod_page::{draw_mod_page, ModPageState};
use crate::ui::piano_strip::draw_piano_monitor;
use crate::ui::preset_browser::{draw_preset_browser_page, PresetBrowserState};
use crate::ui::synth_page::{draw_synth_page, SynthPageState};
use crate::ui::theme::{
    apply_p5_style, draw_active_note_trapezoid, draw_p5_3d_text, draw_starburst_bg, font_id,
    midi_note_name, setup_p5_fonts, FONT_HEAVY, FONT_IMPACT, P5_BG, P5_BLACK, P5_CYAN, P5_GOLD,
    P5_MAGENTA, P5_MUTED, P5_PANEL, P5_RED, P5_WHITE,
};
use crate::ui::widgets::{
    draw_stereo_meter, p5_button, p5_enum_dropdown, p5_int_stepper, p5_toggle_pill,
    PopupOverlayState,
};
use crate::updater::{UpdateStatus, UpdaterHandle};
use crate::{SharedSynthState, SharedUiState};
use nih_plug::prelude::*;
use nih_plug_egui::create_egui_editor;
use nih_plug_egui::egui::{
    self, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UiTab {
    Synth,
    FxMaster,
    ModArp,
}

impl Default for UiTab {
    fn default() -> Self {
        match crate::env_ui_tab().as_deref() {
            Some("fx" | "fx_master" | "fx-master" | "master") => Self::FxMaster,
            Some("mod" | "mod_arp" | "mod-arp" | "arp" | "matrix") => Self::ModArp,
            _ => Self::Synth,
        }
    }
}

struct EditorUiState {
    active_tab: UiTab,
    auditioning_note: Option<u8>,
    popups: PopupOverlayState,
    browser: PresetBrowserState,
    synth_page: SynthPageState,
    fx_page: FxPageState,
    mod_page: ModPageState,
    midi_learn_armed: bool,
    midi_learn_status: Option<String>,
    last_updater_msg: Option<String>,
    // Automated screenshot capture support (PHANTOM_SPIRE_SCREENSHOT_OUT)
    screenshot_out: Option<PathBuf>,
    screenshot_frames: u32,
    screenshot_done: Arc<AtomicBool>,
    screenshot_saved: bool,
}

impl Default for EditorUiState {
    fn default() -> Self {
        Self {
            active_tab: UiTab::default(),
            auditioning_note: None,
            popups: PopupOverlayState::default(),
            browser: PresetBrowserState::default(),
            synth_page: SynthPageState::default(),
            fx_page: FxPageState::default(),
            mod_page: ModPageState::default(),
            midi_learn_armed: false,
            midi_learn_status: None,
            last_updater_msg: None,
            screenshot_out: crate::env_screenshot_out(),
            screenshot_frames: 0,
            screenshot_done: Arc::new(AtomicBool::new(false)),
            screenshot_saved: false,
        }
    }
}

pub fn create(
    params: Arc<PhantomSpireParams>,
    ui_state: Arc<SharedUiState>,
    shared_synth: Arc<SharedSynthState>,
    presets: Arc<Mutex<PresetManager>>,
    updater: UpdaterHandle,
) -> Option<Box<dyn Editor>> {
    let egui_state = params.editor_state.clone();
    create_egui_editor(
        egui_state,
        EditorUiState::default(),
        |egui_ctx, _| {
            setup_p5_fonts(egui_ctx);
            apply_p5_style(egui_ctx);
        },
        move |egui_ctx, setter, state| {
            // Optional F1/F2/F3 tab hotkeys when not typing in preset save input
            if !egui_ctx.wants_keyboard_input() {
                egui_ctx.input(|i| {
                    if i.key_pressed(egui::Key::F1) {
                        state.active_tab = UiTab::Synth;
                    } else if i.key_pressed(egui::Key::F2) {
                        state.active_tab = UiTab::FxMaster;
                    } else if i.key_pressed(egui::Key::F3) {
                        state.active_tab = UiTab::ModArp;
                    }
                });
            }

            // Handle automated egui screenshot capture if PHANTOM_SPIRE_SCREENSHOT_OUT is set
            handle_screenshot_capture(egui_ctx, state);

            let in_mask = ui_state.get_input_mask();
            let out_mask = ui_state.get_output_mask();
            let voices = ui_state.active_voice_count.load(Ordering::Relaxed);
            let up_status = updater.status();
            let status_notice = match &up_status {
                UpdateStatus::Available { version, .. } => Some(format!("UPDATE READY: v{version}")),
                UpdateStatus::ReadyToRestart { version, .. } => {
                    Some(format!("UPDATED -> v{version} (RESTART/RELOAD)"))
                }
                UpdateStatus::Error(err) => Some(format!("UPD ERR: {err}")),
                _ => None,
            };
            if status_notice != state.last_updater_msg {
                if let Some(ref msg) = status_notice {
                    ui_state.set_last_param_label(msg);
                }
                state.last_updater_msg = status_notice;
            }

            if in_mask != 0
                || out_mask != 0
                || voices > 0
                || state.auditioning_note.is_some()
                || (state.screenshot_out.is_some() && !state.screenshot_saved)
            {
                egui_ctx.request_repaint();
            } else if matches!(
                up_status,
                UpdateStatus::Checking | UpdateStatus::Installing(_)
            ) {
                egui_ctx.request_repaint_after(std::time::Duration::from_millis(120));
            }

            let mut midi_learn_slot = if state.midi_learn_armed {
                Some(
                    state
                        .midi_learn_status
                        .clone()
                        .unwrap_or_else(|| String::from("MIDI LEARN ARMED")),
                )
            } else {
                None
            };

            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(P5_BG).inner_margin(8.0))
                .show(egui_ctx, |ui| {
                    let full_rect = ui.available_rect_before_wrap();
                    let piano_h = 96.0_f32;
                    let top_h = (full_rect.height() - piano_h - 8.0).max(520.0);
                    let left_w = 270.0_f32;
                    let right_w = (full_rect.width() - left_w - 8.0).max(600.0);

                    ui.allocate_ui_with_layout(
                        Vec2::new(full_rect.width(), top_h),
                        egui::Layout::left_to_right(egui::Align::Min),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;

                            // LEFT CRIMSON COMMAND ZONE (~270px)
                            draw_left_command_zone(
                                ui,
                                left_w,
                                top_h,
                                &params,
                                &ui_state,
                                &shared_synth,
                                &presets,
                                &updater,
                                &up_status,
                                setter,
                                &mut state.popups,
                                &mut state.browser,
                                &mut midi_learn_slot,
                            );

                            // RIGHT STARBURST ZONE
                            draw_starburst_zone(
                                ui,
                                right_w,
                                top_h,
                                &params,
                                &ui_state,
                                &shared_synth,
                                &presets,
                                setter,
                                state,
                                &mut midi_learn_slot,
                            );
                        },
                    );

                    ui.add_space(8.0);

                    // BOTTOM 49-KEY LIVE MONITOR PIANO STRIP
                    draw_piano_monitor(
                        ui,
                        in_mask,
                        out_mask,
                        &ui_state,
                        &mut state.auditioning_note,
                    );
                });

            if let Some(updated_learn) = midi_learn_slot {
                state.midi_learn_status = Some(updated_learn);
            }

            // Floating Custom P5 Dropdown Popups
            state.popups.render_overlay(egui_ctx);

            // Schedule OpenGL framebuffer read_pixels capture on Debug layer AFTER all UI layers render
            schedule_gl_screenshot_if_needed(egui_ctx, state);
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn draw_starburst_zone(
    ui: &mut egui::Ui,
    width: f32,
    height: f32,
    params: &PhantomSpireParams,
    ui_state: &SharedUiState,
    shared_synth: &SharedSynthState,
    presets: &Arc<Mutex<PresetManager>>,
    setter: &ParamSetter,
    state: &mut EditorUiState,
    midi_learn: &mut Option<String>,
) {
    let (zone_rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter_at(zone_rect);
    draw_starburst_bg(&painter, zone_rect);

    let content_rect = zone_rect.shrink(10.0);
    let mut child_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(content_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child_ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);

    // ========================================================================
    // TOP HEADER: Tabs (SYNTH | FX & MASTER | MOD & ARP) + Status + ACTIVE NOTE
    // ========================================================================
    let header_h = 62.0_f32;
    let (hdr_rect, _) =
        child_ui.allocate_exact_size(Vec2::new(content_rect.width(), header_h), Sense::hover());

    let trap_w = 182.0_f32;
    let hdr_left_rect = Rect::from_min_max(
        hdr_rect.min,
        Pos2::new(hdr_rect.right() - trap_w - 8.0, hdr_rect.bottom()),
    );

    let mut hdr_left = child_ui.new_child(
        egui::UiBuilder::new()
            .max_rect(hdr_left_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    hdr_left.spacing_mut().item_spacing = Vec2::new(6.0, 3.0);

    // Header Row 1: 3D P5 Sticker Tabs + Live Param LCD + Stereo L/R Meter
    hdr_left.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;

        for (tab, title, tab_w, active_col) in [
            (UiTab::Synth, "SYNTH", 82.0_f32, P5_CYAN),
            (UiTab::FxMaster, "FX & MASTER", 126.0_f32, P5_GOLD),
            (UiTab::ModArp, "MOD & ARP", 112.0_f32, P5_MAGENTA),
        ] {
            let is_active = !state.browser.open && state.active_tab == tab;
            let (tr, tresp) = ui.allocate_exact_size(Vec2::new(tab_w, 28.0), Sense::click());
            if tresp.clicked() {
                state.active_tab = tab;
                state.browser.open = false;
                state.popups.active = None;
            }
            draw_p5_3d_text(
                ui.painter(),
                tr.center(),
                egui::Align2::CENTER_CENTER,
                title,
                13.5,
                if is_active { active_col } else { P5_WHITE },
                if is_active { active_col } else if tresp.hovered() { P5_RED } else { P5_WHITE },
                FONT_IMPACT,
            );
        }

        // Live Parameter LCD
        let lcd_w = (hdr_left_rect.width() - 82.0 - 126.0 - 112.0 - 110.0 - 30.0).max(140.0);
        let (lcd_r, _) = ui.allocate_exact_size(Vec2::new(lcd_w, 24.0), Sense::hover());
        let lp = ui.painter();
        lp.rect_filled(lcd_r.translate(Vec2::new(2.0, 2.0)), CornerRadius::ZERO, P5_BLACK);
        lp.rect(
            lcd_r,
            CornerRadius::ZERO,
            P5_BG,
            Stroke::new(1.5_f32, P5_CYAN),
            StrokeKind::Inside,
        );
        let raw_lbl = ui_state.get_last_param_label();
        let lcd_text = if !raw_lbl.is_empty() {
            raw_lbl
        } else if let Some(ml) = &state.midi_learn_status {
            ml.clone()
        } else {
            String::from("PHANTOM SPIRE • READY")
        };
        ui.painter_at(lcd_r.shrink(3.0)).text(
            Pos2::new(lcd_r.left() + 6.0, lcd_r.center().y),
            egui::Align2::LEFT_CENTER,
            lcd_text,
            font_id(10.0, FONT_HEAVY),
            P5_CYAN,
        );

        // Stereo L/R Peak Meter
        let (peak_l, peak_r) = ui_state.get_peaks();
        let (meter_r, _) = ui.allocate_exact_size(Vec2::new(108.0, 24.0), Sense::hover());
        draw_stereo_meter(ui.painter(), meter_r, peak_l, peak_r);
    });

    // Header Row 2: Poly Mode, Voice Count, MIDI Learn, Project BPM Sync, SYNC ALL, BPM, ARP
    hdr_left.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;

        let (m_lbl, _) = ui.allocate_exact_size(Vec2::new(34.0, 22.0), Sense::hover());
        ui.painter().text(
            Pos2::new(m_lbl.left(), m_lbl.center().y),
            egui::Align2::LEFT_CENTER,
            "MODE",
            font_id(9.5, FONT_IMPACT),
            P5_GOLD,
        );
        p5_enum_dropdown(
            ui,
            "hdr_poly_mode",
            &params.global.poly_mode,
            setter,
            ui_state,
            &mut state.popups,
            78.0,
            22.0,
            P5_GOLD,
        );

        p5_int_stepper(
            ui,
            "VOICES",
            &params.global.voices,
            setter,
            ui_state,
            84.0,
            22.0,
            P5_CYAN,
        );

        let active_v = ui_state.active_voice_count.load(Ordering::Relaxed);
        let max_v = params.global.voices.value();
        let (vc_r, _) = ui.allocate_exact_size(Vec2::new(66.0, 22.0), Sense::hover());
        ui.painter().rect(
            vc_r,
            CornerRadius::ZERO,
            P5_PANEL,
            Stroke::new(1.3_f32, if active_v > 0 { P5_RED } else { Color32::from_rgb(66, 66, 80) }),
            StrokeKind::Inside,
        );
        ui.painter().text(
            vc_r.center(),
            egui::Align2::CENTER_CENTER,
            format!("ACT {active_v:02}/{max_v:02}"),
            font_id(9.0, FONT_HEAVY),
            if active_v > 0 { P5_GOLD } else { P5_MUTED },
        );

        if p5_button(
            ui,
            if state.midi_learn_armed { "LEARNING" } else { "LEARN" },
            56.0,
            22.0,
            state.midi_learn_armed,
            P5_RED,
            true,
        ) {
            state.midi_learn_armed = !state.midi_learn_armed;
            if state.midi_learn_armed {
                ui_state.set_last_param_label("MIDI LEARN: CLICK ANY KNOB");
            } else {
                state.midi_learn_status = None;
                ui_state.set_last_param_label("MIDI LEARN OFF");
            }
        }

        p5_toggle_pill(
            ui,
            "PROJ SYNC",
            &params.global.host_bpm_sync,
            setter,
            ui_state,
            74.0,
            22.0,
            P5_CYAN,
        );

        let all_synced = params.lfos.iter().all(|l| l.sync.value())
            && params.steppers.iter().all(|s| s.sync.value())
            && params.arp.sync.value()
            && params.fx.delay.sync.value();
        if p5_button(ui, "SYNC ALL", 66.0, 22.0, all_synced, P5_GOLD, true) {
            let sync_params: [&BoolParam; 8] = [
                &params.lfos[0].sync,
                &params.lfos[1].sync,
                &params.lfos[2].sync,
                &params.lfos[3].sync,
                &params.steppers[0].sync,
                &params.steppers[1].sync,
                &params.arp.sync,
                &params.fx.delay.sync,
            ];
            for bp in sync_params {
                setter.begin_set_parameter(bp);
                setter.set_parameter(bp, true);
                setter.end_set_parameter(bp);
            }
            ui_state.set_last_param_label("SYNC ALL: LFO/STP/ARP/DEL");
        }

        // Live Project / Internal BPM readout (interactive scroll/drag edits internal BPM when not locked to DAW)
        let host_sync_on = params.global.host_bpm_sync.value();
        let host_connected = ui_state.is_host_tempo_connected();
        let active_bpm = if host_sync_on && host_connected {
            ui_state.get_active_bpm()
        } else {
            params.arp.internal_bpm.value()
        };
        let bpm_tag = if host_sync_on && host_connected { "DAW" } else { "INT" };
        let (bpm_r, bpm_resp) =
            ui.allocate_exact_size(Vec2::new(88.0, 22.0), Sense::click_and_drag());
        if bpm_resp.drag_started() {
            setter.begin_set_parameter(&params.arp.internal_bpm);
        }
        if bpm_resp.dragged() {
            let delta = -bpm_resp.drag_delta().y * 0.4;
            let next = (params.arp.internal_bpm.value() + delta).clamp(40.0, 240.0);
            setter.set_parameter(&params.arp.internal_bpm, next);
            ui_state.set_last_param_label(&format!("INTERNAL BPM: {next:.1}"));
        }
        if bpm_resp.drag_stopped() {
            setter.end_set_parameter(&params.arp.internal_bpm);
        }
        if bpm_resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.2 {
                let next = (params.arp.internal_bpm.value() + scroll.signum()).clamp(40.0, 240.0);
                setter.begin_set_parameter(&params.arp.internal_bpm);
                setter.set_parameter(&params.arp.internal_bpm, next);
                setter.end_set_parameter(&params.arp.internal_bpm);
                ui_state.set_last_param_label(&format!("INTERNAL BPM: {next:.1}"));
            }
        }
        ui.painter().rect(
            bpm_r,
            CornerRadius::ZERO,
            P5_PANEL,
            Stroke::new(
                1.3_f32,
                if host_sync_on && host_connected {
                    P5_CYAN
                } else {
                    P5_GOLD
                },
            ),
            StrokeKind::Inside,
        );
        ui.painter().text(
            bpm_r.center(),
            egui::Align2::CENTER_CENTER,
            format!("{bpm_tag} {active_bpm:.1}"),
            font_id(9.5, FONT_HEAVY),
            if host_sync_on && host_connected { P5_CYAN } else { P5_WHITE },
        );

        let arp_on = params.arp.enabled.value();
        if p5_button(
            ui,
            if arp_on { "ARP ON" } else { "ARP OFF" },
            60.0,
            22.0,
            arp_on,
            P5_CYAN,
            true,
        ) {
            setter.begin_set_parameter(&params.arp.enabled);
            setter.set_parameter(&params.arp.enabled, !arp_on);
            setter.end_set_parameter(&params.arp.enabled);
        }
    });

    // Right: Slanted White ACTIVE NOTE Trapezoid Badge (Exact Phantom Voicer match!)
    let trap_rect = Rect::from_min_max(
        Pos2::new(hdr_rect.right() - trap_w, hdr_rect.top()),
        Pos2::new(hdr_rect.right(), hdr_rect.bottom() - 2.0),
    );
    let last_note = ui_state.last_note.load(Ordering::Relaxed);
    let active_out = ui_state.get_output_mask();
    let active_in = ui_state.get_input_mask();
    let note_readout = if active_out != 0 || active_in != 0 {
        if last_note >= 0 {
            midi_note_name(last_note as u8)
        } else {
            let mask = if active_out != 0 { active_out } else { active_in };
            midi_note_name(mask.trailing_zeros() as u8)
        }
    } else if last_note >= 0 && ui_state.active_voice_count.load(Ordering::Relaxed) > 0 {
        midi_note_name(last_note as u8)
    } else {
        String::from("READY")
    };
    draw_active_note_trapezoid(child_ui.painter(), trap_rect, "ACTIVE NOTE", &note_readout);

    // ========================================================================
    // TAB CONTENT BODY (or Spire Preset Vault when MENU / presets tab active)
    // ========================================================================
    let body_w = content_rect.width();
    let body_h = (content_rect.height() - header_h - 6.0).max(400.0);

    if state.browser.open {
        draw_preset_browser_page(
            &mut child_ui,
            body_w,
            body_h,
            params,
            ui_state,
            shared_synth,
            presets,
            setter,
            &mut state.popups,
            &mut state.browser,
        );
    } else {
        match state.active_tab {
            UiTab::Synth => draw_synth_page(
                &mut child_ui,
                body_w,
                body_h,
                params,
                ui_state,
                shared_synth,
                setter,
                &mut state.popups,
                &mut state.synth_page,
                midi_learn,
            ),
            UiTab::FxMaster => draw_fx_page(
                &mut child_ui,
                body_w,
                body_h,
                params,
                ui_state,
                setter,
                &mut state.popups,
                &mut state.fx_page,
                midi_learn,
            ),
            UiTab::ModArp => draw_mod_page(
                &mut child_ui,
                body_w,
                body_h,
                params,
                ui_state,
                shared_synth,
                setter,
                &mut state.popups,
                &mut state.mod_page,
            ),
        }
    }
}

// ============================================================================
// Pure-Rust OpenGL Framebuffer PNG Screenshot Capture (egui_glow read_pixels)
// ============================================================================

fn handle_screenshot_capture(egui_ctx: &egui::Context, state: &mut EditorUiState) {
    let Some(_) = state.screenshot_out else {
        return;
    };
    if state.screenshot_saved {
        return;
    }

    state.screenshot_frames = state.screenshot_frames.saturating_add(1);
    if state.screenshot_done.load(Ordering::Acquire) {
        state.screenshot_saved = true;
        if std::env::var("PHANTOM_SPIRE_EXIT_AFTER_SCREENSHOT")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
        {
            egui_ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

fn schedule_gl_screenshot_if_needed(egui_ctx: &egui::Context, state: &EditorUiState) {
    let Some(out_path) = state.screenshot_out.clone() else {
        return;
    };
    if state.screenshot_saved
        || state.screenshot_frames < 6
        || state.screenshot_done.load(Ordering::Acquire)
    {
        return;
    }

    let done_flag = state.screenshot_done.clone();
    let screen_rect = egui_ctx.screen_rect();
    let debug_painter = egui_ctx.layer_painter(egui::LayerId::new(
        egui::Order::Debug,
        egui::Id::new("ps_gl_shot"),
    ));

    let cb = egui_glow::CallbackFn::new(move |info, painter| {
        if done_flag
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        let w = info.screen_size_px[0];
        let h = info.screen_size_px[1];
        if w == 0 || h == 0 {
            return;
        }
        let byte_len = (w as usize) * (h as usize) * 4;
        let mut raw_gl = vec![0u8; byte_len];
        unsafe {
            use egui_glow::glow::HasContext as _;
            let gl = painter.gl();
            gl.finish();
            gl.pixel_store_i32(egui_glow::glow::PACK_ALIGNMENT, 1);
            gl.read_pixels(
                0,
                0,
                w as i32,
                h as i32,
                egui_glow::glow::RGBA,
                egui_glow::glow::UNSIGNED_BYTE,
                egui_glow::glow::PixelPackData::Slice(Some(&mut raw_gl)),
            );
        }

        // OpenGL read_pixels origin is bottom-left; flip rows to top-down RGBA and ensure opaque alpha
        let row_bytes = (w as usize) * 4;
        let mut top_down = vec![0u8; byte_len];
        for y in 0..h as usize {
            let src_y = (h as usize) - 1 - y;
            let src = &raw_gl[src_y * row_bytes..(src_y + 1) * row_bytes];
            let dst = &mut top_down[y * row_bytes..(y + 1) * row_bytes];
            dst.copy_from_slice(src);
            for px in dst.chunks_exact_mut(4) {
                px[3] = 255;
            }
        }

        let _ = write_rgba_png(&out_path, w, h, &top_down);
    });

    debug_painter.add(egui::PaintCallback {
        rect: screen_rect,
        callback: Arc::new(cb),
    });
}

fn write_rgba_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }

    let row_bytes = (width as usize) * 4;
    let mut raw_scanlines = Vec::with_capacity((row_bytes + 1) * height as usize);
    for y in 0..height as usize {
        raw_scanlines.push(0u8); // Filter type 0 (None)
        let start = y * row_bytes;
        raw_scanlines.extend_from_slice(&rgba[start..start + row_bytes]);
    }

    // Wrap raw scanlines in valid zlib stored deflate blocks (level 0, RFC 1950 + RFC 1951)
    let mut zlib = Vec::with_capacity(raw_scanlines.len() + (raw_scanlines.len() / 65535 + 1) * 5 + 6);
    zlib.push(0x78);
    zlib.push(0x01);

    let mut pos = 0usize;
    while pos < raw_scanlines.len() {
        let chunk_len = (raw_scanlines.len() - pos).min(65535);
        let is_final = pos + chunk_len >= raw_scanlines.len();
        zlib.push(if is_final { 0x01 } else { 0x00 });
        let len_u16 = chunk_len as u16;
        let nlen_u16 = !len_u16;
        zlib.extend_from_slice(&len_u16.to_le_bytes());
        zlib.extend_from_slice(&nlen_u16.to_le_bytes());
        zlib.extend_from_slice(&raw_scanlines[pos..pos + chunk_len]);
        pos += chunk_len;
    }
    zlib.extend_from_slice(&adler32(&raw_scanlines).to_be_bytes());

    let mut out = Vec::with_capacity(zlib.len() + 64);
    out.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]); // PNG signature

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(6); // RGBA
    ihdr.push(0); // compression
    ihdr.push(0); // filter
    ihdr.push(0); // interlace
    push_png_chunk(&mut out, b"IHDR", &ihdr);
    push_png_chunk(&mut out, b"IDAT", &zlib);
    push_png_chunk(&mut out, b"IEND", &[]);

    std::fs::write(path, out)
}

fn push_png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = 0xFFFF_FFFFu32;
    for &b in kind.iter().chain(data.iter()) {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    out.extend_from_slice(&(!crc).to_be_bytes());
}

fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}
