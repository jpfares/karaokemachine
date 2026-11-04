#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod audio;
mod automap;
mod editor;
mod jobs;
mod library;
mod map_package;
mod model;
mod pitch;
mod theme;
mod tools;
mod widgets;

use app::KaraokeMachineApp;
use eframe::egui;

fn main() -> eframe::Result<()> {
    // Native eframe window at a predictable 1600x980 px. Hardware GL is the
    // default; KARAOKEMACHINE_SOFTWARE_GL=1 forces Mesa's software rasterizer if a
    // driver misbehaves.
    #[cfg(target_os = "linux")]
    {
        std::env::set_var("WINIT_X11_SCALE_FACTOR", "1");
        if std::env::var("KARAOKEMACHINE_SOFTWARE_GL").is_ok() {
            std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");
        }
    }
    // KARAOKEMACHINE_WINDOW=WxH opens at another size, for layout checks
    // at the supported minimum (1100x700).
    let inner_size = std::env::var("KARAOKEMACHINE_WINDOW")
        .ok()
        .and_then(|spec| {
            let (w, h) = spec.split_once('x')?;
            Some([w.trim().parse::<f32>().ok()?, h.trim().parse::<f32>().ok()?])
        })
        .unwrap_or([1600.0, 980.0]);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(inner_size)
            .with_min_inner_size([1100.0, 700.0])
            .with_title("KaraokeMachine")
            .with_app_id("karaokemachine"),
        vsync: true,
        ..Default::default()
    };
    eframe::run_native(
        "KaraokeMachine",
        options,
        Box::new(|cc| Ok(Box::new(KaraokeMachineApp::new(cc)))),
    )
}
