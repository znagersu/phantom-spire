use nih_plug_egui::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Pos2, Rect, Shape,
    Stroke, StrokeKind, Vec2,
};
use std::f32::consts::TAU;
use std::sync::Arc;

// ============================================================================
// Exact Phantom Voicer / Persona 5 Color Palette
// ============================================================================

pub const P5_BG: Color32 = Color32::from_rgb(11, 11, 14); // #0B0B0E obsidian bg
pub const P5_CARD: Color32 = Color32::from_rgb(12, 12, 16); // #0C0C10 dark card
pub const P5_PANEL: Color32 = Color32::from_rgb(19, 19, 24); // #131318 elevated card
pub const P5_SURFACE: Color32 = Color32::from_rgb(24, 24, 30); // control recessed fill
pub const P5_RED: Color32 = Color32::from_rgb(230, 0, 26); // #E6001A P5 crimson
pub const P5_RED_DEEP: Color32 = Color32::from_rgb(140, 4, 18); // #8C0412 deep crimson
pub const P5_WHITE: Color32 = Color32::from_rgb(248, 248, 250); // #F8F8FA paper white
pub const P5_MUTED: Color32 = Color32::from_rgb(184, 184, 200); // muted cool silver
pub const P5_DIM: Color32 = Color32::from_rgb(95, 95, 112); // dimmed border/inactive
pub const P5_GOLD: Color32 = Color32::from_rgb(247, 184, 1); // #F7B801 amber gold
pub const P5_CYAN: Color32 = Color32::from_rgb(28, 232, 255); // #1CE8FF electric cyan
pub const P5_MAGENTA: Color32 = Color32::from_rgb(255, 62, 165); // #FF3EA5 neon magenta
pub const P5_BLACK: Color32 = Color32::from_rgb(0, 0, 0); // #000000 ink black

pub const FONT_IMPACT: &str = "p5_impact";
pub const FONT_HEAVY: &str = "p5_heavy";
pub const FONT_SERIF: &str = "p5_serif";
pub const FONT_COMIC: &str = "p5_comic";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum P5Icon {
    Square,
    Triangle,
    Circle,
    Cross,
}

pub fn font_id(size: f32, family: &str) -> FontId {
    FontId::new(size, FontFamily::Name(family.into()))
}

pub fn setup_p5_fonts(egui_ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    let load_win_font = |fonts: &mut FontDefinitions, key: &str, filename: &str| -> bool {
        let path = format!("C:\\Windows\\Fonts\\{filename}");
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert(key.to_owned(), Arc::new(FontData::from_owned(bytes)));
            let mut fam = vec![key.to_owned()];
            if let Some(default_prop) = fonts.families.get(&FontFamily::Proportional) {
                fam.extend(default_prop.iter().cloned());
            }
            fonts
                .families
                .insert(FontFamily::Name(key.into()), fam);
            true
        } else {
            false
        }
    };

    let has_impact = load_win_font(&mut fonts, FONT_IMPACT, "impact.ttf");
    let has_heavy = load_win_font(&mut fonts, FONT_HEAVY, "ariblk.ttf");
    let has_serif = load_win_font(&mut fonts, FONT_SERIF, "georgiab.ttf");
    let has_comic = load_win_font(&mut fonts, FONT_COMIC, "comicbd.ttf");

    let fallback = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();

    for (loaded, key) in [
        (has_impact, FONT_IMPACT),
        (has_heavy, FONT_HEAVY),
        (has_serif, FONT_SERIF),
        (has_comic, FONT_COMIC),
    ] {
        if !loaded {
            fonts
                .families
                .insert(FontFamily::Name(key.into()), fallback.clone());
        }
    }

    egui_ctx.set_fonts(fonts);
}

pub fn apply_p5_style(egui_ctx: &egui::Context) {
    let mut style = (*egui_ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = P5_BG;
    style.visuals.window_fill = P5_CARD;
    style.visuals.window_stroke = Stroke::new(2.0_f32, P5_WHITE);
    style.visuals.widgets.inactive.bg_fill = P5_SURFACE;
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, P5_WHITE);
    style.visuals.widgets.hovered.bg_fill = P5_RED_DEEP;
    style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.5_f32, P5_WHITE);
    style.visuals.widgets.active.bg_fill = P5_RED;
    style.visuals.selection.bg_fill = P5_RED;
    style.visuals.selection.stroke = Stroke::new(1.0_f32, P5_WHITE);
    style.spacing.item_spacing = Vec2::new(6.0, 5.0);
    egui_ctx.set_style(style);
}

#[inline]
pub fn char_hash(idx: usize, ch: char) -> u32 {
    let mut x = (idx as u32).wrapping_mul(0x9E37_79B9) ^ (ch as u32).wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    x = x.wrapping_mul(0xC2B2_AE35);
    x ^ (x >> 16)
}

/// True Persona 5 contour-hugging 3D sticker typography matching `phantom-voicer`.
#[allow(clippy::too_many_arguments)]
pub fn draw_p5_3d_text(
    painter: &egui::Painter,
    pos: Pos2,
    align: egui::Align2,
    text: &str,
    base_size: f32,
    fg_color: Color32,
    rim_color: Color32,
    family_name: &str,
) -> Rect {
    let font_fam = FontFamily::Name(family_name.into());

    struct CharGlyph {
        ch: char,
        rect: Rect,
        tilt_top: f32,
        tilt_bot: f32,
        font: FontId,
    }

    let mut items: Vec<CharGlyph> = Vec::with_capacity(text.len());
    let mut cursor_x = 0.0_f32;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;

    for (idx, ch) in text.chars().enumerate() {
        if ch == ' ' {
            cursor_x += base_size * 0.28;
            continue;
        }

        let h = char_hash(idx, ch);
        let is_lead = idx == 0 || text.chars().nth(idx.saturating_sub(1)) == Some(' ');
        let scale = if is_lead {
            1.10
        } else {
            0.94 + ((h & 0xFF) as f32 / 255.0) * 0.12
        };
        let dy = (((h >> 8) & 0xFF) as f32 / 255.0 - 0.5) * (base_size * 0.13);
        let tilt_top = (((h >> 16) & 0xFF) as f32 / 255.0 - 0.5) * 3.2;
        let tilt_bot = (((h >> 24) & 0xFF) as f32 / 255.0 - 0.5) * 3.2;

        let font = FontId::new(base_size * scale, font_fam.clone());
        let g = painter.layout_no_wrap(ch.to_string(), font.clone(), fg_color);
        let gsize = g.size();

        let r = Rect::from_min_size(Pos2::new(cursor_x, dy - gsize.y * 0.5), gsize);
        min_y = min_y.min(r.top());
        max_y = max_y.max(r.bottom());

        items.push(CharGlyph {
            ch,
            rect: r,
            tilt_top,
            tilt_bot,
            font,
        });

        cursor_x += (gsize.x * 0.90).max(base_size * 0.24);
    }

    if items.is_empty() {
        return Rect::from_center_size(pos, Vec2::ZERO);
    }

    let total_w = items.last().map(|i| i.rect.right()).unwrap_or(0.0);
    let total_h = (max_y - min_y).max(base_size);
    let anchor = align.anchor_size(pos, Vec2::new(total_w, total_h));
    let offset = Vec2::new(anchor.left(), anchor.top() - min_y);

    // Pass 1: 3D depth drop shadow + outer sticker rim
    let rim_pad = Vec2::new((base_size * 0.20).max(3.0), (base_size * 0.15).max(2.2));
    for item in &items {
        let r = item.rect.translate(offset).expand2(rim_pad);
        let poly = vec![
            Pos2::new(r.left() + item.tilt_top, r.top() - 1.0),
            Pos2::new(r.right() + item.tilt_top, r.top() + 0.5),
            Pos2::new(r.right() - item.tilt_bot, r.bottom() + 1.0),
            Pos2::new(r.left() - item.tilt_bot, r.bottom() - 0.5),
        ];
        let shadow: Vec<Pos2> = poly.iter().map(|p| *p + Vec2::new(2.2, 2.5)).collect();
        painter.add(Shape::convex_polygon(shadow, P5_BLACK, Stroke::NONE));
        painter.add(Shape::convex_polygon(poly, rim_color, Stroke::NONE));
    }

    // Pass 2: Fused heavy black inner sticker silhouette
    let inner_pad = Vec2::new((base_size * 0.12).max(1.8), (base_size * 0.08).max(1.2));
    for item in &items {
        let r = item.rect.translate(offset).expand2(inner_pad);
        let poly = vec![
            Pos2::new(r.left() + item.tilt_top, r.top() - 0.5),
            Pos2::new(r.right() + item.tilt_top, r.top() + 0.5),
            Pos2::new(r.right() - item.tilt_bot, r.bottom() + 0.5),
            Pos2::new(r.left() - item.tilt_bot, r.bottom() - 0.5),
        ];
        painter.add(Shape::convex_polygon(poly, P5_BLACK, Stroke::NONE));
    }

    // Pass 3: 3D black glyph extrusion + outline
    let extrude_offsets = [
        Vec2::new(2.2, 2.4),
        Vec2::new(1.4, 1.6),
        Vec2::new(-1.1, 0.0),
        Vec2::new(1.1, 0.0),
        Vec2::new(0.0, -1.1),
        Vec2::new(0.0, 1.1),
    ];
    for item in &items {
        let ch_pos = item.rect.min + offset;
        for off in extrude_offsets {
            let g_blk = painter.layout_no_wrap(item.ch.to_string(), item.font.clone(), P5_BLACK);
            painter.galley(ch_pos + off, g_blk, P5_BLACK);
        }
    }

    // Pass 4: Crisp bright foreground glyphs
    for item in &items {
        let ch_pos = item.rect.min + offset;
        let g_fg = painter.layout_no_wrap(item.ch.to_string(), item.font.clone(), fg_color);
        painter.galley(ch_pos, g_fg, fg_color);
    }

    anchor.expand2(rim_pad)
}

/// Mixed-font, tilted alternating cutout-tile + bubble-outline typography.
pub fn draw_p5_ransom_word(
    painter: &egui::Painter,
    pos: Pos2,
    align: egui::Align2,
    text: &str,
    base_size: f32,
    accent: Color32,
) -> Rect {
    let families = [FONT_HEAVY, FONT_SERIF, FONT_IMPACT, FONT_COMIC];

    struct RansomGlyph {
        ch: char,
        rect: Rect,
        style_kind: u32,
        tilt_top: f32,
        tilt_bot: f32,
        font: FontId,
    }

    let mut items: Vec<RansomGlyph> = Vec::with_capacity(text.len());
    let mut cx = 0.0_f32;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;

    for (idx, ch) in text.chars().enumerate() {
        if ch == ' ' {
            cx += base_size * 0.34;
            continue;
        }
        let h = char_hash(idx + 17, ch);
        let fam = families[(h as usize) % families.len()];
        let scale = 0.90 + (((h >> 4) & 0xFF) as f32 / 255.0) * 0.22;
        let dy = (((h >> 12) & 0xFF) as f32 / 255.0 - 0.5) * (base_size * 0.16);
        let tilt_top = (((h >> 20) & 0xFF) as f32 / 255.0 - 0.5) * 4.2;
        let tilt_bot = (((h >> 24) & 0xFF) as f32 / 255.0 - 0.5) * 4.2;
        let style_kind = (h >> 2) % 4;

        let font = FontId::new(base_size * scale, FontFamily::Name(fam.into()));
        let g = painter.layout_no_wrap(ch.to_string(), font.clone(), P5_WHITE);
        let gsize = g.size();
        let r = Rect::from_min_size(Pos2::new(cx, dy - gsize.y * 0.5), gsize);
        min_y = min_y.min(r.top());
        max_y = max_y.max(r.bottom());

        items.push(RansomGlyph {
            ch,
            rect: r,
            style_kind,
            tilt_top,
            tilt_bot,
            font,
        });
        cx += (gsize.x + 4.0).max(base_size * 0.52);
    }

    if items.is_empty() {
        return Rect::from_center_size(pos, Vec2::ZERO);
    }

    let total_w = items.last().map(|i| i.rect.right()).unwrap_or(0.0);
    let total_h = (max_y - min_y).max(base_size);
    let anchor = align.anchor_size(pos, Vec2::new(total_w, total_h));
    let offset = Vec2::new(anchor.left(), anchor.top() - min_y);

    for item in &items {
        let r = item.rect.translate(offset);
        let box_r = r.expand2(Vec2::new(3.0, 2.0));
        let poly = vec![
            Pos2::new(box_r.left() + item.tilt_top, box_r.top() - 1.0),
            Pos2::new(box_r.right() + item.tilt_top, box_r.top() + 1.0),
            Pos2::new(box_r.right() - item.tilt_bot, box_r.bottom() + 1.0),
            Pos2::new(box_r.left() - item.tilt_bot, box_r.bottom() - 1.0),
        ];
        let shadow: Vec<Pos2> = poly.iter().map(|p| *p + Vec2::new(2.0, 2.2)).collect();

        match item.style_kind {
            0 => {
                painter.add(Shape::convex_polygon(shadow, P5_BLACK, Stroke::NONE));
                painter.add(Shape::convex_polygon(
                    poly,
                    P5_BLACK,
                    Stroke::new(1.5_f32, P5_WHITE),
                ));
                let g = painter.layout_no_wrap(item.ch.to_string(), item.font.clone(), P5_WHITE);
                painter.galley(r.min, g, P5_WHITE);
            }
            1 => {
                painter.add(Shape::convex_polygon(shadow, P5_BLACK, Stroke::NONE));
                painter.add(Shape::convex_polygon(
                    poly,
                    P5_WHITE,
                    Stroke::new(2.0_f32, P5_BLACK),
                ));
                let g = painter.layout_no_wrap(item.ch.to_string(), item.font.clone(), P5_BLACK);
                painter.galley(r.min, g, P5_BLACK);
            }
            2 => {
                painter.add(Shape::convex_polygon(shadow, P5_BLACK, Stroke::NONE));
                painter.add(Shape::convex_polygon(
                    poly,
                    accent,
                    Stroke::new(1.8_f32, P5_BLACK),
                ));
                let fg = if accent == P5_GOLD { P5_BLACK } else { P5_WHITE };
                let g = painter.layout_no_wrap(item.ch.to_string(), item.font.clone(), fg);
                painter.galley(r.min, g, fg);
            }
            _ => {
                for off in [
                    Vec2::new(2.4, 2.4),
                    Vec2::new(-1.8, 0.0),
                    Vec2::new(1.8, 0.0),
                    Vec2::new(0.0, -1.8),
                    Vec2::new(0.0, 1.8),
                    Vec2::new(-1.3, -1.3),
                    Vec2::new(1.3, -1.3),
                    Vec2::new(-1.3, 1.3),
                ] {
                    let g_b =
                        painter.layout_no_wrap(item.ch.to_string(), item.font.clone(), P5_BLACK);
                    painter.galley(r.min + off, g_b, P5_BLACK);
                }
                let g_w = painter.layout_no_wrap(item.ch.to_string(), item.font.clone(), P5_WHITE);
                painter.galley(r.min, g_w, P5_WHITE);
            }
        }
    }

    anchor.expand2(Vec2::new(4.0, 3.0))
}

/// Exact Phantom Voicer ransom-note logo strip adapted for "P H a N t o M  SPIRE".
pub fn draw_ransom_logo(painter: &egui::Painter, origin: Pos2, width: f32) {
    let strip = Rect::from_min_size(origin, Vec2::new(width, 40.0));
    painter.rect_filled(strip.translate(Vec2::new(3.5, 3.5)), CornerRadius::ZERO, P5_BLACK);
    painter.rect(
        strip,
        CornerRadius::ZERO,
        P5_WHITE,
        Stroke::new(2.5_f32, P5_BLACK),
        StrokeKind::Inside,
    );

    let tiles = [
        ("P", P5_BLACK, P5_WHITE, -2.5_f32, FONT_HEAVY),
        ("H", P5_WHITE, P5_RED, 2.0_f32, FONT_SERIF),
        ("a", P5_RED, P5_BLACK, -1.5_f32, FONT_SERIF),
        ("N", P5_BLACK, P5_WHITE, 2.5_f32, FONT_IMPACT),
        ("t", P5_WHITE, P5_BLACK, -2.0_f32, FONT_SERIF),
        ("o", P5_RED, P5_WHITE, 1.5_f32, FONT_HEAVY),
        ("M", P5_BLACK, P5_WHITE, -1.5_f32, FONT_IMPACT),
    ];

    let mut x = origin.x + 6.0;
    for (ch, bg, fg, dy, fam) in tiles {
        let tr = Rect::from_min_size(Pos2::new(x, origin.y + 6.5 + dy), Vec2::new(22.0, 26.0));
        painter.rect_filled(tr.translate(Vec2::new(2.0, 2.0)), CornerRadius::ZERO, P5_BLACK);
        painter.rect(
            tr,
            CornerRadius::ZERO,
            bg,
            Stroke::new(1.8_f32, P5_BLACK),
            StrokeKind::Inside,
        );
        painter.text(
            tr.center(),
            egui::Align2::CENTER_CENTER,
            ch,
            FontId::new(17.0, FontFamily::Name(fam.into())),
            fg,
        );
        x += 23.2;
    }

    draw_p5_3d_text(
        painter,
        Pos2::new(x + 5.0, origin.y + 20.0),
        egui::Align2::LEFT_CENTER,
        "SPIRE",
        16.5,
        P5_WHITE,
        P5_RED,
        FONT_IMPACT,
    );
}

pub fn draw_p5_icon_badge(painter: &egui::Painter, center: Pos2, radius: f32, icon: P5Icon) {
    painter.circle_filled(center + Vec2::new(2.0, 2.2), radius + 1.5, P5_BLACK);
    painter.circle(center, radius + 1.5, P5_WHITE, Stroke::NONE);
    painter.circle_filled(center, radius, P5_BLACK);

    let ir = radius * 0.52;
    match icon {
        P5Icon::Square => {
            painter.rect_stroke(
                Rect::from_center_size(center, Vec2::splat(ir * 1.85)),
                CornerRadius::ZERO,
                Stroke::new(2.2_f32, P5_MAGENTA),
                StrokeKind::Middle,
            );
        }
        P5Icon::Triangle => {
            let pts = vec![
                Pos2::new(center.x, center.y - ir * 1.05),
                Pos2::new(center.x + ir * 1.0, center.y + ir * 0.85),
                Pos2::new(center.x - ir * 1.0, center.y + ir * 0.85),
            ];
            painter.add(Shape::closed_line(pts, Stroke::new(2.2_f32, P5_CYAN)));
        }
        P5Icon::Circle => {
            painter.circle_stroke(center, ir * 0.95, Stroke::new(2.2_f32, P5_RED));
        }
        P5Icon::Cross => {
            let d = ir * 0.85;
            painter.line_segment(
                [
                    Pos2::new(center.x - d, center.y - d),
                    Pos2::new(center.x + d, center.y + d),
                ],
                Stroke::new(2.2_f32, P5_GOLD),
            );
            painter.line_segment(
                [
                    Pos2::new(center.x + d, center.y - d),
                    Pos2::new(center.x - d, center.y + d),
                ],
                Stroke::new(2.2_f32, P5_GOLD),
            );
        }
    }
}

pub fn draw_star_outline(
    painter: &egui::Painter,
    center: Pos2,
    outer_r: f32,
    inner_r: f32,
    rotation: f32,
    stroke: Stroke,
) {
    let mut pts = Vec::with_capacity(10);
    for i in 0..10 {
        let ang = rotation + (i as f32) * (TAU / 10.0);
        let r = if i % 2 == 0 { outer_r } else { inner_r };
        pts.push(Pos2::new(center.x + ang.cos() * r, center.y + ang.sin() * r));
    }
    painter.add(Shape::closed_line(pts, stroke));
}

pub fn draw_crimson_command_bg(painter: &egui::Painter, zone_rect: Rect) {
    painter.rect_filled(zone_rect, CornerRadius::same(4), P5_RED_DEEP);
    let inner_crimson = Rect::from_min_max(
        zone_rect.min,
        Pos2::new(zone_rect.right() - 26.0, zone_rect.bottom() - 22.0),
    );
    painter.rect_filled(inner_crimson, CornerRadius::ZERO, P5_RED);

    let slash = vec![
        Pos2::new(zone_rect.right() - 72.0, zone_rect.bottom()),
        Pos2::new(zone_rect.right(), zone_rect.bottom() - 116.0),
        Pos2::new(zone_rect.right(), zone_rect.bottom()),
    ];
    painter.add(Shape::convex_polygon(slash, P5_BG, Stroke::NONE));

    // Signature halftone dots fading from top-right corner
    for row in 0..9 {
        for col in 0..9 {
            let r = 3.2 - (row as f32 + col as f32) * 0.17;
            if r > 0.6 {
                let cx = zone_rect.right() - 12.0 - col as f32 * 11.0;
                let cy = zone_rect.top() + 12.0 + row as f32 * 11.0;
                painter.circle_filled(
                    Pos2::new(cx, cy),
                    r,
                    Color32::from_rgba_unmultiplied(255, 255, 255, 38),
                );
            }
        }
    }

    painter.rect_stroke(
        zone_rect,
        CornerRadius::same(4),
        Stroke::new(2.5_f32, P5_BLACK),
        StrokeKind::Inside,
    );
}

pub fn draw_starburst_bg(painter: &egui::Painter, zone_rect: Rect) {
    painter.rect_filled(zone_rect, CornerRadius::same(4), P5_CARD);

    let step = 86.0;
    let cols = (zone_rect.width() / step).ceil() as i32 + 1;
    let rows = (zone_rect.height() / step).ceil() as i32 + 1;
    for r in 0..rows {
        for c in 0..cols {
            let offset_x = if r % 2 == 0 { 0.0 } else { step * 0.5 };
            let center = Pos2::new(
                zone_rect.left() + c as f32 * step + offset_x,
                zone_rect.top() + r as f32 * step,
            );
            draw_star_outline(
                painter,
                center,
                34.0,
                14.0,
                0.25,
                Stroke::new(2.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 28)),
            );
            draw_star_outline(
                painter,
                center,
                18.0,
                7.5,
                0.25,
                Stroke::new(1.5_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 16)),
            );
        }
    }

    painter.rect_stroke(
        zone_rect,
        CornerRadius::same(4),
        Stroke::new(2.5_f32, P5_BLACK),
        StrokeKind::Inside,
    );
}

pub fn draw_active_note_trapezoid(
    painter: &egui::Painter,
    trap_rect: Rect,
    header_label: &str,
    readout: &str,
) {
    let trap_shadow = vec![
        Pos2::new(trap_rect.left() + 14.0, trap_rect.top() + 5.0),
        Pos2::new(trap_rect.right() + 5.0, trap_rect.top() + 5.0),
        Pos2::new(trap_rect.right() - 6.0, trap_rect.bottom() + 5.0),
        Pos2::new(trap_rect.left() + 4.0, trap_rect.bottom() + 5.0),
    ];
    painter.add(Shape::convex_polygon(trap_shadow, P5_RED, Stroke::NONE));

    let trap_pts = vec![
        Pos2::new(trap_rect.left() + 10.0, trap_rect.top()),
        Pos2::new(trap_rect.right(), trap_rect.top()),
        Pos2::new(trap_rect.right() - 10.0, trap_rect.bottom()),
        Pos2::new(trap_rect.left(), trap_rect.bottom()),
    ];
    painter.add(Shape::convex_polygon(
        trap_pts,
        P5_WHITE,
        Stroke::new(2.5_f32, P5_BLACK),
    ));

    painter.text(
        Pos2::new(trap_rect.center().x, trap_rect.top() + 5.0),
        egui::Align2::CENTER_TOP,
        header_label,
        font_id(10.0, FONT_HEAVY),
        P5_RED,
    );

    draw_p5_3d_text(
        painter,
        Pos2::new(trap_rect.center().x, trap_rect.bottom() - 19.0),
        egui::Align2::CENTER_CENTER,
        readout,
        21.0,
        P5_WHITE,
        P5_RED,
        FONT_IMPACT,
    );
}

pub fn icon_for_category(cat: &str) -> P5Icon {
    let lower = cat.to_ascii_lowercase();
    if lower.contains("bass") || lower.contains("acid") || lower.contains("drum") {
        P5Icon::Triangle
    } else if lower.contains("lead") || lower.contains("chord") || lower.contains("brass") || lower.contains("synth") {
        P5Icon::Circle
    } else if lower.contains("arp") || lower.contains("seq") || lower.contains("fx") || lower.contains("pluck") {
        P5Icon::Cross
    } else {
        P5Icon::Square
    }
}

pub fn midi_note_name(midi: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let pitch = (midi % 12) as usize;
    let octave = (midi as i32 / 12) - 1;
    format!("{}{}", NAMES[pitch], octave)
}
