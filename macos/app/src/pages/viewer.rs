//! file-viewer.js: images (fit / zoom), PDFs (rendered in pure Rust by hayro), text (editable when
//! it is plain UTF-8), everything from memory. Esc or [Close] closes it.

use std::sync::Arc;

use egui::{Color32, ColorImage, TextureHandle, TextureOptions};

use crate::app::{Shared, layer_over};
use crate::ui::{Btn, CH, InputOpts, LINE, Pg, Ws, WrapOpts, col, s};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Image,
    Pdf,
    Text,
    Audio,
    Video,
    Other,
}

pub fn sniff(b: &[u8]) -> &'static str {
    let st = |sig: &[u8]| b.len() >= sig.len() && &b[..sig.len()] == sig;
    if st(&[0x89, 0x50, 0x4e, 0x47]) { "image/png" }
    else if st(&[0xff, 0xd8, 0xff]) { "image/jpeg" }
    else if st(b"GIF8") { "image/gif" }
    else if st(b"RIFF") && b.len() > 11 && &b[8..12] == b"WEBP" { "image/webp" }
    else if st(b"BM") { "image/bmp" }
    else if st(&[0, 0, 1, 0]) { "image/x-icon" }
    else if st(b"%PDF-") { "application/pdf" }
    else { "" }
}

fn ext_in(name: &str, list: &[&str]) -> bool {
    let n = name.to_lowercase();
    list.iter().any(|e| n.ends_with(&format!(".{e}")))
}

/// file-viewer.js `fileKind`.
pub fn kind(name: &str, mime: &str, bytes: &[u8]) -> Kind {
    let m = mime.to_lowercase();
    let sn = sniff(bytes);
    if sn == "application/pdf" || m == "application/pdf" || ext_in(name, &["pdf"]) { return Kind::Pdf; }
    if sn.starts_with("image/") || ext_in(name, &["svg"]) || m == "image/svg+xml" { return Kind::Image; }
    if m.starts_with("image/") || ext_in(name, &["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "avif", "svg"]) { return Kind::Image; }
    if m.starts_with("audio/") || ext_in(name, &["mp3", "wav", "ogg", "oga", "m4a", "aac", "flac", "opus", "mka", "aif", "aiff", "caf"]) { return Kind::Audio; }
    if m.starts_with("video/") || ext_in(name, &["mp4", "m4v", "webm", "ogv", "mov", "mkv"]) { return Kind::Video; }
    if m.starts_with("text/") || ["json", "xml", "csv", "yaml", "javascript"].iter().any(|k| m.contains(k))
        || ext_in(name, &["txt", "md", "markdown", "csv", "tsv", "json", "xml", "yaml", "yml", "log", "ini", "cfg", "conf", "toml", "asc", "pem", "pub", "eml", "tex", "js", "ts", "py", "rb", "go", "rs", "java", "c", "h", "cpp", "sh", "bat", "ps1", "sql", "html", "htm", "css", "svg"]) {
        return Kind::Text;
    }
    Kind::Other
}

fn kind_label(k: Kind) -> &'static str {
    match k { Kind::Image => "Image", Kind::Pdf => "PDF", Kind::Text => "Text", Kind::Audio => "Audio", Kind::Video => "Video", Kind::Other => "File" }
}

struct Decoded {
    text: String,
    editable: bool,
    bom: bool,
    eol: &'static str,
}

fn decode_text(b: &[u8]) -> Decoded {
    if b.starts_with(&[0xff, 0xfe]) || b.starts_with(&[0xfe, 0xff]) {
        let le = b[0] == 0xff;
        let units: Vec<u16> = b[2..].chunks(2).filter(|c| c.len() == 2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
        return Decoded { text: String::from_utf16_lossy(&units), editable: false, bom: false, eol: "\n" };
    }
    let bom = b.starts_with(&[0xef, 0xbb, 0xbf]);
    let rest = if bom { &b[3..] } else { b };
    match std::str::from_utf8(rest) {
        Ok(t) => {
            let eol = if t.contains("\r\n") { "\r\n" } else if t.contains('\r') { "\r" } else { "\n" };
            Decoded { text: t.to_string(), editable: true, bom, eol }
        }
        Err(_) => Decoded { text: String::from_utf8_lossy(rest).to_string(), editable: false, bom, eol: "\n" },
    }
}

pub struct Viewer {
    pub name: String,
    pub mime: String,
    pub bytes: Arc<Vec<u8>>,
    kind: Kind,
    /// "Text · 118 B": written once when the viewer opens
    meta: String,
    image: Option<Result<(TextureHandle, [usize; 2], bool), String>>,
    /// 0 = fit to the window
    zoom: f32,
    pdf: Option<Result<Vec<(TextureHandle, [usize; 2])>, String>>,
    pdf_scale: f32,
    text: Option<Decoded>,
    editing: Option<String>,
    focus_edit: bool,
    /// text edits go back to the editor through this
    pub saved: Option<Vec<u8>>,
    can_edit: bool,
    /// `body.focus()` scrolls the body into view once it is laid out
    focus_scroll: bool,
    /// the body's width last frame (`body.clientWidth`)
    body_w: f32,
    /// audio and video: an AVPlayer reading the bytes from memory
    media: Option<Result<crate::media::AnyPlayer, String>>,
    media_tex: Option<TextureHandle>,
    /// snapshots: start playing as soon as it can
    pub snap_play: bool,
}

const MAX_TEXT_PREVIEW: usize = 2 * 1024 * 1024;

impl Viewer {
    /// Snapshots: [Edit text] already pressed.
    pub fn snap_edit(&mut self) {
        let cut = self.bytes.len().min(MAX_TEXT_PREVIEW);
        let t = decode_text(&self.bytes[..cut]);
        self.editing = Some(t.text.replace("\r\n", "\n").replace('\r', "\n"));
        self.text = Some(t);
        self.focus_edit = true;
    }
}

impl Viewer {
    pub fn new(name: &str, mime: &str, bytes: Arc<Vec<u8>>, editable: Option<bool>) -> Viewer {
        let k = kind(name, mime, &bytes);
        let meta = format!("{} · {}", kind_label(k), mvault_core::format_bytes(bytes.len() as u64));
        Viewer {
            name: name.into(), mime: mime.into(), kind: k, meta, bytes, image: None, zoom: 0.0, pdf: None, pdf_scale: 1.25, text: None,
            editing: None, focus_edit: false, saved: None, can_edit: editable.unwrap_or(false), focus_scroll: true, body_w: 0.0,
            media: None,
            media_tex: None,
            snap_play: false,
        }
    }
}

fn load_image(ctx: &egui::Context, name: &str, bytes: &[u8]) -> Result<(TextureHandle, [usize; 2], bool), String> {
    let img = image::load_from_memory(bytes).map_err(|_| "This image could not be decoded. You can still download it.".to_string())?;
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let see_through = rgba.pixels().any(|p| p[3] < 255);
    let ci = ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    Ok((ctx.load_texture(format!("view-{name}"), ci, TextureOptions::LINEAR), size, see_through))
}

fn load_pdf(ctx: &egui::Context, bytes: &[u8], scale: f32) -> Result<Vec<(TextureHandle, [usize; 2])>, String> {
    let pdf = hayro::hayro_syntax::Pdf::new(bytes.to_vec()).map_err(|_| "This file could not be displayed (Invalid PDF). You can still download it.".to_string())?;
    let px = ctx.pixels_per_point();
    let pages = hayro::render_pdf(&pdf, scale * px, hayro::hayro_interpret::InterpreterSettings::default(), &hayro::RenderSettings::default(), None).unwrap_or_default();
    Ok(pages.into_iter().enumerate().map(|(i, p)| {
        let (w, h) = (p.width() as usize, p.height() as usize);
        let mut buf = Vec::with_capacity(w * h * 4);
        for c in p.data() {
            buf.extend_from_slice(&[c.r, c.g, c.b, c.a]);
        }
        let ci = ColorImage::from_rgba_premultiplied([w, h], &buf);
        (ctx.load_texture(format!("pdf-{i}"), ci, TextureOptions::LINEAR), [w, h])
    }).collect())
}

/// A head control: a bracketed button, or a zoom label (6px padding each side).
enum Tool {
    B(&'static str, &'static str, Color32),
    L(String),
}

fn tool_w(pg: &Pg<'_>, t: &Tool) -> f32 {
    match t {
        Tool::B(l, ..) => pg.measure(&format!("[{l}]"), 16.0),
        Tool::L(l) => (pg.measure(l, 16.0) + 12.0).max(44.0),
    }
}

/// Lays out `tools` from `x`, 2px apart; returns the clicked key.
fn draw_tools(pg: &mut Pg<'_>, x: f32, y: f32, tools: &[Tool]) -> Option<&'static str> {
    let mut hit = None;
    let mut tx = x;
    for t in tools {
        let w = tool_w(pg, t);
        match t {
            Tool::B(label, key, c) => {
                if pg.button(*key, tx, y, Btn::new(label).color(*c)).clicked() {
                    hit = Some(*key);
                }
            }
            Tool::L(l) => {
                let lw = pg.measure(l, 16.0);
                pg.text(tx + (w - lw) / 2.0, y, l, col::T2);
            }
        }
        tx += w + 2.0;
    }
    hit
}

const BODY_BG: Color32 = Color32::from_rgb(0x12, 0x12, 0x12);

/// Draws the viewer; returns true once it is closed. `under` is the page whose scrollbar stays
/// beside it (none while the editor is open: the editor hides the page's).
pub fn show(v: &mut Viewer, sh: &mut Shared, ui: &mut egui::Ui, under: Option<&str>) -> bool {
    let ctx = ui.ctx().clone();
    // Esc closes the viewer, an unfinished edit included (file-viewer.js onKey)
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        return true;
    }
    match v.kind {
        Kind::Image if v.image.is_none() => v.image = Some(load_image(&ctx, &v.name, &v.bytes)),
        Kind::Pdf if v.pdf.is_none() => v.pdf = Some(load_pdf(&ctx, &v.bytes, v.pdf_scale)),
        Kind::Text if v.text.is_none() => {
            let cut = v.bytes.len().min(MAX_TEXT_PREVIEW);
            v.text = Some(decode_text(&v.bytes[..cut]));
        }
        Kind::Audio | Kind::Video if v.media.is_none() => {
            v.media = Some(crate::media::AnyPlayer::open(v.bytes.clone(), &v.name, &v.mime, &ctx));
        }
        _ => {}
    }
    if let Some(Ok(p)) = &mut v.media {
        p.poll();
        if let Some(f) = p.frame() {
            let img = ColorImage::from_rgba_unmultiplied([f.w, f.h], &f.rgba);
            match &mut v.media_tex {
                Some(t) => t.set(img, TextureOptions::LINEAR),
                None => v.media_tex = Some(ctx.load_texture("viewer-media", img, TextureOptions::LINEAR)),
            }
        }
        if v.snap_play && matches!(p.state(), crate::media::State::Ready) {
            v.snap_play = false;
            p.play();
        }
        let wait = match p.state() {
            crate::media::State::Loading => Some(100),
            _ if p.playing() => Some(16),
            _ => None,
        };
        if let Some(ms) = wait {
            ctx.request_repaint_after(std::time::Duration::from_millis(ms));
        }
    }
    let mut media_act: Option<&'static str> = None;
    let mut seek_to: Option<f64> = None;
    let screen = ctx.content_rect();
    let (vw, vh) = (screen.width(), screen.height());
    let truncated = v.bytes.len() > MAX_TEXT_PREVIEW;
    let mut act: Option<&'static str> = None;
    let mut want_scroll: Option<f32> = None;
    let mut body_w_now = v.body_w;
    // the controls in the head
    let mut tools: Vec<Tool> = Vec::new();
    match v.kind {
        Kind::Image => {
            if let Some(Ok(_)) = &v.image {
                tools.push(Tool::B("−", "out", col::T2));
                tools.push(Tool::L(String::new())); // filled in once the size is known
                tools.push(Tool::B("+", "in", col::T2));
                tools.push(Tool::B("Fit", "fit", col::T2));
                tools.push(Tool::B("100%", "one", col::T2));
            }
        }
        Kind::Pdf => {
            if let Some(Ok(pages)) = &v.pdf {
                let n = pages.len();
                tools.push(Tool::B("−", "pout", col::T2));
                tools.push(Tool::L(format!("{n} page{} · {}%", if n == 1 { "" } else { "s" }, (v.pdf_scale * 100.0).round())));
                tools.push(Tool::B("+", "pin", col::T2));
            }
        }
        Kind::Text if v.can_edit && !truncated && v.text.as_ref().is_some_and(|t| t.editable) => {
            if v.editing.is_some() {
                tools.push(Tool::B("Keep changes", "keep", col::WHITE));
                tools.push(Tool::B("Cancel", "cancel", col::T2));
            } else {
                tools.push(Tool::B("Edit text", "edit", col::T2));
            }
        }
        _ => {}
    }
    layer_over(ui, sh, "viewer", under, true, |pg, width| {
        // .mv-viewer: 8px in; the panel: 1px border, as wide as its head or its body needs
        let avail = width - 16.0 - 2.0;
        let x0 = 9.0;
        // body content: what it needs (max-content) and how it is drawn
        let fit_scale = |bw: f32, size: [usize; 2]| -> f32 {
            (1.0f32).min((bw - 32.0) / size[0] as f32).min((vh - 140.0) / size[1] as f32).max(0.0)
        };
        // every page slot takes the first page's size at this scale (floored, in CSS pixels)
        let pdf_base = |pages: &Vec<(TextureHandle, [usize; 2])>, ppp: f32| -> (f32, f32) {
            let s0 = pages.first().map(|p| p.1).unwrap_or([0, 0]);
            ((s0[0] as f32 / ppp).floor(), (s0[1] as f32 / ppp).floor())
        };
        let ppp = pg.ui.ctx().pixels_per_point();
        let text_w = |pg: &Pg<'_>, t: &str| t.split('\n').map(|l| pg.measure(l.trim_end_matches('\r'), 16.0)).fold(0.0, f32::max);
        let empty_msg = |v: &Viewer| -> Option<String> {
            match v.kind {
                Kind::Image => v.image.as_ref().and_then(|r| r.as_ref().err().cloned()),
                Kind::Pdf => v.pdf.as_ref().and_then(|r| r.as_ref().err().cloned()),
                Kind::Audio | Kind::Video => match &v.media {
                    Some(Err(e)) => Some(format!("This file could not be played ({e}). You can still download it.")),
                    Some(Ok(p)) => match p.state() {
                        crate::media::State::Failed(e) => Some(format!("This file could not be played ({e}). You can still download it.")),
                        _ => None,
                    },
                    None => None,
                },
                Kind::Other => Some("Memoro Vault cannot display this file type. You can still download it.".into()),
                Kind::Text => None,
            }
        };
        let need = 32.0 + match v.kind {
            _ if empty_msg(v).is_some() => pg.measure(&empty_msg(v).unwrap(), 16.0),
            Kind::Image => match &v.image {
                Some(Ok((_, size, _))) => if v.zoom == 0.0 { size[0] as f32 } else { (size[0] as f32 * v.zoom).round() },
                _ => 0.0,
            },
            Kind::Pdf => match &v.pdf {
                Some(Ok(pages)) => pdf_base(pages, ppp).0,
                _ => 0.0,
            },
            Kind::Audio | Kind::Video => {
                let nat = match &v.media {
                    Some(Ok(p)) => p.video_size().0,
                    _ => 0.0,
                };
                nat.max(MEDIA_BAR_W)
            }
            Kind::Text => {
                if v.editing.is_some() {
                    20.0 * CH + 30.0
                } else {
                    let t = v.text.as_ref().map(|t| t.text.as_str()).unwrap_or("");
                    (text_w(pg, t) + 30.0).min(1000.0)
                }
            }
            _ => 0.0,
        };
        // head: name over kind and size, then [tools][Download][Close] 2px apart in a 4px pad
        let name_w = pg.measure(&v.name, 16.0).min(0.52 * vw);
        let title_w = name_w.max(pg.measure(&v.meta, 16.0));
        let mut tools_now: Vec<Tool> = Vec::new();
        for t in &tools {
            tools_now.push(match t {
                Tool::B(l, k, c) => Tool::B(l, k, *c),
                Tool::L(l) if l.is_empty() => {
                    // the image's zoom label: the shown size of the image
                    let pct = match &v.image {
                        Some(Ok((_, size, _))) => {
                            if v.zoom == 0.0 {
                                let pw = need.max(1.0).min(avail);
                                let s = fit_scale(pw, *size);
                                ((size[0] as f32 * s).round() / size[0] as f32 * 100.0).round()
                            } else {
                                (v.zoom * 100.0).round()
                            }
                        }
                        _ => 100.0,
                    };
                    Tool::L(format!("{pct}%{}", if v.zoom == 0.0 { " (fit)" } else { "" }))
                }
                Tool::L(l) => Tool::L(l.clone()),
            });
        }
        let tools_w = if tools_now.is_empty() { 0.0 } else { tools_now.iter().map(|t| tool_w(pg, t)).sum::<f32>() + 2.0 * (tools_now.len() - 1) as f32 };
        let actions = [Tool::B("Download", "download", col::T2), Tool::B("Close", "close", col::T2)];
        let acts_w = 4.0 + tools_w + 2.0 + actions.iter().map(|t| tool_w(pg, t)).sum::<f32>() + 2.0 + 4.0;
        let head_w = 14.0 + title_w + 12.0 + acts_w + 14.0;
        let panel_w = head_w.max(need).min(avail);
        body_w_now = panel_w;
        // the head (actions wrap under the title when they do not fit)
        let (hx, hy) = (x0 + 14.0, x0 + 10.0);
        let name = if pg.measure(&v.name, 16.0) > name_w { crate::pages::common::ellipsize(pg, &v.name, name_w) } else { v.name.clone() };
        pg.text(hx, hy, &name, col::T1);
        pg.text(hx, hy + LINE, &v.meta, col::T2);
        let (ax, ay, head_h) = if head_w <= avail { (hx + title_w + 12.0, hy + 5.5, 2.0 * LINE) } else { (hx, hy + 2.0 * LINE + 12.0, 2.0 * LINE + 12.0 + 27.0) };
        if let Some(k) = draw_tools(pg, ax + 4.0, ay + 4.0, &tools_now) {
            act = Some(k);
        }
        let dx = ax + 4.0 + if tools_now.is_empty() { 0.0 } else { tools_w + 2.0 } + if tools_now.is_empty() { 2.0 } else { 0.0 };
        if let Some(k) = draw_tools(pg, dx, ay + 4.0, &actions) {
            act = Some(k);
        }
        // the body: 16px padding on a slightly raised grey
        let by = hy + head_h + 10.0 + 1.0;
        let (cx, cy, cw) = (x0 + 16.0, by + 16.0, panel_w - 32.0);
        // the body's grey goes in first, sized once the content is known
        let bg_idx = pg.ui.painter().add(egui::Shape::Noop);
        let content_h: f32 = if let Some(msg) = empty_msg(v) {
            // .mv-viewer-empty: centred, its p keeps 1em margins
            let mw = pg.measure(&msg, 16.0).min(cw).max(1.0);
            16.0 + pg.para(cx + (cw - mw) / 2.0, cy + 16.0, mw, &[s(&msg, col::T2)], Ws::Normal, WrapOpts::default(), LINE) + 16.0
        } else {
            match v.kind {
                Kind::Image => match &v.image {
                    Some(Ok((tex, size, see_through))) => {
                        let (nw, nh) = (size[0] as f32, size[1] as f32);
                        let (iw, ih) = if v.zoom == 0.0 {
                            let z = fit_scale(panel_w, *size);
                            (nw * z, nh * z)
                        } else {
                            let w = (nw * v.zoom).round();
                            (w, w * nh / nw)
                        };
                        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                        if iw <= cw + 0.5 {
                            let ix = cx + (cw - iw) / 2.0;
                            let r = pg.rect(ix, cy, iw, ih);
                            if *see_through {
                                checker(pg.ui.painter(), r);
                            }
                            pg.ui.painter().image(tex.id(), r, uv, Color32::WHITE);
                            ih
                        } else {
                            // zoomed past the window: the body scrolls sideways (6px bar under it)
                            let id = pg.wid("img-scroll");
                            let r = pg.rect(x0, by, panel_w, 16.0 + ih + 16.0 + 6.0);
                            let mut child = pg.ui.new_child(egui::UiBuilder::new().max_rect(r).id_salt(id));
                            egui::ScrollArea::horizontal().id_salt(id).auto_shrink([false, false]).show(&mut child, |ui| {
                                let (rr, _) = ui.allocate_exact_size(egui::vec2(16.0 + iw + 16.0, 16.0 + ih + 16.0), egui::Sense::hover());
                                let ir = egui::Rect::from_min_size(rr.min + egui::vec2(16.0, 16.0), egui::vec2(iw, ih));
                                if *see_through {
                                    checker(ui.painter(), ir);
                                }
                                ui.painter().image(tex.id(), ir, uv, Color32::WHITE);
                            });
                            ih + 6.0
                        }
                    }
                    _ => 0.0,
                },
                Kind::Pdf => match &v.pdf {
                    Some(Ok(pages)) => {
                        let (sw, sh_min) = pdf_base(pages, ppp);
                        let px0 = cx + ((cw - sw) / 2.0).max(0.0);
                        let mut y = cy;
                        for (i, (tex, size)) in pages.iter().enumerate() {
                            let (w, h) = ((size[0] as f32 / ppp).floor(), (size[1] as f32 / ppp).floor());
                            let slot_h = sh_min.max(h);
                            pg.fill(px0, y, sw, slot_h, Color32::WHITE);
                            // the page number sits under the drawn page, so it only shows until then
                            let _ = i;
                            let r = pg.rect(px0, y, w, h);
                            pg.ui.painter().image(tex.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                            y += slot_h + 14.0;
                        }
                        (y - 14.0 - cy).max(0.0)
                    }
                    _ => 0.0,
                },
                Kind::Text => {
                    if let Some(ed) = &mut v.editing {
                        let h = (0.6 * vh).max(2.0 * LINE);
                        let id = pg.wid("edit");
                        let focused = pg.ui.memory(|m| m.has_focus(id));
                        pg.fill(cx, cy, cw, h, if focused || v.focus_edit { col::INPUT_FOCUS } else { col::INPUT });
                        // a console field: 1ch either side, no border (pgpm.css), at least 60vh tall
                        let rows = ((h / LINE).floor() as usize).max(1);
                        let out = pg.input("edit", cx, cy, cw, ed, "", InputOpts::area(rows));
                        if std::mem::take(&mut v.focus_edit) {
                            out.response.request_focus();
                        }
                        h
                    } else if let Some(t) = &v.text {
                        let lines = pg.para_h(cw - 30.0, &[s(&t.text, col::T1)], Ws::PreWrap, WrapOpts { anywhere: true }).max(1);
                        let h = lines as f32 * LINE + 30.0;
                        pg.fill(cx, cy, cw, h, col::BG);
                        pg.para(cx + 15.0, cy + 15.0, cw - 30.0, &[s(&t.text, col::T1)], Ws::PreWrap, WrapOpts { anywhere: true }, LINE);
                        let mut total = h;
                        if truncated {
                            total += 6.0 + pg.para(cx, cy + h + 6.0, cw, &[s(format!("Showing the first {}. Download the file to see all of it.", mvault_core::format_bytes(MAX_TEXT_PREVIEW as u64)), col::T2)], Ws::Normal, WrapOpts::default(), LINE) + 6.0;
                        }
                        total
                    } else {
                        0.0
                    }
                }
                Kind::Audio | Kind::Video => match &v.media {
                    Some(Ok(p)) => media_body(pg, p, v.media_tex.as_ref(), (cx, cy, cw), vh, &mut media_act, &mut seek_to),
                    _ => 0.0,
                },
                _ => 0.0,
            }
        };
        let body_h = content_h + 32.0;
        let bg = pg.rect(x0, by, panel_w, body_h);
        pg.ui.painter().set(bg_idx, egui::Shape::rect_filled(bg, 0.0, BODY_BG));
        // body.focus(): scrolled into view (its top, when it is taller than the window)
        if v.focus_scroll && content_h > 0.0 {
            v.focus_scroll = false;
            let bottom = by + body_h;
            if bottom > vh {
                want_scroll = Some(if body_h > vh { by } else { bottom - vh });
            }
        }
        by + body_h + 1.0 + 8.0
    });
    v.body_w = body_w_now;
    if let Some(Ok(p)) = &mut v.media {
        match media_act {
            Some("mtoggle") => {
                if p.playing() { p.pause() } else { p.play() }
            }
            Some("mmute") => p.set_muted(!p.muted()),
            _ => {}
        }
        if let Some(t) = seek_to {
            p.seek(t);
        }
    }
    let _ = (&mut media_act, &mut seek_to);
    if let Some(y) = want_scroll {
        crate::app::request_scroll(&ctx, "viewer", y);
    }
    let mut close = false;
    match act {
        Some("close") => close = true,
        Some("download") => {
            let _ = crate::pages::common::save_bytes_any(&v.name, &v.bytes);
        }
        Some("out") | Some("in") => {
            if let Some(Ok((_, size, _))) = &v.image {
                let base = if v.zoom == 0.0 { (1.0f32).min((v.body_w - 24.0) / size[0] as f32) } else { v.zoom };
                v.zoom = (base * if act == Some("in") { 1.25 } else { 0.8 }).clamp(0.1, 8.0);
            }
        }
        Some("fit") => v.zoom = 0.0,
        Some("one") => v.zoom = 1.0,
        Some("pout") | Some("pin") => {
            v.pdf_scale = (v.pdf_scale * if act == Some("pin") { 1.25 } else { 0.8 }).clamp(0.5, 4.0);
            v.pdf = None;
        }
        Some("edit") => {
            v.editing = v.text.as_ref().map(|t| t.text.replace("\r\n", "\n").replace('\r', "\n"));
            v.focus_edit = true;
        }
        Some("cancel") => v.editing = None,
        Some("keep") => {
            if let (Some(ed), Some(t)) = (v.editing.take(), &mut v.text) {
                if ed != t.text.replace("\r\n", "\n").replace('\r', "\n") {
                    let text = if t.eol == "\n" { ed } else { ed.replace('\n', t.eol) };
                    let mut out = Vec::new();
                    if t.bom {
                        out.extend_from_slice(&[0xef, 0xbb, 0xbf]);
                    }
                    out.extend_from_slice(text.as_bytes());
                    t.text = text;
                    v.bytes = Arc::new(out.clone());
                    v.saved = Some(out);
                }
            }
        }
        _ => {}
    }
    close
}

/// The transparent-image backdrop: #8883 squares, 10px, in a 20px checker.
fn checker(p: &egui::Painter, r: egui::Rect) {
    let c = Color32::from_rgba_unmultiplied(0x88, 0x88, 0x88, 0x33);
    let (nx, ny) = ((r.width() / 10.0).ceil() as usize, (r.height() / 10.0).ceil() as usize);
    for j in 0..ny {
        for i in 0..nx {
            if (i + j) % 2 == 1 {
                let min = r.min + egui::vec2(i as f32 * 10.0, j as f32 * 10.0);
                let cell = egui::Rect::from_min_size(min, egui::vec2(10.0, 10.0)).intersect(r);
                p.rect_filled(cell, 0.0, c);
            }
        }
    }
}

/// The narrowest the media controls get (a little wider than Chromium's 300px audio bar, for the
/// seek bar between the time and [Mute]).
const MEDIA_BAR_W: f32 = 480.0;

/// `m:ss`, or `h:mm:ss` past an hour (Chromium's media clock).
fn clock(t: f64) -> String {
    if !t.is_finite() || t < 0.0 {
        return "0:00".into();
    }
    let s = t.floor() as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// The picture (video) fitted like `.mv-media`, then a console control line:
/// [Play] 0:05 / 1:30 ────────── [Mute]. Returns the height used.
#[allow(clippy::too_many_arguments)]
fn media_body(pg: &mut Pg<'_>, p: &crate::media::AnyPlayer, tex: Option<&TextureHandle>, (cx, cy, cw): (f32, f32, f32), vh: f32, act: &mut Option<&'static str>, seek_to: &mut Option<f64>) -> f32 {
    let mut y = cy;
    // a picture when there is one (a .webm or .ogg may be sound only, whatever its name says)
    let (nw, nh) = p.video_size();
    if nw > 0.0 && nh > 0.0 {
        let (bw, bh) = if nw > 0.0 && nh > 0.0 {
            let s = 1.0f32.min(cw / nw).min((vh - 140.0).max(60.0) / nh);
            (nw * s, nh * s)
        } else {
            (300.0f32.min(cw), 150.0)
        };
        let bx = cx + (cw - bw) / 2.0;
        pg.fill(bx, y, bw, bh, Color32::BLACK);
        if let Some(t) = tex {
            let r = pg.rect(bx, y, bw, bh);
            pg.ui.painter().image(t.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        }
        // a click on the picture plays or pauses it, as on a <video>
        let r = pg.rect(bx, y, bw, bh);
        if pg.ui.interact(r, pg.wid("media-picture"), egui::Sense::click()).clicked() {
            *act = Some("mtoggle");
        }
        y += bh + 8.0;
    }
    let ready = matches!(p.state(), crate::media::State::Ready);
    let label = if p.playing() { "Pause" } else { "Play" };
    if pg.button("media-play", cx, y, Btn::new(label).color(col::T1).enabled(ready)).clicked() {
        *act = Some("mtoggle");
    }
    let mut x = cx + pg.measure(&format!("[{label}]"), 16.0).max(pg.measure("[Pause]", 16.0)) + 2.0 * CH;
    let (t, d) = (p.time(), p.duration());
    let times = format!("{} / {}", clock(t), if d.is_finite() { clock(d) } else { "0:00".into() });
    x += pg.text(x, y, &times, col::T2) + 2.0 * CH;
    let mute = if p.muted() { "Unmute" } else { "Mute" };
    let mw = pg.measure(&format!("[{mute}]"), 16.0);
    let mx = cx + cw - mw;
    if pg.button("media-mute", mx, y, Btn::new(mute).color(col::T2).enabled(ready)).clicked() {
        *act = Some("mmute");
    }
    // the seek bar: the progress track drawn like the app's other 4px bars
    let sw = (mx - 2.0 * CH - x).max(40.0);
    pg.fill(x, y + 7.5, sw, 4.0, col::LINE);
    if d.is_finite() && d > 0.0 {
        pg.fill(x, y + 7.5, sw * (t / d).clamp(0.0, 1.0) as f32, 4.0, col::T1);
    }
    let r = pg.rect(x, y, sw, LINE);
    let bar = pg.ui.interact(r, pg.wid("media-seek"), egui::Sense::click_and_drag());
    if bar.hovered() && ready {
        pg.ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if ready && d.is_finite() && d > 0.0 && (bar.clicked() || bar.dragged()) {
        if let Some(pos) = bar.interact_pointer_pos() {
            let f = ((pos.x - r.min.x) / r.width()).clamp(0.0, 1.0) as f64;
            *seek_to = Some(f * d);
        }
    }
    y + LINE - cy
}
