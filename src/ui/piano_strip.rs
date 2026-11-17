use crate::ui::theme::{
    draw_p5_3d_text, font_id, FONT_HEAVY, FONT_IMPACT, P5_BLACK, P5_CYAN, P5_GOLD, P5_MUTED,
    P5_PANEL, P5_RED, P5_WHITE,
};
use crate::SharedUiState;
use nih_plug_egui::egui::{
    self, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
};

#[inline]
fn is_black_key(midi: u8) -> bool {
    matches!(midi % 12, 1 | 3 | 6 | 8 | 10)
}

/// Exact Phantom Voicer "Live Monitor" 49-key playable piano strip (C2..C6, MIDI 36..84).
pub fn draw_piano_monitor(
    ui: &mut egui::Ui,
    in_mask: u128,
    out_mask: u128,
    ui_state: &SharedUiState,
    auditioning_note: &mut Option<u8>,
) {
    let start_midi: u8 = 36;
    let end_midi: u8 = 84;
    let white_count = (start_midi..=end_midi).filter(|&m| !is_black_key(m)).count();

    egui::Frame::new()
        .fill(P5_PANEL)
        .stroke(Stroke::new(2.0_f32, P5_WHITE))
        .inner_margin(5.0)
        .show(ui, |ui| {
            let (bar_r, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 20.0), Sense::hover());
            draw_p5_3d_text(
                ui.painter(),
                Pos2::new(bar_r.left() + 4.0, bar_r.center().y),
                egui::Align2::LEFT_CENTER,
                "Live Monitor",
                13.0,
                P5_WHITE,
                P5_WHITE,
                FONT_IMPACT,
            );
            ui.painter().text(
                Pos2::new(bar_r.left() + 112.0, bar_r.center().y),
                egui::Align2::LEFT_CENTER,
                "CLICK KEYS TO PLAY • DRAG TO GLISSANDO",
                font_id(9.5, FONT_HEAVY),
                P5_MUTED,
            );
            ui.painter().text(
                Pos2::new(bar_r.right() - 4.0, bar_r.center().y),
                egui::Align2::RIGHT_CENTER,
                "■ SOUND   ■ BASS   ● INPUT",
                font_id(9.5, FONT_HEAVY),
                P5_GOLD,
            );

            let avail = ui.available_width();
            let height = 62.0;
            let (rect, response) =
                ui.allocate_exact_size(Vec2::new(avail, height), Sense::click_and_drag());
            let painter = ui.painter_at(rect);

            let white_w = rect.width() / white_count as f32;
            let black_w = white_w * 0.62;
            let black_h = rect.height() * 0.60;

            let mut white_rects: Vec<(u8, Rect)> = Vec::with_capacity(32);
            let mut black_rects: Vec<(u8, Rect)> = Vec::with_capacity(24);

            let mut w_idx = 0usize;
            for midi in start_midi..=end_midi {
                if !is_black_key(midi) {
                    let x0 = rect.left() + w_idx as f32 * white_w;
                    let kr = Rect::from_min_size(
                        Pos2::new(x0, rect.top()),
                        Vec2::new(white_w - 1.0, rect.height()),
                    );
                    white_rects.push((midi, kr));
                    w_idx += 1;
                } else {
                    let x_center = rect.left() + w_idx as f32 * white_w;
                    let kr = Rect::from_min_size(
                        Pos2::new(x_center - black_w * 0.5, rect.top()),
                        Vec2::new(black_w, black_h),
                    );
                    black_rects.push((midi, kr));
                }
            }

            // Mouse click / glissando audition pushing into lock-free SharedUiState SPSC queue
            let pointer_down = ui.input(|i| i.pointer.primary_down());
            if (response.hovered() || response.is_pointer_button_down_on()) && pointer_down {
                if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                    let hit = black_rects
                        .iter()
                        .find(|(_, r)| r.contains(pos))
                        .or_else(|| white_rects.iter().find(|(_, r)| r.contains(pos)))
                        .map(|(m, _)| *m);

                    if let Some(note) = hit {
                        if *auditioning_note != Some(note) {
                            if let Some(prev) = *auditioning_note {
                                ui_state.push_audition_note_off(prev);
                            }
                            ui_state.push_audition_note_on(note, 108);
                            *auditioning_note = Some(note);
                        }
                    }
                }
            } else if let Some(prev) = auditioning_note.take() {
                ui_state.push_audition_note_off(prev);
            }

            // Render white keys
            for &(midi, kr) in &white_rects {
                let is_out = (out_mask & (1u128 << midi)) != 0;
                let is_in = (in_mask & (1u128 << midi)) != 0 || *auditioning_note == Some(midi);
                let fill = if is_out {
                    if midi < 48 { P5_GOLD } else { P5_RED }
                } else {
                    P5_WHITE
                };
                painter.rect_filled(kr, CornerRadius::same(2), fill);
                painter.rect_stroke(
                    kr,
                    CornerRadius::same(2),
                    Stroke::new(1.0_f32, P5_BLACK),
                    StrokeKind::Inside,
                );
                if midi % 12 == 0 {
                    let oct = (midi as i32 / 12) - 1;
                    painter.text(
                        Pos2::new(kr.center().x, kr.bottom() - 9.0),
                        egui::Align2::CENTER_CENTER,
                        format!("C{oct}"),
                        font_id(8.0, FONT_HEAVY),
                        if is_out { P5_WHITE } else { Color32::from_rgb(120, 120, 134) },
                    );
                }
                if is_in {
                    painter.circle_filled(
                        Pos2::new(kr.center().x, kr.bottom() - 18.0),
                        3.8,
                        P5_CYAN,
                    );
                }
            }

            // Render black keys
            for &(midi, kr) in &black_rects {
                let is_out = (out_mask & (1u128 << midi)) != 0;
                let is_in = (in_mask & (1u128 << midi)) != 0 || *auditioning_note == Some(midi);
                let fill = if is_out {
                    if midi < 48 { P5_GOLD } else { P5_RED }
                } else {
                    Color32::from_rgb(20, 20, 26)
                };
                painter.rect_filled(kr, CornerRadius::same(2), fill);
                painter.rect_stroke(
                    kr,
                    CornerRadius::same(2),
                    Stroke::new(1.0_f32, Color32::from_rgb(72, 72, 86)),
                    StrokeKind::Inside,
                );
                if is_in {
                    painter.circle_filled(
                        Pos2::new(kr.center().x, kr.bottom() - 7.0),
                        3.2,
                        P5_CYAN,
                    );
                }
            }
        });
}
