//! MVault — Memoro Vault as a native app, written entirely in Rust.

mod app;
mod fonts;
#[cfg(target_os = "macos")]
mod macos;
mod pages;
mod prefs;
mod qrscan;
mod snap;
mod ui;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    // what releases up to 1.0.8 may have left on disk goes first
    prefs::scrub_legacy_web_storage();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let snap = arg("--snap").map(|s| snap::Snap::new(s, arg("--out").unwrap_or_else(|| ".".into()).into()));

    let icon = image::load_from_memory(include_bytes!("../assets/icon-256.png"))
        .ok()
        .map(|i| {
            let i = i.to_rgba8();
            egui::IconData { width: i.width(), height: i.height(), rgba: i.into_raw() }
        });
    // first run: the licence window (680x532, fixed) comes first and becomes the app on Accept
    let gate = match &snap {
        Some(s) => s.scenario.starts_with("license"),
        None => !prefs::license_accepted(),
    };
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size(if gate { [680.0, 532.0] } else { [1000.0, 672.0] })
        .with_resizable(!gate)
        .with_title(if gate { app::LICENSE_TITLE } else { "Memoro Vault" });
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("Memoro Vault", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, snap, gate)))))
}
