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
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        if bridge::bridge_ready() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(12));
    }
    bridge::bridge_ready()
}

fn run_ui_window() {
    let Some(_ui_lock) = ipc::InstanceLock::try_acquire("zenhush-ui") else {
        let _ = ipc::try_send_to_existing(ipc::RemoteCmd::Show);
        return;
    };

    let client = match bridge::BridgeClient::connect() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("zenhush ui bridge error: {}", e);
            return;
        }
    };

    let loaded = settings::load();
    let has_file = loaded.active_file.is_some()
        || loaded
            .locked_default_file
            .as_ref()
            .map(|p| p.is_file())
            .unwrap_or(false);
    let gnome_hint = shortcuts::is_gnome() && !gnome_ext::is_active();
    let init_w = 336.0_f32;
    let mut init_h = if has_file { 346.0_f32 } else { 296.0_f32 };
    if gnome_hint {
        init_h += 24.0;
    }

    let use_transparent =
        cfg!(not(target_os = "linux")) || std::env::var_os("WAYLAND_DISPLAY").is_some();

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Zenhush")
        .with_app_id("zenhush")
        .with_inner_size([init_w, init_h])
        .with_min_inner_size([init_w, 260.0])
        .with_max_inner_size([init_w, 520.0])
        .with_resizable(true)
        .with_decorations(false)
        .with_always_on_top()
        .with_transparent(use_transparent)
        .with_taskbar(false);

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

    shortcuts::ensure_icon_assets_fresh();

    if cli.ui_mode {
        run_ui_window();
        return;
    }

    if cli.daemon_mode || cli.start_background {
        if ipc::InstanceLock::is_locked("zenhush-daemon") {
            if !cli.start_background {
                let _ = ipc::wait_and_send_to_existing(ipc::RemoteCmd::Show, 4000);
            }
            return;
        }
        daemon::run_daemon(false);
        return;
    }

    // If another instance is already running (or mid-startup), forward command and exit
    if let Some(cmd) = cli.command {
        if ipc::try_send_to_existing(cmd)
            || (ipc::InstanceLock::is_locked("zenhush-daemon")
                && ipc::wait_and_send_to_existing(cmd, 4000))
        {
            return;
        }
        if matches!(cmd, ipc::RemoteCmd::Quit) {
            return;
        }
    } else if ipc::try_send_to_existing(ipc::RemoteCmd::Toggle)
        || (ipc::InstanceLock::is_locked("zenhush-daemon")
            && ipc::wait_and_send_to_existing(ipc::RemoteCmd::Toggle, 4000))
    {
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
