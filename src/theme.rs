//! Theme colors, ported exactly from the Python app's THEMES dict
//! (same hex values, same warm palette, same accent #C8823E).
//! Contrast was verified in the Python review: light status 4.80:1,
//! play icon on accent 3.13:1.

use egui::Color32;

#[derive(Clone, Copy)]
pub struct Theme {
    pub window: Color32,
    pub card: Color32,
    pub inset: Color32,
    pub text: Color32,
    pub subtext: Color32,
    pub bar: Color32,
    pub bar_strong: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub on_accent: Color32,
    pub outline: Color32,
    pub edge: Color32,
    pub hover: Color32,
    pub field: Color32,
    pub pressed: Color32,
    pub switch_off: Color32,
}

pub const LIGHT: Theme = Theme {
    window: Color32::from_rgb(243, 239, 232),      // #F3EFE8
    card: Color32::from_rgb(253, 251, 247),        // #FDFBF7
    inset: Color32::from_rgb(240, 235, 226),       // #F0EBE2
    text: Color32::from_rgb(28, 25, 21),           // #1C1915
    subtext: Color32::from_rgb(111, 102, 89),      // #6F6659
    bar: Color32::from_rgb(221, 213, 199),         // #DDD5C7
    bar_strong: Color32::from_rgb(59, 53, 44),     // #3B352C
    accent: Color32::from_rgb(200, 130, 62),       // #C8823E
    accent_text: Color32::from_rgb(143, 91, 30),   // #8F5B1E
    on_accent: Color32::from_rgb(255, 255, 255),   // #FFFFFF
    outline: Color32::from_rgb(220, 212, 198),     // #DCD4C6
    edge: Color32::from_rgba_premultiplied(0, 0, 0, 18), // black at 7%
    hover: Color32::from_rgb(239, 233, 223),       // #EFE9DF
    field: Color32::from_rgb(240, 235, 226),       // #F0EBE2
    pressed: Color32::from_rgb(169, 110, 51),      // #A96E33
    switch_off: Color32::from_rgb(217, 208, 192),  // #D9D0C0
};

pub const DARK: Theme = Theme {
    window: Color32::from_rgb(16, 14, 12),         // #100E0C
    card: Color32::from_rgb(11, 10, 8),            // #0B0A08
    inset: Color32::from_rgb(34, 30, 25),          // #221E19
    text: Color32::from_rgb(245, 241, 234),        // #F5F1EA
    subtext: Color32::from_rgb(168, 159, 146),     // #A89F92
    bar: Color32::from_rgb(58, 51, 43),            // #3A332B
    bar_strong: Color32::from_rgb(232, 225, 212),  // #E8E1D4
    accent: Color32::from_rgb(200, 130, 62),       // #C8823E
    accent_text: Color32::from_rgb(221, 162, 102), // #DDA266
    on_accent: Color32::from_rgb(255, 255, 255),   // #FFFFFF
    outline: Color32::from_rgb(58, 51, 43),        // #3A332B
    edge: Color32::from_rgba_premultiplied(255, 255, 255, 16), // white at 6%
    hover: Color32::from_rgb(29, 26, 21),          // #1D1A15
    field: Color32::from_rgb(34, 30, 25),          // #221E19
    pressed: Color32::from_rgb(169, 110, 51),      // #A96E33
    switch_off: Color32::from_rgb(74, 66, 56),     // #4A4238
};
