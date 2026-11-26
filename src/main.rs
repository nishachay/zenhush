//! Zenhush — lightweight top-bar & notch focus audio machine.
//!
//! Live stereo procedural focus soundscapes, atmosphere layering,
//! streaming local audio tracks with Lock-as-Default persistence,
//! global/in-app shortcuts, and single-instance background IPC.

mod app;
mod audio;
mod filemode;
mod icons;
mod ipc;
mod settings;
mod shortcuts;
mod synth;
mod theme;
mod tray;

use eframe::egui;

#[cfg(target_os = "macos")]
fn set_mac_accessory_policy() {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    unsafe {
        let mtm = objc2::foundation::MainThreadMarker::new().unwrap();
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    }
}

fn load_app_icon() -> Option<egui::IconData> {
    let img = image::load_from_memory(include_bytes!("../assets/icon.png"))
        .ok()?
        .into_rgba8();
    let (width, height) = img.dimensions();
    Some(egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    })
}

fn main() {
    let cli = ipc::parse_cli();
    if cli.help_requested {
        ipc::print_help();
        return;
    }
    if cli.install_requested {
        match shortcuts::install_to_user_local() {
            Ok(msg) => println!("{}", msg),
            Err(e) => {
                eprintln!("Install failed: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }
    if cli.uninstall_requested {
        match shortcuts::uninstall_from_user_local() {
            Ok(msg) => println!("{}", msg),
            Err(e) => {
                eprintln!("Uninstall failed: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }

    // If another Zenhush instance is already running, forward the command
    // (or toggle visibility if invoked bare) and exit immediately.
    if let Some(cmd) = cli.command {
        if ipc::try_send_to_existing(cmd) {
            return;
        }
        if matches!(cmd, ipc::RemoteCmd::Quit) {
            return;
        }
    } else if !cli.start_background && ipc::try_send_to_existing(ipc::RemoteCmd::Toggle) {
        return;
    }

    #[cfg(target_os = "macos")]
    set_mac_accessory_policy();

    let loaded_settings = settings::load();
    let start_hidden = cli.start_background || loaded_settings.start_in_background;

    let audio = audio::AudioEngine::new();
    let tray = tray::Tray::new();
    let has_tray = tray.is_some();
    let ipc_server = ipc::IpcServer::start();

    let (init_w, init_h) = match loaded_settings.window_mode {
        settings::WindowMode::NotchPill => (286.0_f32, 42.0_f32),
        settings::WindowMode::Card => (336.0_f32, 344.0_f32),
    };

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([init_w, init_h])
        .with_min_inner_size([240.0, 40.0])
        .with_max_inner_size([400.0, 720.0])
        .with_decorations(false)
        .with_always_on_top()
        .with_resizable(false)
        .with_transparent(true)
        .with_visible(!(start_hidden && has_tray));

    if has_tray {
        viewport = viewport.with_taskbar(false);
    }
    if let Some(icon) = load_app_icon() {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }
    if let Some([x, y]) = loaded_settings.saved_pos {
        if loaded_settings.dock_anchor == settings::DockAnchor::Free {
            viewport = viewport.with_position(egui::pos2(x, y));
        }
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    let _ = eframe::run_native(
        "Zenhush",
        options,
        Box::new(move |cc| {
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
            fonts.families.insert(
                egui::FontFamily::Name("SchibstedGroteskBold".into()),
                vec!["SchibstedGroteskBold".to_owned()],
            );
            fonts.families.insert(
                egui::FontFamily::Name("SchibstedGroteskSemiBold".into()),
                vec!["SchibstedGroteskSemiBold".to_owned()],
            );
            cc.egui_ctx.set_fonts(fonts);

            if let Some(ref srv) = ipc_server {
                srv.bind_egui_ctx(&cc.egui_ctx);
            }
            if let Some(ref tr) = tray {
                tr.bind_egui_ctx(&cc.egui_ctx);
            }

            Ok(Box::new(app::ZenhushApp::new(
                audio,
                tray,
                ipc_server,
                loaded_settings,
                !(start_hidden && has_tray),
            )))
        }),
    );
}
