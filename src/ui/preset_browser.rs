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

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum BrowserViewMode {
    #[default]
    Folder,
    Author,
    Tag,
    Favorite,
}

pub struct PresetBrowserState {
    pub open: bool,
    pub view_mode: BrowserViewMode,
    pub factory_expanded: bool,
    pub search_text: String,
    pub save_name: String,
    pub save_cat_idx: usize,
    pub status_msg: Option<String>,
}

impl Default for PresetBrowserState {
    fn default() -> Self {
        Self {
            open: crate::env_ui_tab().as_deref() == Some("presets"),
            view_mode: BrowserViewMode::Folder,
            factory_expanded: true,
            search_text: String::new(),
            save_name: String::from("My Phantom Patch"),
            save_cat_idx: 1, // Basses
            status_msg: None,
        }
    }
}

pub fn category_accent(cat: &str) -> Color32 {
    match cat {
        "Init" => P5_WHITE,
        "Basses" | "Bass" | "Acid" => P5_GOLD,
        "Drums" => P5_RED,
        "FX" => P5_MAGENTA,
        "Keys" => P5_CYAN,
        "Leads" | "Lead" => P5_RED,
        "Pads" | "Pad" => P5_MAGENTA,
        "Plucks" | "Pluck" => P5_CYAN,
        "Seq & Arp" | "Arp" | "Seq" => P5_GOLD,
        "Synth" | "Chord" => P5_CYAN,
        _ => P5_WHITE,
    }
}

pub fn category_prefix(cat: &str) -> &'static str {
    match cat {
        "Init" => "IN",
        "Basses" | "Bass" => "BS",
        "Drums" => "DR",
        "FX" => "FX",
        "Keys" => "KY",
        "Leads" | "Lead" => "LD",
        "Pads" | "Pad" => "PD",
        "Plucks" | "Pluck" => "PL",
        "Seq & Arp" | "Arp" | "Seq" => "SQ",
        "Synth" | "Chord" => "SY",
        _ => "PR",
    }
}

/// Full Spire-style 3-column Preset Browser + Factory Sounds Tree in Persona 5 style.
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
        active_author,
        active_tag,
        cur_index,
        cur_name,
        cur_cat,
        cur_author,
        cur_is_fac,
        cur_is_fav,
        cur_tags,
        all_count,
        fac_count,
        usr_count,
        fav_count,
        cat_counts,
        author_counts,
        tag_counts,
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
        let auth = mgr.active_author().map(str::to_owned);
        let tag = mgr.active_tag().map(str::to_owned);
        let cur_idx = mgr.current_index();
        let cur_entry = mgr.current_preset().clone();

        let all_c = mgr.total_count_for_bank(PresetBank::All);
        let fac_c = mgr.total_count_for_bank(PresetBank::Factory);
        let usr_c = mgr.total_count_for_bank(PresetBank::User);
        let fav_c = mgr.total_count_for_bank(PresetBank::Favorites);
        let cats = mgr.category_counts(PresetBank::Factory, "");
        let auths = mgr.authors(bank);
        let tags = mgr.tags(bank);
        let filt: Vec<(usize, String, String, bool, bool)> = mgr
            .filtered_indices()
            .into_iter()
            .filter_map(|idx| {
                mgr.entries().get(idx).map(|e| {
                    (
                        idx,
                        e.name.clone(),
                        e.category.clone(),
                        e.is_factory,
                        e.is_favorite,
                    )
                })
            })
            .collect();
        (
            bank,
            cat,
            auth,
            tag,
            cur_idx,
            cur_entry.name,
            cur_entry.category,
            cur_entry.author,
            cur_entry.is_factory,
            cur_entry.is_favorite,
            cur_entry.data.tags,
            all_c,
            fac_c,
            usr_c,
            fav_c,
            cats,
            auths,
            tags,
            filt,
            mgr.can_undo(),
            mgr.can_redo(),
        )
    };

    let mut next_view_mode: Option<BrowserViewMode> = None;
    let mut next_bank: Option<PresetBank> = None;
    let mut next_cat: Option<Option<String>> = None;
    let mut next_author: Option<Option<String>> = None;
    let mut next_tag: Option<Option<String>> = None;
    let mut do_rescan = false;
    let mut do_close = false;
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
                    if p5_button(ui, "CLOSE", 52.0, 20.0, false, P5_RED, true) {
                        do_close = true;
                    }
                    if p5_button(ui, "RESCAN", 56.0, 20.0, false, P5_CYAN, true) {
                        do_rescan = true;
                    }
                    if p5_button(ui, "REDO", 44.0, 20.0, false, P5_WHITE, can_redo) {
                        let mut mgr = presets.lock().unwrap();
                        if mgr.redo_gui(params, setter, shared_synth) {
                            ui_state.set_last_param_label("REDO APPLIED");
                        }
                    }
                    if p5_button(ui, "UNDO", 44.0, 20.0, false, P5_WHITE, can_undo) {
                        let mut mgr = presets.lock().unwrap();
                        if mgr.undo_gui(params, setter, shared_synth) {
                            ui_state.set_last_param_label("UNDO APPLIED");
                        }
                    }
                    if p5_button(ui, "INIT", 42.0, 20.0, false, P5_RED, true) {
                        let mut mgr = presets.lock().unwrap();
                        let _ = mgr.apply_to_gui(0, params, setter, shared_synth);
                        ui_state.set_last_param_label("PRESET: Init");
                    }

                    ui.add_space(6.0);
                    for (mode, lbl, col) in [
                        (BrowserViewMode::Favorite, format!("★ FAV ({fav_count})"), P5_GOLD),
                        (BrowserViewMode::Tag, "TAG".to_string(), P5_MAGENTA),
                        (BrowserViewMode::Author, "AUTHOR".to_string(), P5_CYAN),
                        (BrowserViewMode::Folder, "FOLDER".to_string(), P5_WHITE),
                    ] {
                        if p5_button(
                            ui,
                            &lbl,
                            if mode == BrowserViewMode::Favorite { 76.0 } else { 58.0 },
                            20.0,
                            browser.view_mode == mode,
                            col,
                            true,
                        ) {
                            next_view_mode = Some(mode);
                        }
                    }
                },
                |ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(6.0, 5.0);
                    let avail_w = (width - 20.0).max(600.0);
                    ui.set_max_width(avail_w);

                    // ============================================================
                    // SEARCH + MARK FAVORITE + COUNT BAR
                    // ============================================================
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 5.0;
                        let (sr_lbl, _) = ui.allocate_exact_size(Vec2::new(56.0, 23.0), Sense::hover());
                        ui.painter().text(
                            Pos2::new(sr_lbl.left() + 2.0, sr_lbl.center().y),
                            egui::Align2::LEFT_CENTER,
                            "SEARCH:",
                            font_id(10.0, FONT_IMPACT),
                            P5_CYAN,
                        );

                        egui::Frame::new()
                            .fill(P5_BG)
                            .stroke(Stroke::new(1.4_f32, P5_CYAN))
                            .inner_margin(egui::Margin::symmetric(6, 2))
                            .show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut browser.search_text)
                                        .desired_width(240.0)
                                        .frame(false)
                                        .text_color(P5_WHITE)
                                        .hint_text("Search presets, author, tags...")
                                        .font(font_id(11.0, FONT_HEAVY)),
                                );
                            });

                        if p5_button(ui, "CLR", 36.0, 23.0, false, P5_WHITE, !browser.search_text.is_empty()) {
                            browser.search_text.clear();
                        }

                        if p5_button(
                            ui,
                            if cur_is_fav { "★ FAVORITED" } else { "☆ MARK FAV" },
                            92.0,
                            23.0,
                            cur_is_fav,
                            P5_GOLD,
                            true,
                        ) {
                            toggle_fav_idx = Some(cur_index);
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
                    // MAIN SPLIT: LEFT TREE SIDEBAR + RIGHT 3-COLUMN PRESET GRID
                    // ============================================================
                    let body_h = (height - 150.0).max(340.0);
                    let side_w = 172.0_f32;
                    let grid_w = (avail_w - side_w - 8.0).max(400.0);

                    ui.allocate_ui_with_layout(
                        Vec2::new(avail_w, body_h),
                        egui::Layout::left_to_right(egui::Align::Min),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;

                            // --- LEFT TREE SIDEBAR ---
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
                                            let inner_w = side_w - 14.0;

                                            egui::ScrollArea::vertical()
                                                .id_salt("spire_tree_scroll")
                                                .max_height(body_h - 12.0)
                                                .auto_shrink([false, false])
                                                .show(ui, |ui| {
                                                    ui.spacing_mut().item_spacing.y = 2.0;

                                                    match browser.view_mode {
                                                        BrowserViewMode::Folder
                                                        | BrowserViewMode::Favorite => {
                                                            // All Sounds root
                                                            let is_all_sel = active_bank == PresetBank::All
                                                                && active_cat.is_none();
                                                            if draw_tree_row(
                                                                ui,
                                                                "All Sounds",
                                                                all_count,
                                                                is_all_sel,
                                                                P5_WHITE,
                                                                false,
                                                                inner_w,
                                                            ) {
                                                                next_bank = Some(PresetBank::All);
                                                                next_cat = Some(None);
                                                            }

                                                            // Collapsible Factory Sounds (291) header
                                                            let fac_root_sel = active_bank == PresetBank::Factory
                                                                && active_cat.is_none();
                                                            let arrow = if browser.factory_expanded { "▾" } else { "▸" };
                                                            let (fr, fresp) = ui.allocate_exact_size(
                                                                Vec2::new(inner_w, 23.0),
                                                                Sense::click(),
                                                            );
                                                            let arrow_rect = Rect::from_min_size(fr.min, Vec2::new(18.0, 23.0));
                                                            if fresp.clicked() {
                                                                if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                                                                    if arrow_rect.contains(pos) {
                                                                        browser.factory_expanded = !browser.factory_expanded;
                                                                    } else {
                                                                        next_bank = Some(PresetBank::Factory);
                                                                        next_cat = Some(None);
                                                                    }
                                                                }
                                                            }
                                                            draw_folder_header_row(
                                                                ui.painter(),
                                                                fr,
                                                                fresp.hovered(),
                                                                arrow,
                                                                "Factory Sounds",
                                                                fac_count,
                                                                fac_root_sel,
                                                                P5_CYAN,
                                                            );

                                                            if browser.factory_expanded {
                                                                for (cat_name, count) in &cat_counts {
                                                                    let is_sel = active_bank != PresetBank::User
                                                                        && active_cat
                                                                            .as_deref()
                                                                            .map(|c| c.eq_ignore_ascii_case(cat_name))
                                                                            .unwrap_or(false);
                                                                    if draw_tree_row(
                                                                        ui,
                                                                        cat_name,
                                                                        *count,
                                                                        is_sel,
                                                                        category_accent(cat_name),
                                                                        true,
                                                                        inner_w,
                                                                    ) {
                                                                        if active_bank == PresetBank::User {
                                                                            next_bank = Some(PresetBank::Factory);
                                                                        }
                                                                        next_cat = Some(Some((*cat_name).to_owned()));
                                                                    }
                                                                }
                                                            }

                                                            // User Library (N)
                                                            let usr_sel = active_bank == PresetBank::User;
                                                            if draw_tree_row(
                                                                ui,
                                                                "User Library",
                                                                usr_count,
                                                                usr_sel,
                                                                P5_MAGENTA,
                                                                false,
                                                                inner_w,
                                                            ) {
                                                                next_bank = Some(PresetBank::User);
                                                                next_cat = Some(None);
                                                            }
                                                        }
                                                        BrowserViewMode::Author => {
                                                            if draw_tree_row(
                                                                ui,
                                                                "All Authors",
                                                                all_count,
                                                                active_author.is_none(),
                                                                P5_CYAN,
                                                                false,
                                                                inner_w,
                                                            ) {
                                                                next_author = Some(None);
                                                            }
                                                            for (auth_name, count) in &author_counts {
                                                                let is_sel = active_author
                                                                    .as_deref()
                                                                    .map(|a| a.eq_ignore_ascii_case(auth_name))
                                                                    .unwrap_or(false);
                                                                if draw_tree_row(
                                                                    ui,
                                                                    auth_name,
                                                                    *count,
                                                                    is_sel,
                                                                    P5_GOLD,
                                                                    true,
                                                                    inner_w,
                                                                ) {
                                                                    next_author = Some(Some(auth_name.clone()));
                                                                }
                                                            }
                                                        }
                                                        BrowserViewMode::Tag => {
                                                            if draw_tree_row(
                                                                ui,
                                                                "All Tags",
                                                                all_count,
                                                                active_tag.is_none(),
                                                                P5_MAGENTA,
                                                                false,
                                                                inner_w,
                                                            ) {
                                                                next_tag = Some(None);
                                                            }
                                                            for (tag_name, count) in &tag_counts {
                                                                let is_sel = active_tag
                                                                    .as_deref()
                                                                    .map(|t| t.eq_ignore_ascii_case(tag_name))
                                                                    .unwrap_or(false);
                                                                if draw_tree_row(
                                                                    ui,
                                                                    tag_name,
                                                                    *count,
                                                                    is_sel,
                                                                    P5_CYAN,
                                                                    true,
                                                                    inner_w,
                                                                ) {
                                                                    next_tag = Some(Some(tag_name.clone()));
                                                                }
                                                            }
                                                        }
                                                    }
                                                });
                                        });
                                },
                            );

                            // --- RIGHT 3-COLUMN SCROLLABLE PRESET GRID ---
                            ui.allocate_ui_with_layout(
                                Vec2::new(grid_w, body_h),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    egui::Frame::new()
                                        .fill(P5_BG)
                                        .stroke(Stroke::new(1.5_f32, P5_WHITE))
                                        .inner_margin(6.0)
                                        .show(ui, |ui| {
                                            let gw = grid_w - 12.0;
                                            ui.set_width(gw);
                                            ui.set_height(body_h - 12.0);

                                            let scroll_inner_w = gw - 14.0;
                                            let cols = 3usize;
                                            let col_gap = 4.0_f32;
                                            let cell_w = ((scroll_inner_w - col_gap * (cols as f32 - 1.0)) / cols as f32).floor();
                                            let row_h = 25.0_f32;

                                            egui::ScrollArea::vertical()
                                                .id_salt("spire_3col_grid_scroll")
                                                .max_height(body_h - 14.0)
                                                .auto_shrink([false, false])
                                                .show(ui, |ui| {
                                                    ui.spacing_mut().item_spacing.y = 3.0;
                                                    if filtered.is_empty() {
                                                        let (er, _) = ui.allocate_exact_size(
                                                            Vec2::new(scroll_inner_w, 80.0),
                                                            Sense::hover(),
                                                        );
                                                        ui.painter().text(
                                                            er.center(),
                                                            egui::Align2::CENTER_CENTER,
                                                            "NO PRESETS MATCH CURRENT FILTER / SEARCH",
                                                            font_id(12.0, FONT_IMPACT),
                                                            P5_MUTED,
                                                        );
                                                    } else {
                                                        for chunk in filtered.chunks(cols) {
                                                            ui.horizontal(|ui| {
                                                                ui.spacing_mut().item_spacing.x = col_gap;
                                                                for (entry_idx, name, cat, is_fac, is_fav) in chunk {
                                                                    let is_cur = *entry_idx == cur_index;
                                                                    let (cr, cresp) = ui.allocate_exact_size(
                                                                        Vec2::new(cell_w, row_h),
                                                                        Sense::click(),
                                                                    );
                                                                    let star_r = Rect::from_min_size(
                                                                        Pos2::new(cr.right() - 20.0, cr.top()),
                                                                        Vec2::new(20.0, row_h),
                                                                    );

                                                                    if cresp.clicked() {
                                                                        if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                                                                            if star_r.contains(pos) {
                                                                                toggle_fav_idx = Some(*entry_idx);
                                                                            } else {
                                                                                load_idx = Some(*entry_idx);
                                                                            }
                                                                        } else {
                                                                            load_idx = Some(*entry_idx);
                                                                        }
                                                                    }

                                                                    let p = ui.painter();
                                                                    let bg = if is_cur {
                                                                        P5_WHITE
                                                                    } else if cresp.hovered() {
                                                                        P5_RED_DEEP
                                                                    } else {
                                                                        P5_PANEL
                                                                    };
                                                                    let border = if is_cur {
                                                                        P5_RED
                                                                    } else if cresp.hovered() {
                                                                        P5_WHITE
                                                                    } else {
                                                                        Color32::from_rgb(42, 42, 54)
                                                                    };
                                                                    p.rect(
                                                                        cr,
                                                                        CornerRadius::ZERO,
                                                                        bg,
                                                                        Stroke::new(if is_cur { 1.6_f32 } else { 1.0_f32 }, border),
                                                                        StrokeKind::Inside,
                                                                    );

                                                                    // Category prefix badge (BS/DR/FX/KY/LD/PD/PL/SQ/SY/IN)
                                                                    let cat_col = category_accent(cat);
                                                                    let pre_r = Rect::from_min_size(
                                                                        Pos2::new(cr.left() + 3.0, cr.top() + 4.0),
                                                                        Vec2::new(22.0, row_h - 8.0),
                                                                    );
                                                                    p.rect_filled(pre_r, CornerRadius::ZERO, P5_BLACK);
                                                                    p.text(
                                                                        pre_r.center(),
                                                                        egui::Align2::CENTER_CENTER,
                                                                        category_prefix(cat),
                                                                        font_id(8.5, FONT_IMPACT),
                                                                        if is_cur { P5_RED } else { cat_col },
                                                                    );

                                                                    // Clipped preset name
                                                                    let name_clip = Rect::from_min_max(
                                                                        Pos2::new(pre_r.right() + 4.0, cr.top()),
                                                                        Pos2::new(star_r.left() - 2.0, cr.bottom()),
                                                                    );
                                                                    let disp_name = if !*is_fac {
                                                                        format!("• {name}")
                                                                    } else {
                                                                        name.clone()
                                                                    };
                                                                    ui.painter_at(name_clip).text(
                                                                        Pos2::new(pre_r.right() + 5.0, cr.center().y),
                                                                        egui::Align2::LEFT_CENTER,
                                                                        disp_name,
                                                                        font_id(10.0, FONT_HEAVY),
                                                                        if is_cur { P5_BLACK } else { P5_WHITE },
                                                                    );

                                                                    // Star toggle
                                                                    p.text(
                                                                        star_r.center(),
                                                                        egui::Align2::CENTER_CENTER,
                                                                        if *is_fav { "★" } else { "☆" },
                                                                        font_id(11.0, FONT_HEAVY),
                                                                        if *is_fav {
                                                                            P5_GOLD
                                                                        } else if is_cur {
                                                                            P5_DIM
                                                                        } else {
                                                                            Color32::from_rgb(76, 76, 90)
                                                                        },
                                                                    );
                                                                }
                                                            });
                                                        }
                                                    }
                                                });
                                        });
                                },
                            );
                        },
                    );

                    // ============================================================
                    // BOTTOM ACTIVE PRESET DETAIL + SAVE AS / DELETE BAR
                    // ============================================================
                    egui::Frame::new()
                        .fill(P5_PANEL)
                        .stroke(Stroke::new(1.5_f32, P5_RED))
                        .inner_margin(egui::Margin::symmetric(7, 5))
                        .show(ui, |ui| {
                            ui.set_width(avail_w - 14.0);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;

                                // Left: Active Preset Badge Card
                                let cat_col = category_accent(&cur_cat);
                                let (emblem_r, _) =
                                    ui.allocate_exact_size(Vec2::new(82.0, 38.0), Sense::hover());
                                let ep = ui.painter();
                                ep.rect_filled(emblem_r, CornerRadius::ZERO, P5_BLACK);
                                ep.rect_stroke(
                                    emblem_r,
                                    CornerRadius::ZERO,
                                    Stroke::new(1.6_f32, cat_col),
                                    StrokeKind::Inside,
                                );
                                draw_p5_icon_badge(
                                    ep,
                                    Pos2::new(emblem_r.left() + 14.0, emblem_r.center().y),
                                    7.0,
                                    icon_for_category(&cur_cat),
                                );
                                ep.text(
                                    Pos2::new(emblem_r.left() + 26.0, emblem_r.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    cur_cat.to_uppercase(),
                                    font_id(9.5, FONT_IMPACT),
                                    cat_col,
                                );

                                // Name + Author + Tags
                                let info_w = 278.0_f32;
                                let (ir, _) =
                                    ui.allocate_exact_size(Vec2::new(info_w, 38.0), Sense::hover());
                                let ip = ui.painter_at(ir);
                                ip.text(
                                    Pos2::new(ir.left(), ir.top() + 9.0),
                                    egui::Align2::LEFT_CENTER,
                                    &cur_name,
                                    font_id(12.0, FONT_IMPACT),
                                    P5_WHITE,
                                );
                                let tags_str = if cur_tags.is_empty() {
                                    String::new()
                                } else {
                                    format!(" • {}", cur_tags.join(", "))
                                };
                                ip.text(
                                    Pos2::new(ir.left(), ir.top() + 26.0),
                                    egui::Align2::LEFT_CENTER,
                                    format!(
                                        "{} ({}){}",
                                        cur_author,
                                        if cur_is_fac { "Factory" } else { "User" },
                                        tags_str
                                    ),
                                    font_id(8.5, FONT_HEAVY),
                                    P5_CYAN,
                                );

                                // Right: SAVE AS + Category Picker + SAVE + DEL
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    if p5_button(ui, "DEL", 40.0, 24.0, false, P5_RED, !cur_is_fac) {
                                        delete_idx = Some(cur_index);
                                    }
                                    if p5_button(ui, "SAVE", 50.0, 24.0, false, P5_GOLD, true) {
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
                                                    "Saved '{}' [{}]",
                                                    browser.save_name.trim(),
                                                    cat
                                                ));
                                                ui_state.set_last_param_label("USER PRESET SAVED");
                                            }
                                            Err(e) => browser.status_msg = Some(e),
                                        }
                                    }
                                    if let Some(sel_cat) = p5_string_dropdown(
                                        ui,
                                        "preset_save_category",
                                        SPIRE_CATEGORIES,
                                        browser.save_cat_idx,
                                        popups,
                                        96.0,
                                        24.0,
                                        P5_GOLD,
                                    ) {
                                        browser.save_cat_idx = sel_cat;
                                    }
                                    egui::Frame::new()
                                        .fill(P5_BG)
                                        .stroke(Stroke::new(1.3_f32, P5_WHITE))
                                        .inner_margin(egui::Margin::symmetric(5, 2))
                                        .show(ui, |ui| {
                                            ui.add(
                                                egui::TextEdit::singleline(&mut browser.save_name)
                                                    .desired_width(146.0)
                                                    .frame(false)
                                                    .text_color(P5_WHITE)
                                                    .font(font_id(10.5, FONT_HEAVY)),
                                            );
                                        });
                                    let (sl, _) =
                                        ui.allocate_exact_size(Vec2::new(54.0, 24.0), Sense::hover());
                                    ui.painter().text(
                                        Pos2::new(sl.left(), sl.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        "SAVE AS:",
                                        font_id(9.5, FONT_IMPACT),
                                        P5_GOLD,
                                    );
                                });
                            });
                        });
                },
            );
        },
    );

    // Apply deferred PresetManager actions
    if next_view_mode.is_some()
        || next_bank.is_some()
        || next_cat.is_some()
        || next_author.is_some()
        || next_tag.is_some()
        || do_rescan
        || load_idx.is_some()
        || toggle_fav_idx.is_some()
        || delete_idx.is_some()
    {
        let mut mgr = presets.lock().unwrap();
        if let Some(vm) = next_view_mode {
            browser.view_mode = vm;
            match vm {
                BrowserViewMode::Folder => {
                    mgr.set_active_bank(PresetBank::All);
                    mgr.set_active_author(None);
                    mgr.set_active_tag(None);
                }
                BrowserViewMode::Author => {
                    mgr.set_active_bank(PresetBank::All);
                    mgr.set_active_category(None);
                    mgr.set_active_tag(None);
                }
                BrowserViewMode::Tag => {
                    mgr.set_active_bank(PresetBank::All);
                    mgr.set_active_category(None);
                    mgr.set_active_author(None);
                }
                BrowserViewMode::Favorite => {
                    mgr.set_active_bank(PresetBank::Favorites);
                    mgr.set_active_author(None);
                    mgr.set_active_tag(None);
                }
            }
        }
        if let Some(b) = next_bank {
            mgr.set_active_bank(b);
        }
        if let Some(c) = next_cat {
            mgr.set_active_category(c.as_deref());
        }
        if let Some(a) = next_author {
            mgr.set_active_author(a.as_deref());
        }
        if let Some(t) = next_tag {
            mgr.set_active_tag(t.as_deref());
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

fn draw_folder_header_row(
    painter: &egui::Painter,
    r: Rect,
    hovered: bool,
    arrow: &str,
    label: &str,
    count: usize,
    selected: bool,
    accent: Color32,
) {
    let bg = if selected {
        P5_WHITE
    } else if hovered {
        P5_RED_DEEP
    } else {
        Color32::from_rgb(26, 26, 36)
    };
    painter.rect(
        r,
        CornerRadius::ZERO,
        bg,
        Stroke::new(1.1_f32, if selected { P5_RED } else { P5_DIM }),
        StrokeKind::Inside,
    );
    painter.text(
        Pos2::new(r.left() + 8.0, r.center().y),
        egui::Align2::CENTER_CENTER,
        arrow,
        font_id(10.0, FONT_HEAVY),
        if selected { P5_RED } else { accent },
    );
    painter.text(
        Pos2::new(r.left() + 18.0, r.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        font_id(9.5, FONT_IMPACT),
        if selected { P5_BLACK } else { P5_WHITE },
    );
    painter.text(
        Pos2::new(r.right() - 5.0, r.center().y),
        egui::Align2::RIGHT_CENTER,
        format!("{count}"),
        font_id(8.5, FONT_HEAVY),
        if selected { P5_RED } else { P5_GOLD },
    );
}

fn draw_tree_row(
    ui: &mut egui::Ui,
    label: &str,
    count: usize,
    selected: bool,
    accent: Color32,
    indented: bool,
    width: f32,
) -> bool {
    let (r, resp) = ui.allocate_exact_size(Vec2::new(width, 22.0), Sense::click());
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
            1.0_f32,
            if selected {
                P5_BLACK
            } else if resp.hovered() {
                P5_WHITE
            } else {
                Color32::from_rgb(44, 44, 56)
            },
        ),
        StrokeKind::Inside,
    );

    let x_base = if indented { r.left() + 12.0 } else { r.left() + 4.0 };
    if selected {
        let cy = r.center().y;
        let wedge = vec![
            Pos2::new(x_base, cy - 4.5),
            Pos2::new(x_base + 7.0, cy),
            Pos2::new(x_base, cy + 4.5),
        ];
        p.add(Shape::convex_polygon(wedge, P5_RED, Stroke::NONE));
    } else {
        p.rect_filled(
            Rect::from_min_size(Pos2::new(x_base, r.top() + 4.0), Vec2::new(4.0, 14.0)),
            CornerRadius::ZERO,
            accent,
        );
    }

    let text_clip = Rect::from_min_max(
        Pos2::new(x_base + 8.0, r.top()),
        Pos2::new(r.right() - 28.0, r.bottom()),
    );
    ui.painter_at(text_clip).text(
        Pos2::new(x_base + 10.0, r.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        font_id(9.5, FONT_HEAVY),
        if selected { P5_BLACK } else { P5_WHITE },
    );

    p.text(
        Pos2::new(r.right() - 5.0, r.center().y),
        egui::Align2::RIGHT_CENTER,
        format!("{count}"),
        font_id(8.5, FONT_HEAVY),
        if selected { P5_RED } else { P5_GOLD },
    );

    resp.clicked()
}
