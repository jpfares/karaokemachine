//! Painter-drawn buttons shared by the stage, lobby, drawers and editor.
//!
//! Every widget here follows one convention: it registers the hit area with
//! `ui.interact`, paints itself, and returns `true` on the frame it was
//! clicked. Callers never pair a paint call with a separate click check.
//!
//! Two visual families exist because the singing screens and the editor were
//! styled separately:
//!
//! - `pill_*`, `round_button`, `glyph_button`: the stage and library look
//!   (fully rounded pills, proportional text, no hover state).
//! - `button`, `toggle`: the editor look (7 px corners, monospace text, hover
//!   and disabled states).

use crate::theme::{alpha, INK, INK_ON_MINT, MINT, MUTED, MUTED2};
use eframe::egui::{Align2, Color32, FontId, Id, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

/// A bare clickable area with an explicit id, for hit regions that paint
/// themselves (timeline, album tiles, list rows, scrims).
pub fn click(ui: &mut Ui, rect: Rect, id: impl std::hash::Hash) -> bool {
    ui.interact(rect, Id::new(id), Sense::click()).clicked()
}

fn widget_id(kind: &'static str, text: &str, rect: Rect) -> Id {
    Id::new((kind, text, rect.left() as i32, rect.top() as i32))
}

// --- Stage / library family ------------------------------------------------

/// Fully rounded text button. `primary` fills it with MINT.
pub fn pill_button(ui: &mut Ui, rect: Rect, text: &str, primary: bool) -> bool {
    let clicked = ui
        .interact(rect, widget_id("pill_button", text, rect), Sense::click())
        .clicked();
    let p = ui.painter();
    let (fill, fg) = if primary {
        (MINT, INK_ON_MINT)
    } else {
        (Color32::from_rgba_unmultiplied(9, 13, 23, 220), INK)
    };
    p.rect_filled(rect, rect.height() / 2.0, fill);
    if !primary {
        p.rect_stroke(
            rect,
            rect.height() / 2.0,
            Stroke::new(1.0f32, Color32::from_white_alpha(35)),
            StrokeKind::Inside,
        );
    }
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(12.0),
        fg,
    );
    clicked
}

/// Fully rounded toggle chip with small monospace text.
pub fn pill_toggle(ui: &mut Ui, rect: Rect, text: &str, active: bool) -> bool {
    let clicked = ui
        .interact(rect, widget_id("pill_toggle", text, rect), Sense::click())
        .clicked();
    let p = ui.painter();
    let fill = if active {
        Color32::from_rgba_unmultiplied(124, 247, 196, 22)
    } else {
        Color32::from_rgba_unmultiplied(255, 255, 255, 7)
    };
    p.rect_filled(rect, rect.height() / 2.0, fill);
    p.rect_stroke(
        rect,
        rect.height() / 2.0,
        Stroke::new(
            1.0f32,
            if active {
                alpha(MINT, 90)
            } else {
                Color32::from_white_alpha(28)
            },
        ),
        StrokeKind::Inside,
    );
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::monospace(10.0),
        if active { MINT } else { MUTED },
    );
    clicked
}

/// Square icon button showing a short glyph or abbreviation.
pub fn glyph_button(ui: &mut Ui, rect: Rect, glyph: &str) -> bool {
    let clicked = ui
        .interact(rect, widget_id("glyph_button", glyph, rect), Sense::click())
        .clicked();
    let p = ui.painter();
    p.rect_filled(rect, 8.0, Color32::from_rgba_unmultiplied(255, 255, 255, 8));
    let size = if glyph.chars().count() <= 1 {
        15.0
    } else {
        10.0
    };
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::monospace(size),
        INK,
    );
    clicked
}

/// Circular transport button (play, restart).
pub fn round_button(ui: &mut Ui, rect: Rect, glyph: &str, primary: bool) -> bool {
    let clicked = ui
        .interact(rect, widget_id("round_button", glyph, rect), Sense::click())
        .clicked();
    let p = ui.painter();
    p.circle_filled(
        rect.center(),
        rect.width() / 2.0,
        if primary {
            MINT
        } else {
            Color32::from_rgba_unmultiplied(255, 255, 255, 12)
        },
    );
    let font_size = if glyph.len() > 2 {
        9.0
    } else if primary {
        20.0
    } else {
        17.0
    };
    p.text(
        rect.center() + Vec2::new(0.0, -1.0),
        Align2::CENTER_CENTER,
        glyph,
        FontId::monospace(font_size),
        if primary { INK_ON_MINT } else { MUTED },
    );
    clicked
}

// --- Editor family ---------------------------------------------------------

/// Rounded-rectangle text button with hover and disabled states.
pub fn button(ui: &mut Ui, rect: Rect, text: &str, primary: bool, enabled: bool) -> bool {
    let response = ui.interact(
        rect,
        widget_id("editor_button", text, rect),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let hovered = response.hovered() && enabled;
    let p = ui.painter();
    let fill = if primary {
        if hovered {
            Color32::from_rgb(152, 255, 213)
        } else {
            MINT
        }
    } else if hovered {
        Color32::from_rgb(24, 32, 49)
    } else {
        Color32::from_rgba_unmultiplied(255, 255, 255, 9)
    };
    p.rect_filled(
        rect,
        7.0,
        if enabled {
            fill
        } else {
            Color32::from_white_alpha(5)
        },
    );
    if !primary {
        p.rect_stroke(
            rect,
            7.0,
            Stroke::new(
                1.0f32,
                if enabled {
                    Color32::from_white_alpha(35)
                } else {
                    Color32::from_white_alpha(12)
                },
            ),
            StrokeKind::Inside,
        );
    }
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::monospace(if text.len() > 8 { 9.0 } else { 10.0 }),
        if !enabled {
            MUTED2
        } else if primary {
            INK_ON_MINT
        } else {
            INK
        },
    );
    enabled && response.clicked()
}

/// Rounded-rectangle toggle chip.
pub fn toggle(ui: &mut Ui, rect: Rect, text: &str, active: bool) -> bool {
    let response = ui.interact(rect, widget_id("editor_toggle", text, rect), Sense::click());
    let p = ui.painter();
    let fill = if active {
        Color32::from_rgba_unmultiplied(124, 247, 196, 24)
    } else {
        Color32::from_rgba_unmultiplied(255, 255, 255, 7)
    };
    p.rect_filled(rect, 7.0, fill);
    p.rect_stroke(
        rect,
        7.0,
        Stroke::new(
            1.0f32,
            if active {
                alpha(MINT, 95)
            } else {
                Color32::from_white_alpha(26)
            },
        ),
        StrokeKind::Inside,
    );
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::monospace(if text.len() > 10 { 8.5 } else { 10.0 }),
        if active { MINT } else { MUTED },
    );
    response.clicked()
}
