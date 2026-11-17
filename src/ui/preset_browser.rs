use crate::params::PhantomSpireParams;
use crate::presets::{PresetBank, PresetManager, SPIRE_CATEGORIES};
use crate::ui::theme::{
    draw_p5_icon_badge, font_id, icon_for_category, FONT_HEAVY, FONT_IMPACT, P5_BG, P5_BLACK,
    P5_CYAN, P5_DIM, P5_GOLD, P5_MAGENTA, P5_MUTED, P5_PANEL, P5_RED, P5_RED_DEEP, P5_SURFACE,
    P5_WHITE,
};
use crate::ui::widgets::{draw_cutout_card, p5_button, p5_string_dropdown, PopupOverlayState};
use crate::{SharedSynthState, SharedUiState};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, Color32, CornerRadius, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2,
};
use std::sync::{Arc, Mutex};

pub struct PresetBrowserState {
    pub open: bool,
    pub search_text: String,
    pub save_name: String,
    pub save_cat_idx: usize,
    pub status_msg: Option<String>,
}

impl Default for PresetBrowserState {
    fn default() -> Self {
        Self {
            open: crate::env_ui_tab().as_deref() == Some("presets"),
            search_text: String::new(),
            save_name: String::from("My Phantom Patch"),
            save_cat_idx: 1, // Bass
            status_msg: None,
        }
    }
}

pub fn category_accent(cat: &str) -> Color32 {
    match cat {
        "Init" => P5_WHITE,
        "Bass" | "Acid" => P5_GOLD,
        "Lead" | "Chord" => P5_RED,
        "Synth" | "Keys" => P5_CYAN,
        "Pad" | "FX" => P5_MAGENTA,
        "Pluck" | "Arp" | "Seq" => P5_CYAN,
        _ => P5_WHITE,
    }
}

/// Full Spire-style Preset Browser rendered inside the Right Starburst Workspace.
#[allow(clippy::too_many_arguments)]
pub fn draw_preset_browser_page(
    ui: &mut egui::Ui,
    width: f32,
    height: f32,
    params: &PhantomSpireParams,
    ui_state: &SharedUiState,
    shared_synth: &SharedSynthState,
    presets: &Arc<Mutex<PresetManager>>,
    setter: &ParamSetter,
    popups: &mut PopupOverlayState,
    browser: &mut PresetBrowserState,
) {
    if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) && popups.active.is_none() {
        browser.open = false;
        return;
    }

    let (
        active_bank,
        active_cat,
        cur_index,
        all_count,
        fac_count,
        usr_count,
        fav_count,
        cat_counts,
        filtered,
        can_undo,
        can_redo,
    ) = {
        let mut mgr = presets.lock().unwrap();
        if mgr.search_query() != browser.search_text {
            mgr.set_search_query(&browser.search_text);
        }
        let bank = mgr.active_bank();
        let cat = mgr.active_category().map(str::to_owned);
        let cur = mgr.current_index();
        let all_c = mgr.total_count_for_bank(PresetBank::All);
        let fac_c = mgr.total_count_for_bank(PresetBank::Factory);
        let usr_c = mgr.total_count_for_bank(PresetBank::User);
        let fav_c = mgr.total_count_for_bank(PresetBank::Favorites);
        let cats = mgr.category_counts(bank, mgr.search_query());
        let filt: Vec<(usize, String, String, String, bool, bool)> = mgr
            .filtered_indices()
            .into_iter()
            .filter_map(|idx| {
                mgr.entries().get(idx).map(|e| {
                    (
                        idx,
                        e.name.clone(),
                        e.category.clone(),
                        e.author.clone(),
                        e.is_factory,
                        e.is_favorite,
                    )
                })
            })
            .collect();
        (
            bank,
            cat,
            cur,
            all_c,
            fac_c,
            usr_c,
            fav_c,
            cats,
            filt,
            mgr.can_undo(),
            mgr.can_redo(),
        )
    };

    let mut next_bank: Option<PresetBank> = None;
    let mut do_rescan = false;
    let mut do_close = false;
    let mut next_cat: Option<Option<String>> = None;
    let mut load_idx: Option<usize> = None;
    let mut toggle_fav_idx: Option<usize> = None;
    let mut delete_idx: Option<usize> = None;

    ui.allocate_ui_with_layout(
        Vec2::new(width, height),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            draw_cutout_card(
                ui,
                "00 // SPIRE PRESET VAULT",
                P5_GOLD,
                P5_WHITE,
                height,
                |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if p5_button(ui, "CLOSE", 56.0, 20.0, false, P5_RED, true) {
                        do_close = true;
                    }
                    if p5_button(ui, "RESCAN", 58.0, 20.0, false, P5_CYAN, true) {
                        do_rescan = true;
                    }
                    ui.add_space(6.0);
                    for (bank, lbl, col) in [
                        (PresetBank::Favorites, format!("FAV ({fav_count})"), P5_GOLD),
                        (PresetBank::User, format!("USER ({usr_count})"), P5_MAGENTA),
                        (PresetBank::Factory, format!("FAC ({fac_count})"), P5_CYAN),
                        (PresetBank::All, format!("ALL ({all_count})"), P5_WHITE),
                    ] {
                        if p5_button(ui, &lbl, 76.0, 20.0, active_bank == bank, col, true) {
                            next_bank = Some(bank);
                        }
                    }
                },
                |ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(6.0, 5.0);
                    let avail_w = (width - 20.0).max(600.0);
                    ui.set_max_width(avail_w);

                    // ============================================================
                    // TOP SEARCH + UNDO/REDO/INIT BAR
                    // ============================================================
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 5.0;
                        let (sr_lbl, _) = ui.allocate_exact_size(Vec2::new(56.0, 24.0), Sense::hover());
                        ui.painter().text(
                            Pos2::new(sr_lbl.left() + 2.0, sr_lbl.center().y),
                            egui::Align2::LEFT_CENTER,
                            "SEARCH:",
                            font_id(10.5, FONT_IMPACT),
                            P5_CYAN,
                        );

                        egui::Frame::new()
                            .fill(P5_BG)
                            .stroke(Stroke::new(1.5_f32, P5_CYAN))
                            .inner_margin(egui::Margin::symmetric(6, 3))
                            .show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut browser.search_text)
                                        .desired_width(220.0)
                                        .frame(false)
                                        .text_color(P5_WHITE)
                                        .hint_text("Filter presets by name, author, category...")
                                        .font(font_id(11.0, FONT_HEAVY)),
                                );
                            });

                        if p5_button(ui, "CLR", 38.0, 24.0, false, P5_WHITE, !browser.search_text.is_empty()) {
                            browser.search_text.clear();
                        }

                        ui.add_space(6.0);
                        if p5_button(ui, "INIT", 48.0, 24.0, false, P5_RED, true) {
                            let mut mgr = presets.lock().unwrap();
                            let _ = mgr.apply_to_gui(0, params, setter, shared_synth);
                            ui_state.set_last_param_label("PRESET: Init");
                        }
                        if p5_button(ui, "UNDO", 50.0, 24.0, false, P5_WHITE, can_undo) {
                            let mut mgr = presets.lock().unwrap();
                            if mgr.undo_gui(params, setter, shared_synth) {
                                ui_state.set_last_param_label("UNDO APPLIED");
                            }
                        }
                        if p5_button(ui, "REDO", 50.0, 24.0, false, P5_WHITE, can_redo) {
                            let mut mgr = presets.lock().unwrap();
                            if mgr.redo_gui(params, setter, shared_synth) {
                                ui_state.set_last_param_label("REDO APPLIED");
                            }
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let (cnt_r, _) = ui.allocate_exact_size(Vec2::new(150.0, 22.0), Sense::hover());
                            ui.painter().rect_filled(cnt_r, CornerRadius::ZERO, P5_PANEL);
                            ui.painter().rect_stroke(
                                cnt_r,
                                CornerRadius::ZERO,
                                Stroke::new(1.2_f32, P5_DIM),
                                StrokeKind::Inside,
                            );
                            ui.painter().text(
                                cnt_r.center(),
                                egui::Align2::CENTER_CENTER,
                                format!("SHOWING {} / {}", filtered.len(), all_count),
                                font_id(9.5, FONT_HEAVY),
                                P5_GOLD,
                            );
                        });
                    });

                    // ============================================================
                    // MAIN SPLIT: LEFT CATEGORY SIDEBAR + RIGHT PRESET LIST
                    // ============================================================
                    let body_h = (height - 134.0).max(360.0);
                    let side_w = 152.0_f32;
                    let list_w = (avail_w - side_w - 8.0).max(400.0);

                    ui.allocate_ui_with_layout(
                        Vec2::new(avail_w, body_h),
                        egui::Layout::left_to_right(egui::Align::Min),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;

                            // --- LEFT CATEGORY SIDEBAR ---
                            ui.allocate_ui_with_layout(
                                Vec2::new(side_w, body_h),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    egui::Frame::new()
                                        .fill(P5_PANEL)
                                        .stroke(Stroke::new(1.5_f32, P5_WHITE))
                                        .inner_margin(5.0)
                                        .show(ui, |ui| {
                                            ui.set_width(side_w - 10.0);
                                            ui.set_height(body_h - 10.0);
                                            ui.spacing_mut().item_spacing.y = 2.0;

                                            let inner_w = side_w - 14.0;
                                            let (ch, _) = ui.allocate_exact_size(
                                                Vec2::new(inner_w, 16.0),
                                                Sense::hover(),
                                            );
                                            ui.painter().text(
                                                Pos2::new(ch.left() + 3.0, ch.center().y),
                                                egui::Align2::LEFT_CENTER,
                                                "CATEGORIES",
                                                font_id(10.0, FONT_IMPACT),
                                                P5_GOLD,
                                            );

                                            let total_cat_sum: usize =
                                                cat_counts.iter().map(|(_, c)| *c).sum();
                                            let is_all_cat = active_cat.is_none();
                                            if draw_cat_row(
                                                ui,
                                                "ALL",
                                                total_cat_sum,
                                                is_all_cat,
                                                P5_WHITE,
                                                inner_w,
                                            ) {
                                                next_cat = Some(None);
                                            }

                                            for (cat_name, count) in &cat_counts {
                                                let is_sel = active_cat
                                                    .as_deref()
                                                    .map(|c| c.eq_ignore_ascii_case(cat_name))
                                                    .unwrap_or(false);
                                                if draw_cat_row(
                                                    ui,
                                                    &cat_name.to_uppercase(),
                                                    *count,
                                                    is_sel,
                                                    category_accent(cat_name),
                                                    inner_w,
                                                ) {
                                                    next_cat = Some(Some((*cat_name).to_owned()));
                                                }
                                            }
                                        });
                                },
                            );

                            // --- RIGHT SCROLLABLE PRESET LIST ---
                            ui.allocate_ui_with_layout(
                                Vec2::new(list_w, body_h),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    egui::Frame::new()
                                        .fill(P5_BG)
                                        .stroke(Stroke::new(1.5_f32, P5_WHITE))
                                        .inner_margin(6.0)
                                        .show(ui, |ui| {
                                            let lw = list_w - 12.0;
                                            ui.set_width(lw);
                                            ui.set_height(body_h - 12.0);

                                            // Column header strip
                                            let (col_r, _) = ui.allocate_exact_size(
                                                Vec2::new(lw, 18.0),
                                                Sense::hover(),
                                            );
                                            let cp = ui.painter();
                                            cp.rect_filled(col_r, CornerRadius::ZERO, P5_PANEL);
                                            cp.text(Pos2::new(col_r.left() + 6.0, col_r.center().y), egui::Align2::LEFT_CENTER, "FAV", font_id(8.5, FONT_HEAVY), P5_GOLD);
                                            cp.text(Pos2::new(col_r.left() + 44.0, col_r.center().y), egui::Align2::LEFT_CENTER, "PRESET NAME", font_id(8.5, FONT_HEAVY), P5_WHITE);
                                            cp.text(Pos2::new(col_r.right() - 266.0, col_r.center().y), egui::Align2::LEFT_CENTER, "CATEGORY", font_id(8.5, FONT_HEAVY), P5_CYAN);
                                            cp.text(Pos2::new(col_r.right() - 172.0, col_r.center().y), egui::Align2::LEFT_CENTER, "AUTHOR", font_id(8.5, FONT_HEAVY), P5_MUTED);
                                            cp.text(Pos2::new(col_r.right() - 48.0, col_r.center().y), egui::Align2::RIGHT_CENTER, "BANK", font_id(8.5, FONT_HEAVY), P5_MUTED);

                                            ui.add_space(3.0);

                                            egui::ScrollArea::vertical()
                                                .id_salt("spire_preset_scroll")
                                                .max_height(body_h - 40.0)
                                                .auto_shrink([false, false])
                                                .show(ui, |ui| {
                                                    ui.spacing_mut().item_spacing.y = 3.0;
                                                    if filtered.is_empty() {
                                                        let (er, _) = ui.allocate_exact_size(
                                                            Vec2::new(lw - 12.0, 64.0),
                                                            Sense::hover(),
                                                        );
                                                        ui.painter().text(
                                                            er.center(),
                                                            egui::Align2::CENTER_CENTER,
                                                            "NO PRESETS MATCH CURRENT FILTER / SEARCH",
                                                            font_id(12.0, FONT_IMPACT),
                                                            P5_MUTED,
                                                        );
                                                    }
                                                    for (entry_idx, name, cat, author, is_fac, is_fav) in &filtered {
                                                        let is_cur = *entry_idx == cur_index;
                                                        let row_w = lw - 14.0;
                                                        ui.horizontal(|ui| {
                                                            ui.spacing_mut().item_spacing.x = 4.0;

                                                            // Favorite star button
                                                            let (fav_r, fav_resp) = ui.allocate_exact_size(
                                                                Vec2::new(26.0, 26.0),
                                                                Sense::click(),
                                                            );
                                                            ui.painter().rect(
                                                                fav_r,
                                                                CornerRadius::ZERO,
                                                                if *is_fav {
                                                                    Color32::from_rgb(48, 36, 8)
                                                                } else {
                                                                    P5_SURFACE
                                                                },
                                                                Stroke::new(
                                                                    1.2_f32,
                                                                    if *is_fav { P5_GOLD } else { P5_DIM },
                                                                ),
                                                                StrokeKind::Inside,
                                                            );
                                                            ui.painter().text(
                                                                fav_r.center(),
                                                                egui::Align2::CENTER_CENTER,
                                                                if *is_fav { "★" } else { "☆" },
                                                                font_id(13.0, FONT_HEAVY),
                                                                if *is_fav { P5_GOLD } else { P5_MUTED },
                                                            );
                                                            if fav_resp.clicked() {
                                                                toggle_fav_idx = Some(*entry_idx);
                                                            }

                                                            // Main preset row
                                                            let main_w = if *is_fac {
                                                                row_w - 30.0
                                                            } else {
                                                                row_w - 74.0
                                                            };
                                                            let (rr, rresp) = ui.allocate_exact_size(
                                                                Vec2::new(main_w, 26.0),
                                                                Sense::click(),
                                                            );
                                                            let rp = ui.painter();
                                                            let bg = if is_cur {
                                                                P5_WHITE
                                                            } else if rresp.hovered() {
                                                                P5_RED_DEEP
                                                            } else {
                                                                P5_PANEL
                                                            };
                                                            rp.rect(
                                                                rr,
                                                                CornerRadius::ZERO,
                                                                bg,
                                                                Stroke::new(
                                                                    1.4_f32,
                                                                    if is_cur {
                                                                        P5_RED
                                                                    } else if rresp.hovered() {
                                                                        P5_WHITE
                                                                    } else {
                                                                        Color32::from_rgb(48, 48, 60)
                                                                    },
                                                                ),
                                                                StrokeKind::Inside,
                                                            );

                                                            draw_p5_icon_badge(
                                                                rp,
                                                                Pos2::new(rr.left() + 14.0, rr.center().y),
                                                                7.0,
                                                                icon_for_category(cat),
                                                            );

                                                            let name_clip = Rect::from_min_max(
                                                                Pos2::new(rr.left() + 26.0, rr.top()),
                                                                Pos2::new(rr.right() - 244.0, rr.bottom()),
                                                            );
                                                            ui.painter_at(name_clip).text(
                                                                Pos2::new(rr.left() + 28.0, rr.center().y),
                                                                egui::Align2::LEFT_CENTER,
                                                                name,
                                                                font_id(11.0, FONT_IMPACT),
                                                                if is_cur { P5_BLACK } else { P5_WHITE },
                                                            );

                                                            // Category pill
                                                            let cat_col = category_accent(cat);
                                                            let cat_r = Rect::from_min_size(
                                                                Pos2::new(rr.right() - 236.0, rr.top() + 4.0),
                                                                Vec2::new(62.0, 18.0),
                                                            );
                                                            rp.rect_filled(cat_r, CornerRadius::ZERO, P5_BLACK);
                                                            rp.rect_stroke(
                                                                cat_r,
                                                                CornerRadius::ZERO,
                                                                Stroke::new(1.1_f32, cat_col),
                                                                StrokeKind::Inside,
                                                            );
                                                            rp.text(
                                                                cat_r.center(),
                                                                egui::Align2::CENTER_CENTER,
                                                                cat.to_uppercase(),
                                                                font_id(8.5, FONT_IMPACT),
                                                                cat_col,
                                                            );

                                                            // Author
                                                            rp.text(
                                                                Pos2::new(rr.right() - 164.0, rr.center().y),
                                                                egui::Align2::LEFT_CENTER,
                                                                author,
                                                                font_id(9.0, FONT_HEAVY),
                                                                if is_cur { P5_RED_DEEP } else { P5_MUTED },
                                                            );

                                                            // Bank tag
                                                            rp.text(
                                                                Pos2::new(rr.right() - 10.0, rr.center().y),
                                                                egui::Align2::RIGHT_CENTER,
                                                                if *is_fac { "FAC" } else { "USR" },
                                                                font_id(9.0, FONT_IMPACT),
                                                                if is_cur {
                                                                    P5_RED
                                                                } else if *is_fac {
                                                                    P5_CYAN
                                                                } else {
                                                                    P5_GOLD
                                                                },
                                                            );

                                                            if rresp.clicked() {
                                                                load_idx = Some(*entry_idx);
                                                            }

                                                            if !is_fac
                                                                && p5_button(
                                                                    ui,
                                                                    "DEL",
                                                                    40.0,
                                                                    26.0,
                                                                    false,
                                                                    P5_RED,
                                                                    true,
                                                                )
                                                            {
                                                                delete_idx = Some(*entry_idx);
                                                            }
                                                        });
                                                    }
                                                });
                                        });
                                },
                            );
                        },
                    );

                    // ============================================================
                    // BOTTOM SAVE AS BAR (Name + Category Picker + SAVE)
                    // ============================================================
                    egui::Frame::new()
                        .fill(P5_PANEL)
                        .stroke(Stroke::new(1.5_f32, P5_RED))
                        .inner_margin(egui::Margin::symmetric(8, 5))
                        .show(ui, |ui| {
                            ui.set_width(avail_w - 16.0);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                let (sl, _) =
                                    ui.allocate_exact_size(Vec2::new(64.0, 22.0), Sense::hover());
                                ui.painter().text(
                                    Pos2::new(sl.left() + 2.0, sl.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    "SAVE AS:",
                                    font_id(10.5, FONT_IMPACT),
                                    P5_GOLD,
                                );

                                egui::Frame::new()
                                    .fill(P5_BG)
                                    .stroke(Stroke::new(1.3_f32, P5_WHITE))
                                    .inner_margin(egui::Margin::symmetric(6, 2))
                                    .show(ui, |ui| {
                                        ui.add(
                                            egui::TextEdit::singleline(&mut browser.save_name)
                                                .desired_width(210.0)
                                                .frame(false)
                                                .text_color(P5_WHITE)
                                                .font(font_id(11.0, FONT_HEAVY)),
                                        );
                                    });

                                if let Some(sel_cat) = p5_string_dropdown(
                                    ui,
                                    "preset_save_category",
                                    SPIRE_CATEGORIES,
                                    browser.save_cat_idx,
                                    popups,
                                    108.0,
                                    22.0,
                                    P5_GOLD,
                                ) {
                                    browser.save_cat_idx = sel_cat;
                                }

                                if p5_button(ui, "SAVE USER PRESET", 136.0, 22.0, false, P5_GOLD, true) {
                                    let cat = SPIRE_CATEGORIES
                                        .get(browser.save_cat_idx)
                                        .copied()
                                        .unwrap_or("Synth");
                                    let data = PresetManager::capture_from_params(params);
                                    let mut mgr = presets.lock().unwrap();
                                    match mgr.save_user_preset_with_category(
                                        &browser.save_name,
                                        cat,
                                        data,
                                    ) {
                                        Ok(_) => {
                                            browser.status_msg = Some(format!(
                                                "SAVED '{}' [{}]",
                                                browser.save_name.trim(),
                                                cat.to_uppercase()
                                            ));
                                            ui_state.set_last_param_label("USER PRESET SAVED");
                                        }
                                        Err(e) => browser.status_msg = Some(e),
                                    }
                                }

                                if let Some(msg) = &browser.status_msg {
                                    let (mr, _) = ui.allocate_exact_size(
                                        Vec2::new(ui.available_width(), 22.0),
                                        Sense::hover(),
                                    );
                                    ui.painter().text(
                                        Pos2::new(mr.left() + 4.0, mr.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        msg,
                                        font_id(9.5, FONT_HEAVY),
                                        P5_CYAN,
                                    );
                                }
                            });
                        });
                },
            );
        },
    );

    // Apply deferred PresetManager actions
    if next_bank.is_some()
        || next_cat.is_some()
        || do_rescan
        || load_idx.is_some()
        || toggle_fav_idx.is_some()
        || delete_idx.is_some()
    {
        let mut mgr = presets.lock().unwrap();
        if let Some(b) = next_bank {
            mgr.set_active_bank(b);
        }
        if let Some(c) = next_cat {
            mgr.set_active_category(c.as_deref());
        }
        if do_rescan {
            mgr.rescan();
            browser.status_msg = Some(String::from("PRESETS RESCANNED"));
        }
        if let Some(idx) = toggle_fav_idx {
            let now_fav = mgr.toggle_favorite(idx);
            browser.status_msg = Some(if now_fav {
                String::from("ADDED TO ★ FAVORITES")
            } else {
                String::from("REMOVED FROM FAVORITES")
            });
        }
        if let Some(idx) = load_idx {
            if mgr.apply_to_gui(idx, params, setter, shared_synth).is_ok() {
                let name = mgr.current_preset().name.clone();
                ui_state.set_last_param_label(&format!("PRESET: {name}"));
                browser.status_msg = Some(format!("LOADED '{name}'"));
            }
        }
        if let Some(idx) = delete_idx {
            match mgr.delete_user_preset(idx) {
                Ok(()) => browser.status_msg = Some(String::from("DELETED USER PRESET")),
                Err(e) => browser.status_msg = Some(e),
            }
        }
    }

    if do_close {
        browser.open = false;
    }
}

fn draw_cat_row(
    ui: &mut egui::Ui,
    label: &str,
    count: usize,
    selected: bool,
    accent: Color32,
    width: f32,
) -> bool {
    let (r, resp) = ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::click());
    let p = ui.painter();
    let bg = if selected {
        P5_WHITE
    } else if resp.hovered() {
        P5_RED_DEEP
    } else {
        P5_SURFACE
    };
    p.rect(
        r,
        CornerRadius::ZERO,
        bg,
        Stroke::new(
            1.2_f32,
            if selected {
                P5_BLACK
            } else if resp.hovered() {
                P5_WHITE
            } else {
                Color32::from_rgb(48, 48, 60)
            },
        ),
        StrokeKind::Inside,
    );

    if selected {
        let cy = r.center().y;
        let wedge = vec![
            Pos2::new(r.left() + 4.0, cy - 5.0),
            Pos2::new(r.left() + 12.0, cy),
            Pos2::new(r.left() + 4.0, cy + 5.0),
        ];
        p.add(Shape::convex_polygon(wedge, P5_RED, Stroke::NONE));
    } else {
        p.rect_filled(
            Rect::from_min_size(Pos2::new(r.left() + 3.0, r.top() + 4.0), Vec2::new(4.0, 18.0)),
            CornerRadius::ZERO,
            accent,
        );
    }

    p.text(
        Pos2::new(r.left() + 16.0, r.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        font_id(10.0, FONT_IMPACT),
        if selected { P5_BLACK } else { P5_WHITE },
    );

    p.text(
        Pos2::new(r.right() - 6.0, r.center().y),
        egui::Align2::RIGHT_CENTER,
        format!("{count}"),
        font_id(9.0, FONT_HEAVY),
        if selected { P5_RED } else { P5_GOLD },
    );

    resp.clicked()
}
