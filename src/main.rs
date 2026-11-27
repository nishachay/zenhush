//! Zenhush — minimal top-bar focus audio machine.
//!
//! Architecture:
//! - Background Core (`--daemon` / `--background`): owns stereo audio synthesis,
//!   streaming local audio decoding, MPRIS2 + `org.zenhush.Control` D-Bus server,
//!   global shortcuts, and IPC (~15 MB RSS, 0% idle CPU, zero OpenGL/GPU RAM).
//! - Native GNOME Top-Bar Popover (`zenhush@zenhush.app`): renders the entire
//!   UI directly inside the GNOME top bar on Ubuntu/GNOME Wayland (zero separate
//!   windows, zero laptop heating).
//! - Lightweight On-Demand Fallback Window (`--ui`): used on macOS, Windows,
//!   non-GNOME Linux, or first-run GNOME before session activation.

mod app;
mod audio;
mod bridge;
mod daemon;
mod filemode;
mod gnome_ext;
mod icons;
mod ipc;
mod mpris;
mod settings;
mod shortcuts;
mod synth;
mod theme;
mod tray;

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use eframe::egui;

#[cfg(target_os = "macos")]
fn set_mac_accessory_policy() {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    use objc2_foundation::MainThreadMarker;
    if let Some(mtm) = MainThreadMarker::new() {
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

fn spawn_background_daemon() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let mut cmd = Command::new(exe);
    cmd.arg("--daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if cmd.spawn().is_err() {
        return false;
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if bridge::bridge_ready() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(12));
    }
    bridge::bridge_ready()
}

fn run_ui_window() {
    let client = match bridge::BridgeClient::connect() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("zenhush ui bridge error: {}", e);
            return;
        }
    };

    let loaded = settings::load();
    let init_w = 304.0_f32;
    let init_h = 272.0_f32;

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([init_w, init_h])
        .with_min_inner_size([260.0, 260.0])
        .with_max_inner_size([360.0, 440.0])
        .with_decorations(false)
        .with_always_on_top()
        .with_resizable(false)
        .with_taskbar(false);

    // Enable window transparency where composited reliably (macOS/Windows).
    // On Linux Wayland/XWayland, opaque corners avoid black alpha-clipping glitches.
    #[cfg(not(target_os = "linux"))]
    {
        viewport = viewport.with_transparent(true);
    }

    if let Some(icon) = load_app_icon() {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }
    if let Some([x, y]) = loaded.saved_pos {
        if loaded.dock_anchor == settings::DockAnchor::Free {
            viewport = viewport.with_position(egui::pos2(x, y));
        }
    }

    let options = eframe::NativeOptions {
        viewport,
        depth_buffer: 0,
        stencil_buffer: 0,
        multisampling: 0,
        ..Default::default()
    };

    if let Err(e) = eframe::run_native(
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
            #[cfg(target_os = "macos")]
            set_mac_accessory_policy();
            client.bind_egui_ctx(&cc.egui_ctx);

            Ok(Box::new(app::ZenhushUiApp::new(client, loaded)))
        }),
    ) {
        eprintln!("zenhush ui run_native error: {:?}", e);
    }
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
    if cli.install_gnome_pill {
        match gnome_ext::install_gnome_pill() {
            Ok(msg) => println!("{}", msg),
            Err(e) => {
                eprintln!("GNOME Top-Bar Popover install failed: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }
    if cli.uninstall_gnome_pill {
        match gnome_ext::uninstall_gnome_pill() {
            Ok(msg) => println!("{}", msg),
            Err(e) => {
                eprintln!("GNOME Top-Bar Popover uninstall failed: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }

    if cli.ui_mode {
        run_ui_window();
        return;
    }

    if cli.daemon_mode || cli.start_background {
        if ipc::try_send_to_existing(ipc::RemoteCmd::Show) {
            if cli.start_background {
                return;
            }
        }
        daemon::run_daemon(false);
        return;
    }

    // If another instance is already running, forward CLI command (or Toggle) and exit
    if let Some(cmd) = cli.command {
        if ipc::try_send_to_existing(cmd) {
            return;
        }
        if matches!(cmd, ipc::RemoteCmd::Quit) {
            return;
        }
    } else if ipc::try_send_to_existing(ipc::RemoteCmd::Toggle) {
        return;
    }

    // Fresh launch: install GNOME extension if on GNOME, start background core,
    // and open either the native GNOME Top-Bar Popover or the lightweight fallback card.
    if shortcuts::is_gnome() {
        gnome_ext::ensure_installed_and_enabled();
    }
    let loaded = settings::load();
    if loaded.start_in_background {
        daemon::run_daemon(false);
    } else if spawn_background_daemon() {
        if gnome_ext::is_active() {
            std::thread::sleep(Duration::from_millis(60));
            let _ = ipc::try_send_to_existing(ipc::RemoteCmd::Show);
        } else {
            run_ui_window();
        }
    } else {
        daemon::run_daemon(true);
    }
}
