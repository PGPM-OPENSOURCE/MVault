//! vault-editor.js: add, rename, remove and view files, edit seed phrases and the final message,
//! then seal a new vault with the same answers (fresh salts and hidden order), verified before saving.
//!
//! Geometry follows the PGPM theme: the editor, its dialogs and the progress screen each take over
//! the window (opaque, 8px in, 1px transparent border), dialogs shrink to their widest line.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::sync::{Arc, Mutex};

use egui::Color32;
use mvault_core::format_bytes;
use mvault_core::jsstr::js_trim;
use mvault_core::vault::{self, Contents, Seed, Session, Upload, VaultMeta};
use serde_json::json;

use crate::app::{Shared, layer};
use crate::pages::common::{ellipsize, li};
use crate::pages::viewer::{self, Viewer};
use crate::ui::{Btn, CH, InputOpts, LINE, Pg, Span, WrapOpts, Ws, col, s};

const LARGE_FILE: u64 = 250 * 1024 * 1024;
/// A text field's own width (size=20, 1ch padding) inside a dialog that shrinks to fit.
const INPUT_INTRINSIC: f32 = 214.25;

#[derive(Clone)]
struct Item {
    uid: u64,
    name: String,
    mime: String,
    bytes: Arc<Vec<u8>>,
}

#[derive(Clone)]
struct SeedItem {
    uid: u64,
    name: String,
    content: String,
}

#[derive(Clone)]
struct Snapshot {
    uploads: Vec<Item>,
    seeds: Vec<SeedItem>,
    final_message: String,
}

struct NewFile {
    name: String,
    path: PathBuf,
    size: u64,
}

#[derive(Clone, Copy)]
enum Confirm {
    RemoveFile(u64),
    RemoveSeed(u64),
    Discard,
    Large,
}

enum Dialog {
    Rename { uid: u64, value: String, error: String, focus: bool },
    Confirm { what: Confirm, title: String, body: Vec<(String, bool)>, ok: &'static str, danger: bool },
    Seed { uid: Option<u64>, name: String, content: String, error: String, focus: bool },
    Save { paper: bool },
    /// paragraphs: (text, is a muted note)
    Info { title: String, body: Vec<(String, bool)> },
    NotSaved { paper: bool, error: String },
    Paper { picks: Vec<bool>, error: String },
}

enum Msg {
    Progress(String, f32),
    Sealed(Result<vault::Resealed, String>),
    Paper(Result<(Vec<u8>, usize, usize), String>),
}

/// What the editor hands back when it closes.
pub struct Closed {
    pub contents: Contents,
    pub session: Arc<Session>,
    pub revision: i64,
    pub modified: String,
}

pub struct Editor {
    session: Arc<Session>,
    revision: i64,
    last_saved: String,
    uploads: Vec<Item>,
    seeds: Vec<SeedItem>,
    final_message: String,
    baseline: Snapshot,
    shown: HashSet<u64>,
    next_uid: u64,
    dialog: Option<Dialog>,
    viewer: Option<(u64, Viewer)>,
    progress: Option<(String, f32)>,
    job: Option<Receiver<Msg>>,
    // a save in flight
    save_now: String,
    want_paper: bool,
    resealed: Option<vault::Resealed>,
    written: String,
    sealed: Option<vault::Sealed>,
    pdf: Option<(Vec<u8>, usize, usize)>,
    /// files waiting on a dialog ("File too large" / "Large file") before they are added
    pending: Vec<NewFile>,
}

impl Editor {
    pub fn new(session: Arc<Session>, c: &Contents, meta: &VaultMeta) -> Editor {
        let mut uid = 0u64;
        let uploads: Vec<Item> = c.uploads.iter().map(|u| {
            uid += 1;
            Item { uid, name: u.name.clone(), mime: u.mime.clone(), bytes: u.bytes.clone() }
        }).collect();
        let seeds: Vec<SeedItem> = c.seeds.iter().map(|sd| {
            uid += 1;
            SeedItem { uid, name: sd.name.clone(), content: sd.content.clone() }
        }).collect();
        let final_message = c.final_message.clone();
        let baseline = Snapshot { uploads: uploads.clone(), seeds: seeds.clone(), final_message: final_message.clone() };
        Editor {
            revision: meta.revision().filter(|r| *r > 0).unwrap_or(1),
            last_saved: meta.modified().filter(|m| !m.is_empty()).or_else(|| session.vault_json.created()).unwrap_or_default(),
            session, uploads, seeds, final_message, baseline, shown: HashSet::new(), next_uid: uid, dialog: None, viewer: None,
            progress: None, job: None, save_now: String::new(), want_paper: false, resealed: None, written: String::new(),
            sealed: None, pdf: None, pending: Vec::new(),
        }
    }

    fn uid(&mut self) -> u64 {
        self.next_uid += 1;
        self.next_uid
    }

    fn changes(&self) -> Vec<String> {
        let b = &self.baseline;
        let mut out = Vec::new();
        for u in &self.uploads {
            match b.uploads.iter().find(|o| o.uid == u.uid) {
                None => out.push(format!("Added {}", u.name)),
                Some(o) => {
                    if o.name != u.name {
                        out.push(format!("Renamed {} to {}", o.name, u.name));
                    }
                    if !Arc::ptr_eq(&o.bytes, &u.bytes) {
                        out.push(format!("Edited {}", u.name));
                    }
                }
            }
        }
        for o in &b.uploads {
            if !self.uploads.iter().any(|u| u.uid == o.uid) {
                out.push(format!("Removed {}", o.name));
            }
        }
        for sd in &self.seeds {
            match b.seeds.iter().find(|o| o.uid == sd.uid) {
                None => out.push(format!("Added seed phrase {}", sd.name)),
                Some(o) if o.name != sd.name || o.content != sd.content => out.push(format!("Changed seed phrase {}", sd.name)),
                _ => {}
            }
        }
        for o in &b.seeds {
            if !self.seeds.iter().any(|x| x.uid == o.uid) {
                out.push(format!("Removed seed phrase {}", o.name));
            }
        }
        if js_trim(&b.final_message) != js_trim(&self.final_message) {
            out.push("Changed the final message".into());
        }
        out
    }

    fn total_bytes(&self) -> u64 {
        self.uploads.iter().map(|u| u.bytes.len() as u64).sum::<u64>()
            + self.seeds.iter().map(|sd| sd.content.len() as u64).sum::<u64>()
            + js_trim(&self.final_message).len() as u64
    }

    fn name_taken(&self, name: &str, except: Option<u64>) -> bool {
        let n = name.to_lowercase();
        n == vault::FINAL_MESSAGE_NAME
            || self.uploads.iter().any(|u| Some(u.uid) != except && u.name.to_lowercase() == n)
            || self.seeds.iter().any(|sd| sd.name.to_lowercase() == n)
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { uploads: self.uploads.clone(), seeds: self.seeds.clone(), final_message: self.final_message.clone() }
    }

    /// `getContents()`
    fn contents(&self) -> Contents {
        Contents {
            uploads: self.uploads.iter().map(|u| Upload { name: u.name.clone(), mime: u.mime.clone(), bytes: u.bytes.clone() }).collect(),
            seeds: self.seeds.iter().map(|sd| Seed { name: sd.name.clone(), content: sd.content.clone() }).collect(),
            final_message: js_trim(&self.final_message).to_string(),
        }
    }

    /// Back to the last saved state, then what the recover page keeps: the contents, and the
    /// saved vault, so the editor reopens on the latest revision (the original reopened on the
    /// vault it first unlocked, numbering the next save from there).
    fn close(&mut self) -> Closed {
        self.uploads = self.baseline.uploads.clone();
        self.seeds = self.baseline.seeds.clone();
        self.final_message = self.baseline.final_message.clone();
        self.viewer = None;
        Closed { contents: self.contents(), session: self.session.clone(), revision: self.revision, modified: self.last_saved.clone() }
    }

    fn info(&mut self, title: &str, text: impl Into<String>) {
        self.dialog = Some(Dialog::Info { title: title.into(), body: vec![(text.into(), false)] });
    }

    /// The paper picker's rows: seed phrases, the final message, then files (name, bytes).
    fn paper_rows(&self) -> Vec<(String, usize)> {
        let mut v: Vec<(String, usize)> = self.seeds.iter().map(|sd| (sd.name.clone(), sd.content.len())).collect();
        let fm = js_trim(&self.final_message).len();
        if fm > 0 {
            v.push(("Final message".into(), fm));
        }
        v.extend(self.uploads.iter().map(|u| (u.name.clone(), u.bytes.len())));
        v
    }
}

/// `new Date(iso).toLocaleString()` in the app's locale (Chromium falls back to en-GB for English
/// regions it has no UI strings for, and uses the US form for en-US).
fn local_time(iso: &str) -> String {
    let Ok(d) = chrono::DateTime::parse_from_rfc3339(iso) else { return "Invalid Date".into() };
    let d = d.with_timezone(&chrono::Local);
    if crate::pages::common::us_locale() { d.format("%-m/%-d/%Y, %-I:%M:%S %p").to_string() } else { d.format("%d/%m/%Y, %H:%M:%S").to_string() }
}

// -------------------------------------------------------------------------------------------
// The editor
// -------------------------------------------------------------------------------------------

/// Draws the editor (or what is over it); `closed` gets the last saved contents once it closes.
pub fn show(e: &mut Editor, sh: &mut Shared, ui: &mut egui::Ui, closed: &mut Option<Closed>) {
    poll(e, ui.ctx());
    if let Some((uid, v)) = &mut e.viewer {
        let done = viewer::show(v, sh, ui, None);
        if let Some(bytes) = v.saved.take() {
            if let Some(u) = e.uploads.iter_mut().find(|u| u.uid == *uid) {
                u.bytes = Arc::new(bytes);
            }
        }
        if done {
            e.viewer = None;
        }
        return;
    }
    if let Some((label, pct)) = e.progress.clone() {
        layer(ui, sh, "ed-progress", false, |pg, width| {
            let w = width - 18.0;
            let h = pg.para(9.0, 9.0, w, &[s(&label, col::T1)], Ws::Normal, WrapOpts::default(), LINE);
            let ty = 9.0 + h + 12.0;
            pg.fill(9.0, ty, w, 4.0, col::LINE);
            pg.fill(9.0, ty, w * pct.clamp(2.0, 100.0) / 100.0, 4.0, col::T1);
            ty + 4.0 + 9.0
        });
        return;
    }
    if e.dialog.is_some() {
        dialog(e, sh, ui, closed);
        return;
    }
    main_view(e, sh, ui, closed);
}

fn main_view(e: &mut Editor, sh: &mut Shared, ui: &mut egui::Ui, closed: &mut Option<Closed>) {
    let changes = e.changes();
    let dirty = !changes.is_empty();
    let mut act: Option<(&'static str, u64)> = None;
    let mut fm = e.final_message.clone();
    if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
        act = Some(("save", 0));
    }
    let (dropped, dragging) = ui.input(|i| (i.raw.dropped_files.clone(), !i.raw.hovered_files.is_empty()));
    let status_tail = {
        let (nf, ns) = (e.uploads.len(), e.seeds.len());
        let rev = format!(" Revision {}{}", e.revision, if e.last_saved.is_empty() { String::new() } else { format!(", last saved {}", local_time(&e.last_saved)) });
        let counts = format!(" · {nf} file{}, {ns} seed phrase{} · {}", if nf == 1 { "" } else { "s" }, if ns == 1 { "" } else { "s" }, format_bytes(e.total_bytes()));
        [rev, counts]
    };
    layer(ui, sh, "editor", true, |pg, width| {
        let (x, w) = (9.0, width - 18.0);
        // --- header: title and status on the left, actions on the right or wrapped below
        pg.text(x, 9.0, "Vault Editor", col::WHITE);
        let (pill, pill_c) = if dirty { ("Unsaved changes", col::AMBER) } else { ("All changes saved", col::T2) };
        // each span is a flex item, so its leading space is dropped at the start of its line
        let items: Vec<f32> = std::iter::once(pg.measure(pill, 16.0) + 20.0).chain(status_tail.iter().map(|t| pg.measure(t.trim_start(), 16.0))).collect();
        // status: flex-wrap, 4px gaps, 25px rows (the pill's padding and border)
        let mut placed = Vec::new();
        let (mut sx, mut row_n) = (x, 0usize);
        for iw in &items {
            if sx > x && sx + iw > x + w {
                sx = x;
                row_n += 1;
            }
            placed.push((sx, 47.0 + row_n as f32 * 29.0));
            sx += iw + 4.0;
        }
        let status_w = items.iter().sum::<f32>() + 4.0 * (items.len() - 1) as f32;
        let status_h = (row_n + 1) as f32 * 25.0 + row_n as f32 * 4.0;
        pg.text(placed[0].0 + 10.0, placed[0].1 + 3.0, pill, pill_c);
        for (t, (px, py)) in status_tail.iter().zip(&placed[1..]) {
            pg.text(*px, py + 3.0, t.trim_start(), col::T2);
        }
        let bw = |pg: &Pg<'_>, l: &str| pg.measure(&format!("[{l}]"), 16.0);
        let actions_w = bw(pg, "Save vault") + 2.0 + bw(pg, "Close editor") + 8.0;
        let title_w = status_w.max(pg.measure("Vault Editor", 16.0));
        let (ax, ay, head_bottom) = if title_w + 16.0 + actions_w <= w {
            (x + w - actions_w, 9.0, (47.0 + status_h).max(9.0 + 27.0))
        } else {
            let ay = 47.0 + status_h + 16.0;
            (x, ay, ay + 27.0)
        };
        if pg.button("save", ax + 4.0, ay + 4.0, Btn::new("Save vault").main()).clicked() {
            act = Some(("save", 0));
        }
        let cx = ax + 4.0 + bw(pg, "Save vault") + 2.0;
        if pg.button("close", cx, ay + 4.0, Btn::new("Close editor").color(col::T2)).clicked() {
            act = Some(("close", 0));
        }
        let mut y = head_bottom + 14.0 + 1.0 + 12.0;
        y += hint(pg, x, y, w, "Everything you see here lives only in this window's memory. Nothing touches the disk until you save, and the saved file is encrypted with your answers.");

        // --- files (the drop zone: 1px border and 4px padding, pulled out 4px each side)
        let sec_top = y + 22.0;
        let (fx, fw) = (x + 1.0, w - 2.0);
        // drawn first so the rows paint over it; the height is known from last frame's layout
        let drop_id = pg.wid("drop-h");
        if dragging {
            let h: f32 = pg.ui.ctx().data(|d| d.get_temp(drop_id)).unwrap_or(0.0);
            pg.fill(x - 4.0, sec_top, w + 8.0, h, Color32::from_rgb(0x1b, 0x1b, 0x1b));
        }
        let mut y = sec_top + 5.0;
        if section_head(pg, fx, y, "FILES", "+ Add files", "add") {
            act = Some(("addfiles", 0));
        }
        y += 2.0 * LINE + 8.0;
        y += hint(pg, fx, y, fw, "Open a file to view it here (images, PDFs, text, audio and video). You can also drop files onto this window.");
        y += 6.0;
        if e.uploads.is_empty() {
            y += empty(pg, fx, y, fw, "No files yet. Add files with the button above or drop them here.");
        } else {
            for (k, u) in e.uploads.iter().enumerate() {
                let base = e.baseline.uploads.iter().find(|b| b.uid == u.uid);
                let tag = match base {
                    None => "new",
                    Some(b) if !Arc::ptr_eq(&b.bytes, &u.bytes) => "edited",
                    Some(b) if b.name != u.name => "renamed",
                    _ => "",
                };
                let badge = crate::pages::recover::badge(viewer::kind(&u.name, &u.mime, &u.bytes));
                let sub = Sub::Plain(format_bytes(u.bytes.len() as u64), tag);
                let acts = [("Open", "open"), ("Rename", "rename"), ("Download", "download"), ("Remove", "remove")];
                y += row(pg, fx, y, fw, badge, &u.name, sub, &acts, u.uid, &mut act);
                if k + 1 < e.uploads.len() {
                    y += 8.0;
                }
            }
        }
        let sec_bottom = y + 4.0 + 1.0;
        pg.ui.ctx().data_mut(|d| d.insert_temp(drop_id, sec_bottom - sec_top));

        // --- seed phrases
        let mut y = sec_bottom + 22.0;
        if section_head(pg, x, y, "SEED PHRASES", "+ Add seed phrase", "addseed") {
            act = Some(("addseed", 0));
        }
        y += 2.0 * LINE + 8.0;
        if e.seeds.is_empty() {
            y += empty(pg, x, y, w, "No seed phrases in this vault.");
        } else {
            for (k, sd) in e.seeds.iter().enumerate() {
                let shown = e.shown.contains(&sd.uid);
                let sub = if shown {
                    Sub::Seed(sd.content.clone())
                } else {
                    let n = vault::word_count(&sd.content);
                    Sub::Plain(format!("{n} word{} (hidden)", if n == 1 { "" } else { "s" }), "")
                };
                let acts = [(if shown { "Hide" } else { "Show" }, "toggle"), ("Edit", "editseed"), ("Remove", "removeseed")];
                y += row(pg, x, y, w, "KEY", &sd.name, sub, &acts, sd.uid, &mut act);
                if k + 1 < e.seeds.len() {
                    y += 8.0;
                }
            }
        }

        // --- final message
        let y = y + 22.0;
        pg.text(x, y, "FINAL MESSAGE", col::WHITE);
        let ty = y + 2.0 * LINE + 8.0;
        pg.input("final", x, ty, w, &mut fm, "A message for whoever opens this vault (optional)", InputOpts::area(5));
        ty + 5.0 * LINE + 4.0 + 9.0
    });
    e.final_message = fm;
    if !dropped.is_empty() {
        let paths: Vec<PathBuf> = dropped.iter().map(|f| f.path().to_path_buf()).collect();
        add_files(e, paths);
    }
    let Some((what, uid)) = act else { return };
    match what {
        "open" => {
            if let Some(u) = e.uploads.iter().find(|u| u.uid == uid) {
                let editable = viewer::kind(&u.name, &u.mime, &u.bytes) == viewer::Kind::Text;
                e.viewer = Some((uid, Viewer::new(&u.name, &u.mime, u.bytes.clone(), Some(editable))));
            }
        }
        "rename" => {
            if let Some(u) = e.uploads.iter().find(|u| u.uid == uid) {
                e.dialog = Some(Dialog::Rename { uid, value: u.name.clone(), error: String::new(), focus: true });
            }
        }
        "download" => {
            if let Some(u) = e.uploads.iter().find(|u| u.uid == uid) {
                let _ = crate::pages::common::save_bytes_any(&u.name, &u.bytes);
            }
        }
        "remove" => {
            if let Some(u) = e.uploads.iter().find(|u| u.uid == uid) {
                e.dialog = Some(remove_file_dialog(uid, &u.name));
            }
        }
        "toggle" => {
            if !e.shown.remove(&uid) {
                e.shown.insert(uid);
            }
        }
        "editseed" => {
            if let Some(sd) = e.seeds.iter().find(|x| x.uid == uid) {
                let n = &sd.name;
                let base = if n.to_lowercase().ends_with(".txt") { &n[..n.len() - 4] } else { n.as_str() };
                e.dialog = Some(Dialog::Seed { uid: Some(uid), name: base.to_string(), content: sd.content.clone(), error: String::new(), focus: true });
            }
        }
        "removeseed" => {
            if let Some(sd) = e.seeds.iter().find(|x| x.uid == uid) {
                e.dialog = Some(Dialog::Confirm {
                    what: Confirm::RemoveSeed(uid), title: "Remove seed phrase".into(), ok: "Remove", danger: true,
                    body: vec![(format!("Remove {} from this vault? Copies of the vault you saved before still contain it.", sd.name), false)],
                });
            }
        }
        "addseed" => e.dialog = Some(Dialog::Seed { uid: None, name: String::new(), content: String::new(), error: String::new(), focus: true }),
        "addfiles" => {
            if let Some(paths) = rfd::FileDialog::new().pick_files() {
                add_files(e, paths);
            }
        }
        "save" => e.dialog = Some(Dialog::Save { paper: false }),
        "close" => {
            if dirty {
                e.dialog = Some(Dialog::Confirm {
                    what: Confirm::Discard, title: "Discard changes?".into(), ok: "Discard changes", danger: true,
                    body: vec![("You have changes that are not saved yet. Close the editor and discard them?".into(), false)],
                });
            } else {
                *closed = Some(e.close());
            }
        }
        _ => {}
    }
}

fn remove_file_dialog(uid: u64, name: &str) -> Dialog {
    Dialog::Confirm {
        what: Confirm::RemoveFile(uid), title: "Remove file".into(), ok: "Remove", danger: true,
        body: vec![
            (format!("Remove {name} from this vault?"), false),
            ("Copies of the vault you saved before still contain it. Replace those copies after you save if it must be gone everywhere.".into(), true),
        ],
    }
}

/// p.mv-hint: muted, wraps; returns the height.
fn hint(pg: &mut Pg<'_>, x: f32, y: f32, w: f32, text: &str) -> f32 {
    pg.para(x, y, w, &[s(text, col::T2)], Ws::Normal, WrapOpts::default(), LINE)
}

/// p.mv-empty inside a list (its 8px margins stay inside the flex column); returns the list height.
fn empty(pg: &mut Pg<'_>, x: f32, y: f32, w: f32, text: &str) -> f32 {
    8.0 + hint(pg, x, y + 8.0, w, text) + 8.0
}

/// An uppercase h2 with its button 12px after it, centred on the heading's two lines.
fn section_head(pg: &mut Pg<'_>, x: f32, y: f32, title: &str, button: &str, key: &str) -> bool {
    let tw = pg.text(x, y, title, col::WHITE);
    pg.button(key, x + tw + 12.0, y + 9.5, Btn::new(button).color(col::T1)).clicked()
}

enum Sub {
    /// muted text, optional amber tag after it
    Plain(String, &'static str),
    /// a shown seed phrase: wraps anywhere, normal colour
    Seed(String),
}

/// One .mv-row (badge, name over sub, actions); returns its height.
#[allow(clippy::too_many_arguments)]
fn row(pg: &mut Pg<'_>, x: f32, top: f32, w: f32, badge: &str, name: &str, sub: Sub, actions: &[(&str, &'static str)], uid: u64, act: &mut Option<(&'static str, u64)>) -> f32 {
    let widths: Vec<f32> = actions.iter().map(|(l, _)| pg.measure(&format!("[{l}]"), 16.0)).collect();
    let acts_w = widths.iter().sum::<f32>() + 2.0 * (widths.len() - 1) as f32 + 6.0;
    let acts_x = x + w - 1.0 - acts_w;
    let main_x = x + 1.0 + 42.0 + 12.0;
    let main_w = (acts_x - 12.0 - main_x).max(0.0);
    let sub_lines = match &sub {
        Sub::Plain(..) => 1,
        Sub::Seed(t) => pg.para_h(main_w, &[s(t, col::T1)], Ws::PreWrap, WrapOpts { anywhere: true }).max(1),
    };
    let tagged = matches!(&sub, Sub::Plain(_, t) if !t.is_empty());
    let sub_h = sub_lines as f32 * LINE + if tagged { 2.0 } else { 0.0 };
    let content_h = (LINE + 2.0 + sub_h).max(30.0);
    let cy = top + 1.0;
    // badge (its box is the page colour, so only the letters show)
    let by = cy + (content_h - 30.0) / 2.0;
    pg.text(x + 2.0, by + 5.5, badge, col::T2);
    // name over sub
    let shown = if pg.measure(name, 16.0) > main_w { ellipsize(pg, name, main_w) } else { name.to_string() };
    pg.text(main_x, cy, &shown, col::T1);
    let sy = cy + LINE + 2.0;
    match &sub {
        Sub::Plain(t, tag) => {
            let ty = sy + if tagged { 1.0 } else { 0.0 };
            let tw = pg.text(main_x, ty, t, col::T2);
            if tagged {
                pg.text(main_x + tw + 8.0 + 1.0 + 6.0, ty, tag, col::AMBER);
            }
        }
        Sub::Seed(t) => {
            pg.para(main_x, sy, main_w, &[s(t, col::T1)], Ws::PreWrap, WrapOpts { anywhere: true }, LINE);
        }
    }
    // actions: centred, 3px padding, 2px apart
    let ay = cy + (content_h - 25.0) / 2.0 + 3.0;
    let mut bx = acts_x + 3.0;
    for ((label, key), bw) in actions.iter().zip(widths) {
        let c = if key.starts_with("remove") { col::RED } else { col::T2 };
        if pg.button((*key, uid), bx, ay, Btn::new(label).color(c)).clicked() {
            *act = Some((key, uid));
        }
        bx += bw + 2.0;
    }
    content_h + 2.0
}

// -------------------------------------------------------------------------------------------
// Adding files
// -------------------------------------------------------------------------------------------
fn add_files(e: &mut Editor, paths: Vec<PathBuf>) {
    let files: Vec<NewFile> = paths.into_iter().filter_map(|p| {
        let size = std::fs::metadata(&p).ok().filter(|m| m.is_file())?.len();
        let name = p.file_name()?.to_string_lossy().to_string();
        Some(NewFile { name, path: p, size })
    }).collect();
    if files.is_empty() {
        return;
    }
    let (too_big, files): (Vec<NewFile>, Vec<NewFile>) = files.into_iter().partition(|f| f.size > vault::MAX_FILE_BYTES);
    if !too_big.is_empty() {
        let one = too_big.len() == 1;
        let names: Vec<&str> = too_big.iter().map(|f| f.name.as_str()).collect();
        e.info("File too large", format!("{} {} larger than the 1.5 GB a single file in a vault can be, so {} not added.",
            names.join(", "), if one { "is" } else { "are" }, if one { "it was" } else { "they were" }));
        e.pending = files; // continues once the message is closed
        return;
    }
    add_checked(e, files);
}

fn add_checked(e: &mut Editor, files: Vec<NewFile>) {
    if files.is_empty() {
        return;
    }
    let current: u64 = e.uploads.iter().map(|u| u.bytes.len() as u64).sum();
    if current + files.iter().map(|f| f.size).sum::<u64>() > vault::MAX_VAULT_BYTES {
        e.info("Vault too large", "Adding these files would take the vault past 1.9 GB, which is more than Memoro Vault can seal and open. Nothing was added.");
        return;
    }
    let large: Vec<&str> = files.iter().filter(|f| f.size > LARGE_FILE).map(|f| f.name.as_str()).collect();
    if !large.is_empty() {
        let text = format!("{} {} over {}. Saving will take a while and needs plenty of free memory. Add anyway?",
            large.join(", "), if large.len() == 1 { "is" } else { "are" }, format_bytes(LARGE_FILE));
        e.dialog = Some(Dialog::Confirm { what: Confirm::Large, title: "Large file".into(), body: vec![(text, false)], ok: "Add", danger: false });
        e.pending = files;
        return;
    }
    push_files(e, files);
}

fn push_files(e: &mut Editor, files: Vec<NewFile>) {
    let mut errors = Vec::new();
    for f in files {
        match std::fs::read(&f.path) {
            Ok(bytes) => {
                let t = js_trim(&f.name);
                let mut name = if t.is_empty() { String::new() } else { vault::clean_file_name(t, "") };
                if name.is_empty() {
                    name = "file.bin".into();
                }
                if e.name_taken(&name, None) {
                    let mut reserved = vec![vault::FINAL_MESSAGE_NAME.to_string()];
                    reserved.extend(e.uploads.iter().map(|u| u.name.clone()));
                    reserved.extend(e.seeds.iter().map(|sd| sd.name.clone()));
                    name = vault::dedupe_names(&[name], &reserved).remove(0);
                }
                let mime = crate::pages::answers::guess_mime(&f.name).to_string();
                let uid = e.uid();
                e.uploads.push(Item { uid, name, mime, bytes: Arc::new(bytes) });
            }
            Err(err) => errors.push((format!("{}: {err}", f.name), false)),
        }
    }
    if !errors.is_empty() {
        e.dialog = Some(Dialog::Info { title: "Could not add file".into(), body: errors });
    }
}

// -------------------------------------------------------------------------------------------
// Saving
// -------------------------------------------------------------------------------------------
fn start_save(e: &mut Editor, ctx: &egui::Context) {
    let now = vault::iso_now();
    e.save_now = now.clone();
    let session = e.session.clone();
    let c = e.contents();
    let fm = e.final_message.clone();
    let extras = json!({ "revision": e.revision + 1, "modified": now }).as_object().cloned();
    let (tx, rx) = channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let ptx = Mutex::new(tx.clone());
        let pctx = ctx.clone();
        let progress = move |done: usize, total: usize, label: &str| {
            let pct = 5.0 + 80.0 * done as f32 / total.max(1) as f32;
            let _ = ptx.lock().map(|t| t.send(Msg::Progress(format!("Sealing: {label}"), pct)));
            pctx.request_repaint();
        };
        let r = vault::reseal_vault(&session, &c.uploads, &c.seeds, &fm, extras, Some(&progress)).map_err(|e| e.to_string());
        let _ = tx.send(Msg::Sealed(r));
        ctx.request_repaint();
    });
    e.progress = Some(("Sealing your vault...".into(), 2.0));
    e.job = Some(rx);
}

fn poll(e: &mut Editor, ctx: &egui::Context) {
    loop {
        let Some(rx) = &e.job else { return };
        match rx.try_recv() {
            Ok(Msg::Progress(label, pct)) => e.progress = Some((label, pct)),
            Ok(Msg::Sealed(r)) => {
                e.job = None;
                e.progress = None;
                match r {
                    Ok(res) => {
                        e.resealed = Some(res);
                        write_vault(e);
                    }
                    Err(err) => e.info("Save failed", format!("Nothing was saved. {err}")),
                }
                return;
            }
            Ok(Msg::Paper(r)) => {
                e.job = None;
                e.progress = None;
                match r {
                    Ok(p) => {
                        e.pdf = Some(p);
                        write_pdf(e);
                    }
                    Err(err) => saved_dialog(e, Some(format!("The paper backup could not be made: {err}"))),
                }
                return;
            }
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }
            Err(TryRecvError::Disconnected) => {
                e.job = None;
                e.progress = None;
                return;
            }
        }
    }
}

/// `saveUntilWritten` for the vault: the save dialog, then "Vault not saved" until written or given up.
fn write_vault(e: &mut Editor) {
    let Some(res) = &e.resealed else { return };
    match crate::pages::common::save_bytes("memoro-vault.zip", &res.zip_bytes, "ZIP", "zip") {
        Ok(file) => {
            let res = e.resealed.take().unwrap();
            commit(e, res, file);
        }
        Err(error) => e.dialog = Some(Dialog::NotSaved { paper: false, error }),
    }
}

/// Only a confirmed write makes this the saved state.
fn commit(e: &mut Editor, res: vault::Resealed, file: String) {
    for (u, n) in e.uploads.iter_mut().zip(&res.upload_names) {
        u.name = n.clone();
    }
    e.final_message = res.final_message.clone();
    let old = e.session.clone();
    e.session = Arc::new(Session {
        vault_json: vault::VaultJson { raw: res.sealed.vault_json.clone() },
        vault_enc: res.sealed.vault_enc.clone(),
        unlock_concat: old.unlock_concat.clone(),
        gate_answers: old.gate_answers.clone(),
        remaining_answers: old.remaining_answers.clone(),
        selected_answers: old.selected_answers.clone(),
    });
    e.revision += 1;
    e.last_saved = std::mem::take(&mut e.save_now);
    e.baseline = e.snapshot();
    e.written = file;
    if e.want_paper {
        e.sealed = Some(res.sealed);
        e.dialog = Some(paper_picker(e));
    } else {
        saved_dialog(e, None);
    }
}

fn paper_picker(e: &Editor) -> Dialog {
    let n = e.paper_rows().len();
    let preset = e.seeds.len() + usize::from(!js_trim(&e.final_message).is_empty());
    Dialog::Paper { picks: (0..n).map(|i| i < preset).collect(), error: String::new() }
}

fn saved_dialog(e: &mut Editor, paper: Option<String>) {
    let mut body = vec![(format!("Revision {} was sealed, re-opened and checked file by file, then saved as {}. It opens with the same answers as before.", e.revision, e.written), false)];
    if let Some(p) = paper {
        body.push((p, false));
    }
    body.push(("Copies you saved earlier still open with these answers and still hold their old contents. Replace them on every drive, disc and cloud folder where you keep this vault, and print the new paper backup if you made one.".into(), true));
    e.dialog = Some(Dialog::Info { title: "Vault saved".into(), body });
}

/// `makePaperBackup` after the picker: a lite vault with the full vault's salts and hidden order.
fn start_paper(e: &mut Editor, picks: &[bool], ctx: &egui::Context) {
    let Some(sealed) = e.sealed.take() else { return };
    let ns = e.seeds.len();
    let has_fm = !js_trim(&e.final_message).is_empty();
    let seeds: Vec<Seed> = e.seeds.iter().enumerate().filter(|(i, _)| picks[*i]).map(|(_, sd)| Seed { name: sd.name.clone(), content: sd.content.clone() }).collect();
    let fm = if has_fm && picks[ns] { e.final_message.clone() } else { String::new() };
    let off = ns + usize::from(has_fm);
    let uploads: Vec<Upload> = e.uploads.iter().enumerate().filter(|(i, _)| picks[off + i]).map(|(_, u)| Upload { name: u.name.clone(), mime: u.mime.clone(), bytes: u.bytes.clone() }).collect();
    let session = e.session.clone();
    let extras = json!({ "revision": e.revision, "modified": e.last_saved, "lite": true }).as_object().cloned();
    let fixed = vault::Fixed { layer1_salt: sealed.layer1_salt.clone(), full_salt: sealed.full_salt.clone(), permutation: sealed.permutation.clone() };
    let (tx, rx) = channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let r = (|| -> Result<(Vec<u8>, usize, usize), String> {
            let l1 = vault::decrypt_layer1(&session.unlock_concat, &session.vault_json, &session.vault_enc).map_err(|e| e.to_string())?;
            let lite = vault::seal_vault(vault::SealArgs {
                unlock_concat: &session.unlock_concat,
                layer1: &l1,
                gate_answers: &session.gate_answers,
                remaining_answers: &session.remaining_answers,
                selected_answers: &session.selected_answers,
                question_prompts: session.vault_json.question_prompts(),
                uploads: &uploads,
                seeds: &seeds,
                final_message: &fm,
                created: session.vault_json.created(),
                meta_extras: extras,
                fixed: Some(fixed),
                on_progress: None,
            }).map_err(|e| e.to_string())?;
            let zip = vault::package_zip(&lite, false).map_err(|e| e.to_string())?;
            let _ = tx.send(Msg::Progress("Drawing QR codes...".into(), 60.0));
            ctx.request_repaint();
            let p = mvault_core::paper::build_paper_pdf(&zip).map_err(|e| e.to_string())?;
            Ok((p.bytes, p.pages, p.tiles))
        })();
        let _ = tx.send(Msg::Paper(r));
        ctx.request_repaint();
    });
    e.progress = Some(("Building the paper backup...".into(), 20.0));
    e.job = Some(rx);
}

fn write_pdf(e: &mut Editor) {
    let Some((pdf, _, _)) = &e.pdf else { return };
    match crate::pages::common::save_bytes("memoro-lite-QR.pdf", pdf, "PDF", "pdf") {
        Ok(file) => {
            let (_, pages, tiles) = e.pdf.take().unwrap();
            saved_dialog(e, Some(format!("The paper backup ({pages} page{}, {} QR codes) was saved as {file}.", if pages == 1 { "" } else { "s" }, tiles + 2)));
        }
        Err(error) => e.dialog = Some(Dialog::NotSaved { paper: true, error }),
    }
}

// -------------------------------------------------------------------------------------------
// Dialogs: title, body (block margins collapse), then the buttons 2px apart
// -------------------------------------------------------------------------------------------

/// Vertical flow with collapsing margins: `y` is the last block's bottom, `mb` its bottom margin.
struct Flow {
    y: f32,
    mb: f32,
}

impl Flow {
    fn top(&self, mt: f32) -> f32 {
        self.y + self.mb.max(mt)
    }
    fn end(&mut self, bottom: f32, mb: f32) {
        self.y = bottom;
        self.mb = mb;
    }
}

/// `.mv-dialog-body p`: 8px margins.
fn dp(pg: &mut Pg<'_>, f: &mut Flow, w: f32, spans: &[Span]) {
    let t = f.top(8.0);
    let h = pg.para(9.0, t, w, spans, Ws::Normal, WrapOpts::default(), LINE);
    f.end(t + h, 8.0);
}

/// p.mv-error: 8px margins, at least 16px tall even when empty.
fn derr(pg: &mut Pg<'_>, f: &mut Flow, w: f32, text: &str) {
    let t = f.top(8.0);
    let h = if text.is_empty() { 0.0 } else { pg.para(9.0, t, w, &[s(text, col::RED)], Ws::Normal, WrapOpts::default(), LINE) };
    f.end(t + h.max(16.0), 8.0);
}

/// label.mv-label: muted caption, 6px, then the field; 12px below. Returns the field's y.
fn dlabel(pg: &mut Pg<'_>, f: &mut Flow, caption: &str, field_h: f32) -> f32 {
    let t = f.top(0.0);
    pg.text(9.0, t, caption, col::T2);
    let fy = t + LINE + 6.0;
    f.end(fy + field_h, 12.0);
    fy
}

fn dialog(e: &mut Editor, sh: &mut Shared, ui: &mut egui::Ui, closed: &mut Option<Closed>) {
    let Some(mut d) = e.dialog.take() else { return };
    let mut res: Option<usize> = None;
    let mut flip: Option<usize> = None;
    let changes = e.changes();
    let rows = e.paper_rows();
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
    layer(ui, sh, "ed-dialog", true, |pg, width| {
        let avail = width - 18.0;
        let x = 9.0;
        let title = match &d {
            Dialog::Rename { .. } => "Rename file".to_string(),
            Dialog::Confirm { title, .. } | Dialog::Info { title, .. } => title.clone(),
            Dialog::Seed { uid, .. } => if uid.is_some() { "Edit seed phrase".into() } else { "Add seed phrase".into() },
            Dialog::Save { .. } => "Save vault".into(),
            Dialog::NotSaved { paper, .. } => format!("{} not saved", if *paper { "Paper backup" } else { "Vault" }),
            Dialog::Paper { .. } => "Paper backup contents".into(),
        };
        pg.text(x, 9.0, &title, col::WHITE);
        let mut f = Flow { y: 9.0 + LINE, mb: LINE };
        let bw = |pg: &Pg<'_>, l: &str| pg.measure(&format!("[{l}]"), 16.0);
        let buttons: Vec<Btn<'_>> = match &mut d {
            Dialog::Rename { value, error, focus, .. } => {
                // the dialog is as wide as its widest content
                let fit = [INPUT_INTRINSIC, pg.measure(&title, 16.0), pg.measure(error, 16.0), bw(pg, "Cancel") + 2.0 + bw(pg, "Save") + 8.0]
                    .into_iter().fold(0.0, f32::max).min(avail);
                let fy = dlabel(pg, &mut f, "New name", LINE);
                let out = pg.input("rn", x, fy, fit, value, "", InputOpts::line());
                if std::mem::take(focus) {
                    out.response.request_focus();
                }
                derr(pg, &mut f, avail, error);
                vec![Btn::new("Cancel").color(col::T2), Btn::new("Save").main()]
            }
            Dialog::Confirm { body, ok, danger, .. } => {
                for (t, note) in body.iter() {
                    dp(pg, &mut f, avail, &[s(t.as_str(), if *note { col::T2 } else { col::T1 })]);
                }
                vec![Btn::new("Cancel").color(col::T2), Btn::new(ok).color(if *danger { col::RED } else { col::WHITE })]
            }
            Dialog::Seed { uid, name, content, error, focus } => {
                let note = "Type the phrase rather than pasting it, so it never passes through the clipboard.";
                let ok = if uid.is_some() { "Keep changes" } else { "Add" };
                let fit = [INPUT_INTRINSIC, pg.measure(note, 16.0), pg.measure(&title, 16.0), pg.measure("Name (saved as name.txt)", 16.0), pg.measure(error, 16.0)]
                    .into_iter().fold(0.0, f32::max).min(avail);
                let fy = dlabel(pg, &mut f, "Name (saved as name.txt)", LINE);
                let out = pg.input("sn", x, fy, fit, name, "for example btc_wallet", InputOpts::line());
                if std::mem::take(focus) {
                    out.response.request_focus();
                }
                let cy = dlabel(pg, &mut f, "Seed phrase", 5.0 * LINE);
                pg.input("sc", x, cy, fit, content, "seed words or key", InputOpts::area(5));
                dp(pg, &mut f, avail, &[s(note, col::T2)]);
                derr(pg, &mut f, avail, error);
                vec![Btn::new("Cancel").color(col::T2), Btn::new(ok).main()]
            }
            Dialog::Save { paper } => {
                if changes.is_empty() {
                    dp(pg, &mut f, avail, &[s("Nothing has changed. Saving now makes a fresh copy of the same contents with new salts and new encryption, which can be useful when you want to replace every stored copy.", col::T1)]);
                } else {
                    dp(pg, &mut f, avail, &[s("These changes will be sealed into a new vault file:", col::T1)]);
                    let mut ly = f.top(6.0);
                    let more = changes.len().saturating_sub(12);
                    let items = changes.iter().take(12).cloned().chain((more > 0).then(|| format!("and {more} more")));
                    for c in items {
                        ly += li(pg, x + 18.0, ly, avail - 18.0, &[s(c, col::T1)], col::T1);
                    }
                    f.end(ly, 10.0);
                }
                dp(pg, &mut f, avail, &[s("Your questions and answers stay the same. Saving takes a few seconds per file because every file gets its own Argon2id key.", col::T2)]);
                let t = f.top(12.0);
                let label = "Also make a new paper backup (QR PDF)";
                let rw = 4.0 * CH + 8.0 + pg.measure(label, 16.0);
                let row_rect = pg.rect(x, t, rw, LINE);
                let hit = pg.ui.interact(row_rect, pg.wid("paper-row"), egui::Sense::click());
                if pg.checkbox("paper", x, t, *paper, true).clicked() || hit.clicked() {
                    *paper = !*paper;
                }
                pg.text(x + 4.0 * CH + 8.0, t, label, col::T1);
                f.end(t + LINE, 0.0);
                vec![Btn::new("Cancel").color(col::T2), Btn::new("Save new vault").main()]
            }
            Dialog::Info { body, .. } => {
                for (t, note) in body.iter() {
                    dp(pg, &mut f, avail, &[s(t.as_str(), if *note { col::T2 } else { col::T1 })]);
                }
                vec![Btn::new("OK").main()]
            }
            Dialog::NotSaved { paper, error } => {
                let text = if error.is_empty() {
                    format!("The {} has not been saved to disk yet.", if *paper { "paper backup" } else { "vault" })
                } else {
                    format!("The file could not be written: {error}")
                };
                dp(pg, &mut f, avail, &[s(text, col::T1)]);
                vec![Btn::new("Do not save").color(col::T2), Btn::new("Choose a location").main()]
            }
            Dialog::Paper { picks, error } => {
                dp(pg, &mut f, avail, &[s("QR codes hold about 150 KB, so pick the items that matter most (seed phrases and the final message are selected for you).", col::T1)]);
                // .mv-pick-list: 1px border, 6px padding, rows 31px tall 4px apart, at most 40vh
                let lt = f.top(0.0);
                let inner = rows.len() as f32 * 31.0 + rows.len().saturating_sub(1) as f32 * 4.0;
                let max_h = (pg.ui.ctx().content_rect().height() * 0.4).floor();
                let box_h = (inner + 14.0).min(max_h);
                let picks_ro = picks.clone();
                let mut clicked = None;
                pg.scroll_box("picks", x + 7.0, lt + 7.0, avail - 14.0, box_h - 14.0, |pg, w| {
                    for (k, (name, size)) in rows.iter().enumerate() {
                        let ry = k as f32 * 35.0;
                        let row_rect = pg.rect(0.0, ry, w, 31.0);
                        let hit = pg.ui.interact(row_rect, pg.wid(("pick-row", k)), egui::Sense::click());
                        if pg.checkbox(("pk", k), 6.0, ry + 6.0, picks_ro[k], true).clicked() || hit.clicked() {
                            clicked = Some(k);
                        }
                        let sz = format_bytes(*size as u64);
                        let szw = pg.measure(&sz, 16.0);
                        let nx = 6.0 + 4.0 * CH + 10.0;
                        let nw = (w - 6.0 - szw - 10.0 - nx).max(0.0);
                        let shown = if pg.measure(name, 16.0) > nw { ellipsize(pg, name, nw) } else { name.clone() };
                        pg.text(nx, ry + 6.0, &shown, col::T1);
                        pg.text(w - 6.0 - szw, ry + 6.0, &sz, col::T2);
                    }
                    inner
                });
                flip = clicked;
                f.end(lt + box_h, 0.0);
                let total: usize = rows.iter().zip(picks.iter()).filter(|(_, on)| **on).map(|(r, _)| r.1).sum();
                dp(pg, &mut f, avail, &[s("Selected: ", col::T1), s(format!("{} of {}", format_bytes(total as u64), format_bytes(vault::LITE_CAP_BYTES)), col::WHITE)]);
                let over = if total as u64 > vault::LITE_CAP_BYTES { "That is more than a paper backup can hold. Untick some items." } else { "" };
                let shown_err = if error.is_empty() { over.to_string() } else { error.clone() };
                derr(pg, &mut f, avail, &shown_err);
                vec![Btn::new("Skip").color(col::T2), Btn::new("Make paper backup").main()]
            }
        };
        let by = f.top(16.0) + 4.0;
        let mut bx = x + 4.0;
        for (k, b) in buttons.iter().enumerate() {
            if pg.button(("db", k), bx, by, *b).clicked() {
                res = Some(k);
            }
            bx += bw(pg, b.label) + 2.0;
        }
        by + LINE + 4.0 + 9.0
    });
    if let (Some(k), Dialog::Paper { picks, error }) = (flip, &mut d) {
        picks[k] = !picks[k];
        // `update()` rewrites the message on every change
        error.clear();
    }
    if res.is_none() && enter && matches!(d, Dialog::Rename { .. }) {
        res = Some(1);
    }
    let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
    // Escape resolves a dialog with null: the same as its first (cancel-like) button
    let res = match res {
        Some(k) => k,
        None if escape => 0,
        None => {
            e.dialog = Some(d);
            return;
        }
    };
    let ctx = ui.ctx().clone();
    match d {
        Dialog::Rename { uid, value, focus, .. } if res == 1 => {
            let t = js_trim(&value);
            let clean = if t.is_empty() { String::new() } else { vault::clean_file_name(t, "") };
            let error = if clean.is_empty() {
                "Please enter a name."
            } else if e.name_taken(&clean, Some(uid)) {
                "Another item in this vault already uses that name."
            } else {
                if let Some(u) = e.uploads.iter_mut().find(|u| u.uid == uid) {
                    u.name = clean;
                }
                return;
            };
            e.dialog = Some(Dialog::Rename { uid, value, error: error.into(), focus });
        }
        Dialog::Confirm { what, .. } => {
            let ok = res == 1;
            match what {
                Confirm::RemoveFile(uid) if ok => e.uploads.retain(|u| u.uid != uid),
                Confirm::RemoveSeed(uid) if ok => {
                    e.seeds.retain(|x| x.uid != uid);
                    e.shown.remove(&uid);
                }
                Confirm::Discard if ok => *closed = Some(e.close()),
                Confirm::Large => {
                    let files = std::mem::take(&mut e.pending);
                    if ok {
                        push_files(e, files);
                    }
                }
                _ => {}
            }
        }
        Dialog::Seed { uid, name, content, focus, .. } if res == 1 => {
            let n = vault::seed_file_name(&name);
            let text = js_trim(&content).to_string();
            let error = if n.is_empty() {
                "Please enter a name."
            } else if text.is_empty() {
                "Please enter the seed phrase."
            } else if e.seeds.iter().any(|x| Some(x.uid) != uid && x.name == n) || e.uploads.iter().any(|u| u.name.to_lowercase() == n) {
                "Another item in this vault already uses that name."
            } else {
                match uid {
                    Some(id) => {
                        if let Some(x) = e.seeds.iter_mut().find(|x| x.uid == id) {
                            x.name = n;
                            x.content = text;
                        }
                    }
                    None => {
                        let id = e.uid();
                        e.seeds.push(SeedItem { uid: id, name: n, content: text });
                    }
                }
                return;
            };
            e.dialog = Some(Dialog::Seed { uid, name, content, error: error.into(), focus });
        }
        Dialog::Save { paper } if res == 1 => {
            e.want_paper = paper;
            start_save(e, &ctx);
        }
        Dialog::Info { .. } => {
            let files = std::mem::take(&mut e.pending);
            add_checked(e, files);
        }
        Dialog::NotSaved { paper, .. } => match (paper, res == 1) {
            (false, true) => write_vault(e),
            (false, false) => {
                e.resealed = None;
                e.info("Changes kept", "The new vault was not saved, so nothing changed on disk. Your edits are still here, and you can save again whenever you are ready.");
            }
            (true, true) => write_pdf(e),
            (true, false) => {
                e.pdf = None;
                saved_dialog(e, Some("The paper backup was not saved.".into()));
            }
        },
        Dialog::Paper { picks, .. } if res == 1 => {
            let total: usize = rows.iter().zip(picks.iter()).filter(|(_, on)| **on).map(|(r, _)| r.1).sum();
            if total == 0 {
                e.dialog = Some(Dialog::Paper { picks, error: "Select at least one item.".into() });
            } else if total as u64 > vault::LITE_CAP_BYTES {
                e.dialog = Some(Dialog::Paper { picks, error: String::new() });
            } else {
                start_paper(e, &picks, &ctx);
            }
        }
        Dialog::Paper { .. } => {
            e.sealed = None;
            saved_dialog(e, Some("The paper backup was skipped.".into()));
        }
        _ => {}
    }
}

/// For snapshots: the editor in the states the reference capture recorded.
impl Editor {
    pub fn snap(&mut self, what: &str) {
        let change = |e: &mut Editor| {
            if let Some(u) = e.uploads.iter_mut().find(|u| u.name == "accounts.txt") {
                u.name = "accounts-and-contacts.txt".into();
            }
            e.final_message = "A changed final message.".into();
            if let Some(uid) = e.seeds.first().map(|sd| sd.uid) {
                e.shown.insert(uid);
            }
        };
        match what {
            "changed" => change(self),
            "save" | "save-checked" => {
                change(self);
                self.dialog = Some(Dialog::Save { paper: what == "save-checked" });
            }
            "rename" => {
                if let Some(u) = self.uploads.iter().find(|u| u.name == "accounts.txt") {
                    self.dialog = Some(Dialog::Rename { uid: u.uid, value: u.name.clone(), error: String::new(), focus: true });
                }
            }
            "remove" => {
                change(self);
                if let Some(u) = self.uploads.iter().find(|u| u.name == "cabin-sunset.png") {
                    self.dialog = Some(remove_file_dialog(u.uid, &u.name));
                }
            }
            "add-seed" => {
                change(self);
                self.dialog = Some(Dialog::Seed { uid: None, name: String::new(), content: String::new(), error: String::new(), focus: true });
            }
            "paper" => {
                change(self);
                self.dialog = Some(paper_picker(self));
            }
            "saved" => {
                change(self);
                self.revision = 2;
                self.written = "memoro-vault.zip".into();
                saved_dialog(self, Some("The paper backup (4 pages, 31 QR codes) was saved as memoro-lite-QR.pdf.".into()));
            }
            "viewer-text" | "viewer-text-edit" => {
                if let Some(u) = self.uploads.iter().find(|u| u.name == "accounts.txt") {
                    let mut v = Viewer::new(&u.name, &u.mime, u.bytes.clone(), Some(true));
                    if what == "viewer-text-edit" {
                        v.snap_edit();
                    }
                    self.viewer = Some((u.uid, v));
                }
            }
            _ => {}
        }
    }
}
