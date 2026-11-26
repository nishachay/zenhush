//! Theme tokens for Zenhush: warm obsidian (Dark) and cream paper (Light)
//! paired with burnt amber (#C8823E) accents.

use egui::Color32;

#[derive(Clone, Copy)]
pub struct Theme {
    pub card: Color32,
    pub inset: Color32,
    pub text: Color32,
    pub subtext: Color32,
    pub bar: Color32,
    pub bar_strong: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub accent_text: Color32,
    pub on_accent: Color32,
    pub edge: Color32,
    pub hover: Color32,
    pub field: Color32,
    pub pressed: Color32,
}

pub const LIGHT: Theme = Theme {
    card: Color32::from_rgb(253, 251, 247),                  // #FDFBF7
    inset: Color32::from_rgb(241, 236, 227),                 // #F1ECE3
    text: Color32::from_rgb(28, 25, 21),                     // #1C1915
    subtext: Color32::from_rgb(108, 99, 86),                 // #6C6356
    bar: Color32::from_rgb(221, 213, 199),                   // #DDD5C7
    bar_strong: Color32::from_rgb(59, 53, 44),               // #3B352C
    accent: Color32::from_rgb(200, 130, 62),                 // #C8823E
    accent_soft: Color32::from_rgba_premultiplied(200, 130, 62, 34),
    accent_text: Color32::from_rgb(143, 91, 30),             // #8F5B1E
    on_accent: Color32::from_rgb(255, 255, 255),             // #FFFFFF
    edge: Color32::from_rgba_premultiplied(0, 0, 0, 22),     // subtle border
    hover: Color32::from_rgb(236, 230, 220),                 // #ECE6DC
    field: Color32::from_rgb(238, 232, 222),                 // #EEE8DE
    pressed: Color32::from_rgb(169, 110, 51),                // #A96E33
};

pub const DARK: Theme = Theme {
    card: Color32::from_rgb(14, 12, 10),                     // #0E0C0A
    inset: Color32::from_rgb(28, 24, 20),                    // #1C1814
    text: Color32::from_rgb(245, 241, 234),                  // #F5F1EA
    subtext: Color32::from_rgb(165, 156, 143),               // #A59C8F
    bar: Color32::from_rgb(56, 49, 41),                      // #383129
    bar_strong: Color32::from_rgb(232, 225, 212),            // #E8E1D4
    accent: Color32::from_rgb(200, 130, 62),                 // #C8823E
    accent_soft: Color32::from_rgba_premultiplied(200, 130, 62, 42),
    accent_text: Color32::from_rgb(226, 168, 108),           // #E2A86C
    on_accent: Color32::from_rgb(255, 255, 255),             // #FFFFFF
    edge: Color32::from_rgba_premultiplied(255, 255, 255, 20),
    hover: Color32::from_rgb(36, 31, 26),                    // #241F1A
    field: Color32::from_rgb(30, 26, 22),                    // #1E1A16
    pressed: Color32::from_rgb(169, 110, 51),                // #A96E33
};
