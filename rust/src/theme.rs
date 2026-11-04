//! Shared colour palette for every screen. The stage, lobby, drawers and the
//! level editor all draw with these so a colour change happens in one place.

use eframe::egui::Color32;

/// Window background.
pub const BG: Color32 = Color32::from_rgb(5, 7, 14);
/// Primary text.
pub const INK: Color32 = Color32::from_rgb(244, 247, 251);
/// Secondary text.
pub const MUTED: Color32 = Color32::from_rgb(142, 150, 170);
/// Tertiary text, disabled controls.
/// About 4.8:1 against BG, so small labels still pass AA contrast.
pub const MUTED2: Color32 = Color32::from_rgb(120, 128, 148);
/// Accent: primary buttons, active toggles, PERFECT judgments.
pub const MINT: Color32 = Color32::from_rgb(124, 247, 196);
/// Text drawn on top of a MINT fill.
pub const INK_ON_MINT: Color32 = Color32::from_rgb(5, 18, 13);
/// MISS judgments and errors.
pub const PINK: Color32 = Color32::from_rgb(255, 93, 158);
/// NICE judgments and warnings.
pub const GOLD: Color32 = Color32::from_rgb(255, 209, 102);
/// OCTAVE judgments.
pub const OCTAVE_BLUE: Color32 = Color32::from_rgb(100, 200, 255);
/// Raised panel background (editor inspector, toasts).
pub const PANEL: Color32 = Color32::from_rgb(9, 13, 23);
/// Slightly lighter panel background.
pub const PANEL2: Color32 = Color32::from_rgb(12, 17, 29);
/// Hairline separators.
pub const LINE: Color32 = Color32::from_rgba_premultiplied(255, 255, 255, 22);

/// `color` with its alpha replaced by `a` (unmultiplied).
pub fn alpha(color: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), a)
}

/// Colour for a results rank, best to worst, on the stage's own ladder
/// (MINT is a hit, GOLD a near hit). Never OCTAVE_BLUE or PINK: those mean
/// an octave slip and a miss, and PINK also marks unmapped songs.
pub fn rank_colour(rank: char) -> Color32 {
    match rank {
        'S' => MINT,
        'A' => GOLD,
        'B' => INK,
        'C' => MUTED,
        _ => MUTED2,
    }
}
