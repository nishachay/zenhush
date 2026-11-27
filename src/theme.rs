//! Theme tokens for Zenhush: warm obsidian (Dark) and cream paper (Light)
//! paired with burnt amber (#C8823E) accents.
//!
//! All text/background pairs are WCAG 2.2 AA verified:
//! - LIGHT text #1C1915 on #FDFBF7: 16.9:1
//! - DARK  text #F5F1EA on #0E0C0A: 17.3:1
//! - Espresso #14100C (`on_accent`) on #C8823E (`accent`): 6.06:1 (AA Pass!)

use egui::Color32;

#[derive(Clone, Copy)]
pub struct Theme {
    pub card: Color32,
    pub inset: Color32,
    pub text: Color32,
    pub subtext: Color32,
    pub bar: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub on_accent: Color32,
    pub edge: Color32,
    pub hover: Color32,
    pub pressed: Color32,
}

pub const LIGHT: Theme = Theme {
    card: Color32::from_rgb(253, 251, 247),                  // #FDFBF7
    inset: Color32::from_rgb(241, 236, 227),                 // #F1ECE3
    text: Color32::from_rgb(28, 25, 21),                     // #1C1915 (16.9:1)
    subtext: Color32::from_rgb(97, 88, 76),                  // #61584C (5.93:1 on inset)
    bar: Color32::from_rgb(212, 203, 188),                   // #D4CBBC
    accent: Color32::from_rgb(200, 130, 62),                 // #C8823E
    accent_text: Color32::from_rgb(138, 84, 24),             // #8A5418 (5.31:1 on inset)
    on_accent: Color32::from_rgb(20, 16, 12),                // #14100C (6.06:1 on #C8823E)
    edge: Color32::from_rgba_premultiplied(0, 0, 0, 24),     // subtle structural edge
    hover: Color32::from_rgb(235, 229, 218),                 // #EBE5DA
    pressed: Color32::from_rgb(180, 114, 50),                // #B47232
};

pub const DARK: Theme = Theme {
    card: Color32::from_rgb(14, 12, 10),                     // #0E0C0A
    inset: Color32::from_rgb(28, 24, 20),                    // #1C1814
    text: Color32::from_rgb(245, 241, 234),                  // #F5F1EA (17.3:1)
    subtext: Color32::from_rgb(176, 167, 154),               // #B0A79A (7.4:1 on inset)
    bar: Color32::from_rgb(66, 58, 49),                      // #423A31
    accent: Color32::from_rgb(200, 130, 62),                 // #C8823E
    accent_text: Color32::from_rgb(226, 168, 108),           // #E2A86C (8.2:1 on inset)
    on_accent: Color32::from_rgb(20, 16, 12),                // #14100C (6.06:1 on #C8823E)
    edge: Color32::from_rgba_premultiplied(255, 255, 255, 22),
    hover: Color32::from_rgb(38, 33, 28),                    // #26211C
    pressed: Color32::from_rgb(180, 114, 50),                // #B47232
};
