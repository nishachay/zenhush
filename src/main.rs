//! Zenhush — a light system-tray noise machine.
//!
//! Live brown/white/pink/rain/ocean noise, generated fresh forever,
//! plus a loop of the user's own audio file. Sleep timer, themes,
//! and settings that persist across restarts.
//!
//! Rust rewrite of the Python + PySide6 app: same sound (the synth is
//! ported exactly), same behaviour, but a single small native binary
//! instead of a Python interpreter plus Qt.

mod app;
mod audio;
mod filemode;
mod icons;
mod settings;
mod synth;
mod theme;
mod tray;

use eframe::egui;

fn main() {
    // The audio engine never fails to construct: with no device it
    // records the error and the UI shows it instead of "Live".
    let audio = audio::AudioEngine::new();

    // System tray (None when no tray is available; the app still runs).
    let tray = tray::Tray::new();

    // Window height, matching the Python app's _set_fixed_height:
    // BASE_HEIGHT 296, + SETTINGS_HEIGHT 336 when the settings are
    // open, + 28 for the Quit row when there is no tray (the Quit
    // button only shows without a tray). Without that +28 the Quit
    // is crammed against the play button (6px gap, Python has 22px)
    // and the bottom of the card is unbalanced.
    let no_tray = tray.is_none();
    let closed_h = if no_tray { 324.0 } else { 296.0 };
    let open_h = if no_tray { 660.0 } else { 632.0 };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([300.0, closed_h])
            .with_min_inner_size([300.0, closed_h])
            .with_max_inner_size([300.0, open_h])
            .with_decorations(false)
            .with_always_on_top()
            .with_resizable(false)
            .with_transparent(true),
        ..Default::default()
    };

    let _ = eframe::run_native(
        "Zenhush",
        options,
        Box::new(move |cc| {
            // The bundled font: Schibsted Grotesk, the same TTF the
            // Python app loads (SIL OFL 1.1). It becomes the default
            // for all text, so the Rust popup uses the same typeface.
            // The TTF is a variable font (wght 400-900), but egui has
            // no weight support (epaint TODO) and RichText::strong()
            // only changes text color, never weight. So the bold and
            // semibold weights are separate static instances, each
            // registered as its own named family and used explicitly.
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "SchibstedGrotesk".to_owned(),
                std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
                    "../assets/fonts/SchibstedGrotesk.ttf"
                ))),
            );
            fonts.font_data.insert(
                "SchibstedGroteskBold".to_owned(),
                std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
                    "../assets/fonts/SchibstedGrotesk-Bold.ttf"
                ))),
            );
            fonts.font_data.insert(
                "SchibstedGroteskSemiBold".to_owned(),
                std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
                    "../assets/fonts/SchibstedGrotesk-SemiBold.ttf"
                ))),
            );
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "SchibstedGrotesk".to_owned());
            // A named family must be bound to its fonts, or egui
            // panics when the family is used ("not bound to any
            // fonts"). Bind each weight to its own family.
            fonts.families.insert(
                egui::FontFamily::Name("SchibstedGroteskBold".into()),
                vec!["SchibstedGroteskBold".to_owned()],
            );
            fonts.families.insert(
                egui::FontFamily::Name("SchibstedGroteskSemiBold".into()),
                vec!["SchibstedGroteskSemiBold".to_owned()],
            );
            cc.egui_ctx.set_fonts(fonts);
            Ok(Box::new(app::ZenhushApp::new(audio, tray)))
        }),
    );
}
