//! Pieces several pages share: tip lists, button rows, dialog sizing.

use egui::Color32;

use crate::ui::{Btn, CH, LINE, Pg, Span, Ws, WrapOpts, col, s, wrap};

/// A run of words, buttons laid out on one line (wrapping to the next when needed), `gap` apart.
/// Returns (positions, total height).
pub fn row_positions(pg: &Pg<'_>, labels: &[&str], x0: f32, max_w: f32, gap: f32) -> (Vec<(f32, f32)>, f32) {
    let mut out = Vec::new();
    let (mut x, mut y) = (x0, 0.0);
    for l in labels {
        let w = pg.measure(&format!("[{l}]"), 16.0);
        if x > x0 && x + w > x0 + max_w + 0.01 {
            x = x0;
            y += LINE;
        }
        out.push((x, y));
        x += w + gap;
    }
    (out, y + LINE)
}

/// Draws a row of buttons; returns the index clicked, if any, and the height.
pub fn button_row(pg: &mut Pg<'_>, key: &str, x: f32, y: f32, max_w: f32, btns: &[Btn<'_>]) -> (Option<usize>, f32) {
    let labels: Vec<&str> = btns.iter().map(|b| b.label).collect();
    let (pos, h) = row_positions(pg, &labels, x, max_w, 2.0 * CH);
    let mut clicked = None;
    for (i, (b, (bx, by))) in btns.iter().zip(pos).enumerate() {
        if pg.button((key, i), bx, y + by, *b).clicked() {
            clicked = Some(i);
        }
    }
    (clicked, h)
}

/// Width of the longest unwrapped line of each span list (a shrink-to-fit dialog's width).
pub fn max_content(pg: &Pg<'_>, blocks: &[Vec<Span>]) -> f32 {
    let ctx = pg.ctx();
    blocks.iter().map(|b| {
        wrap(&ctx, b, f32::INFINITY, Ws::Normal, WrapOpts::default()).iter().map(|l| l.width).fold(0.0, f32::max)
    }).fold(0.0, f32::max)
}

/// A "• tip" block: bullet 2px in from the left, text indented 16px. Returns the height.
pub fn tip(pg: &mut Pg<'_>, x: f32, y: f32, w: f32, spans: &[Span], bullet: Color32) -> f32 {
    pg.text(x + 2.0, y, "•", bullet);
    pg.para(x + 16.0, y, w - 16.0, spans, Ws::Normal, WrapOpts::default(), LINE)
}

/// A list item with an outside disc marker ("• " ending where the text starts).
pub fn li(pg: &mut Pg<'_>, x: f32, y: f32, w: f32, spans: &[Span], marker: Color32) -> f32 {
    let mw = pg.measure("• ", 16.0);
    pg.text(x - mw, y, "• ", marker);
    pg.para(x, y, w, spans, Ws::Normal, WrapOpts::default(), LINE)
}

/// A decimal list item ("1. " ending where the text starts).
pub fn li_num(pg: &mut Pg<'_>, x: f32, y: f32, w: f32, n: usize, spans: &[Span], marker: Color32) -> f32 {
    let m = format!("{n}. ");
    let mw = pg.measure(&m, 16.0);
    pg.text(x - mw, y, &m, marker);
    pg.para(x, y, w, spans, Ws::Normal, WrapOpts::default(), LINE)
}

/// Plain paragraph helper.
pub fn p(pg: &mut Pg<'_>, x: f32, y: f32, w: f32, text: &str, c: Color32) -> f32 {
    pg.para(x, y, w, &[s(text, c)], Ws::Normal, WrapOpts::default(), LINE)
}

pub fn code(text: &str) -> Span {
    s(text, col::T1).code(col::INPUT, 5.0, 1.0)
}

/// Text cut to fit `max_w`, ending in "…" (text-overflow: ellipsis).
pub fn ellipsize(pg: &Pg<'_>, text: &str, max_w: f32) -> String {
    let ell = pg.measure("…", 16.0);
    let mut out = String::new();
    let mut w = 0.0;
    for c in text.chars() {
        let cw = pg.measure(&c.to_string(), 16.0);
        if w + cw + ell > max_w {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

/// The native save dialog (default folder: Downloads), then an atomic write: next to the target,
/// synced, size-checked, renamed. Ok(file name) once on disk; Err("") when cancelled.
pub fn save_bytes(name: &str, bytes: &[u8], filter: &str, ext: &str) -> Result<String, String> {
    let mut dlg = rfd::FileDialog::new().set_file_name(name).add_filter(filter, &[ext]);
    if let Some(home) = std::env::var_os("HOME") {
        dlg = dlg.set_directory(std::path::Path::new(&home).join("Downloads"));
    }
    let Some(path) = dlg.save_file() else { return Err(String::new()) };
    write_atomic(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())
}

/// The save dialog with no type filter (a file of any kind, named as in the vault).
pub fn save_bytes_any(name: &str, bytes: &[u8]) -> Result<String, String> {
    let mut dlg = rfd::FileDialog::new().set_file_name(name);
    if let Some(home) = std::env::var_os("HOME") {
        dlg = dlg.set_directory(std::path::Path::new(&home).join("Downloads"));
    }
    let Some(path) = dlg.save_file() else { return Err(String::new()) };
    write_atomic(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())
}

pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().unwrap_or(std::path::Path::new("."));
    let tmp = dir.join(format!(".{}.{}.part", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), mvault_core::crypto::random_hex(6)));
    let res = (|| {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        if std::fs::metadata(&tmp)?.len() != bytes.len() as u64 {
            return Err(std::io::Error::other("The file on disk has the wrong size."));
        }
        std::fs::rename(&tmp, path)
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

/// Whether dates and times read the US way: Chromium follows the first preferred language and
/// falls back to en-GB for English regions it has no strings for.
pub fn us_locale() -> bool {
    static US: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *US.get_or_init(|| {
        std::process::Command::new("defaults").args(["read", "-g", "AppleLanguages"]).output().ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|t| t.split('"').nth(1).map(|l| l == "en-US" || l == "en"))
            .unwrap_or(false)
    })
}

/// `new Date().toLocaleTimeString()`
pub fn clock_now() -> String {
    let now = chrono::Local::now();
    if us_locale() { now.format("%-I:%M:%S %p").to_string() } else { now.format("%H:%M:%S").to_string() }
}
