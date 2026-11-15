//! Fonts: Cascadia Mono (the PGPM console font, bundled with its OFL licence) first, then the
//! fallbacks Chromium picks on macOS for glyphs Cascadia lacks (Apple Symbols has ⋮), then
//! whatever system font eframe's provider finds (CJK and the like typed into answers).

use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};

pub const CASCADIA: &[u8] = include_bytes!("../assets/fonts/CascadiaMono-Regular.ttf");

/// Cascadia Mono metrics (units per em 2048).
pub const UPEM: f32 = 2048.0;
pub const ASCENT: f32 = 1900.0;
pub const DESCENT: f32 = 480.0;
pub const ADVANCE: f32 = 1200.0;

pub fn install(ctx: &egui::Context) {
    let mut defs = FontDefinitions::default();
    defs.font_data.insert("cascadia".into(), Arc::new(FontData::from_static(CASCADIA)));
    let mut chain = vec!["cascadia".to_string()];
    for (name, path) in [
        ("apple-symbols", "/System/Library/Fonts/Apple Symbols.ttf"),
        ("menlo", "/System/Library/Fonts/Menlo.ttc"),
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            defs.font_data.insert(name.into(), Arc::new(FontData::from_owned(bytes)));
            chain.push(name.into());
        }
    }
    for fam in [FontFamily::Monospace, FontFamily::Proportional] {
        let list = defs.families.entry(fam).or_default();
        for (i, n) in chain.iter().enumerate() {
            list.insert(i, n.clone());
        }
    }
    ctx.set_fonts(defs);
}
