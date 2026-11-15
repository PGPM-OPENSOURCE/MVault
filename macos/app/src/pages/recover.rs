//! recover.html: answer every question, find the hidden order, decrypt and verify the files,
//! then the recovered vault (viewer, editor, download), all in memory.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use mvault_core::recover::{self, Progress};
use mvault_core::vault::{self, Contents, Layer1, QuestionView, VaultJson};

use crate::app::{Page, Shared, layer, layer_over};
use crate::pages::common::code;
use crate::pages::dashboard::Dashboard;
use crate::pages::{editor, viewer};
use crate::ui::{Btn, CH, InputOpts, LINE, Span, Ws, WrapOpts, col, s};

pub struct Handoff {
    pub zip: Arc<Vec<u8>>,
    pub vault_json: VaultJson,
    pub layer1: Layer1,
    pub unlock_concat: zeroize::Zeroizing<String>,
}

enum Msg {
    Line(String),
    Slow(String),
    Progress(usize, usize),
    Done(Result<Recovered, String>),
}

pub struct Recovered {
    pub contents: Contents,
    pub meta: vault::VaultMeta,
    pub failed: usize,
    pub session: Arc<vault::Session>,
}

pub struct Recover {
    h: Handoff,
    order: Vec<usize>,
    questions: Vec<QuestionView>,
    answers: Vec<String>,
    mcq: Vec<Vec<String>>,
    rh_modal: bool,
    running: Option<Receiver<Msg>>,
    terminal: Vec<String>,
    slow: Option<(String, usize, f64)>,
    blocks: (usize, usize),
    recovered: Option<Recovered>,
    toast: Option<(String, bool, f64)>,
    viewer: Option<viewer::Viewer>,
    editor: Option<editor::Editor>,
    alert: Option<String>,
}

impl Recover {
    pub fn new(h: Handoff) -> Recover {
        let l = &h.layer1;
        let order: Vec<usize> = l.gates.iter().chain(&l.remaining).chain(&l.selected).copied().collect();
        let questions: Vec<QuestionView> = l.question_list.iter().map(QuestionView::from_json).collect();
        let mcq = order.iter().map(|&i| {
            let q = questions.get(i).cloned().unwrap_or_default();
            if !q.mcq_enabled {
                return vec![];
            }
            let mut seen = Vec::new();
            for o in q.mcq_options.unwrap_or_default() {
                let t = mvault_core::jsstr::js_trim(&o).to_string();
                if !t.is_empty() && !seen.contains(&t) {
                    seen.push(t);
                }
            }
            if seen.len() < 2 {
                return vec![];
            }
            mvault_core::crypto::display_shuffle(&mut seen);
            seen
        }).collect();
        let n = order.len();
        Recover { h, order, questions, answers: vec![String::new(); n], mcq, rh_modal: true, running: None, terminal: vec![], slow: None, blocks: (0, 40),
                  recovered: None, toast: None, viewer: None, editor: None, alert: None }
    }
}

/// recover.js updateHint(): "_ _ S _ _" with spaces kept, or the multiple-choice line.
fn hint_line(q: &QuestionView) -> (Option<String>, Option<String>, bool) {
    if q.mcq_enabled && q.mcq_options.is_some() {
        let n = q.mcq_options.as_ref().map(|o| o.len()).unwrap_or(0);
        let custom = mvault_core::jsstr::js_trim(&q.custom).to_string();
        return (None, Some(format!("Multiple choice ({n} options){}", if custom.is_empty() { String::new() } else { format!(" — Hint: {custom}") })), true);
    }
    let len = q.expected_length.unwrap_or(0.0).clamp(0.0, 500.0) as usize;
    let mut rendered: Vec<char> = Vec::new();
    let (mut pos, mut vi) = (1usize, 0usize);
    for j in 0..len {
        if q.space_indexes.contains(&(j as f64)) {
            rendered.push(' ');
        } else if q.hint_letters.contains(&(pos as f64)) {
            let v = q.hint_values.get(vi).cloned().flatten();
            vi += 1;
            rendered.push(v.and_then(|v| v.chars().next()).unwrap_or('_'));
        } else {
            rendered.push('_');
        }
        pos += 1;
    }
    let code: String = rendered.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(" ");
    let custom = if q.custom.is_empty() { None } else { Some(format!("Hint: {}", q.custom)) };
    (Some(code), custom, false)
}

pub fn show(st: &mut Recover, sh: &mut Shared, ui: &mut egui::Ui) {
    poll(st, ui.ctx(), sh.time);
    if let Some(msg) = st.alert.take() {
        rfd::MessageDialog::new().set_title("Memoro Vault").set_description(msg).set_buttons(rfd::MessageButtons::Ok).show();
    }
    if let Some(ed) = &mut st.editor {
        let mut closed = None;
        editor::show(ed, sh, ui, &mut closed);
        if let Some(c) = closed {
            if let Some(r) = &mut st.recovered {
                r.contents = c.contents;
                r.session = c.session;
                if let Some(m) = r.meta.raw.as_object_mut() {
                    m.insert("revision".into(), serde_json::json!(c.revision));
                    if !c.modified.is_empty() {
                        m.insert("modified".into(), serde_json::json!(c.modified));
                    }
                }
            }
            st.editor = None;
        }
        return;
    }
    if let Some(v) = &mut st.viewer {
        if viewer::show(v, sh, ui, Some("recover")) {
            st.viewer = None;
        }
        return;
    }
    if st.running.is_some() {
        return pow_overlay(st, sh, ui);
    }
    if st.recovered.is_some() {
        // the page stays underneath (its scrollbar shows beside the recovered vault)
        if !crate::app::has_geom(ui.ctx(), "recover") {
            page(st, sh, ui);
        }
        return recovered(st, sh, ui);
    }
    if st.rh_modal {
        if !crate::app::has_geom(ui.ctx(), "recover") {
            page(st, sh, ui);
        }
        return rh_modal(st, sh, ui);
    }
    page(st, sh, ui);
    toast(st, sh, ui);
}

fn page(st: &mut Recover, sh: &mut Shared, ui: &mut egui::Ui) {
    let mut submit = false;
    let mut pick: Option<(usize, String)> = None;
    layer(ui, sh, "recover", true, |pg, width| {
        let mut y = 8.0;
        y += pg.para(8.0, y, width - 16.0, &[s("Recover Your Vault", col::WHITE)], Ws::Normal, WrapOpts::default(), LINE) + LINE;
        // .recovery-form: up to 600px wide, 16px side padding, 8px gaps
        let fw = (width - 16.0).min(600.0);
        let bx = 8.0 + 16.0 + 1.0;
        let bw = fw - 32.0 - 2.0;
        for (r, &oi) in st.order.iter().enumerate() {
            let q = st.questions.get(oi).cloned().unwrap_or_default();
            let by = y + 1.0;
            let label = [s(format!("{}.", r + 1), col::T3).margin_r(2.0), s(format!(" {}", q.question), col::T1)];
            let mut yy = by + pg.para(bx, by, bw, &label, Ws::Normal, WrapOpts::default(), LINE) + 10.0;
            pg.input(("a", r), bx, yy, bw, &mut st.answers[r], "", InputOpts::lower());
            yy += LINE + 10.0;
            let opts = &st.mcq[r];
            if !opts.is_empty() {
                yy += 2.0;
                let mut xx = bx;
                let cur = mvault_core::jsstr::js_trim(&st.answers[r]).to_string();
                let mut labels: Vec<String> = opts.clone();
                labels.push("Clear".into());
                for (k, o) in labels.iter().enumerate() {
                    let w = pg.measure(&format!("[{o}]"), 16.0);
                    if xx > bx && xx + w > bx + bw {
                        xx = bx;
                        yy += LINE + 6.0;
                    }
                    let is_clear = k == opts.len();
                    let selected = !is_clear && !cur.is_empty() && o.to_lowercase() == cur;
                    // the chosen answer is shown inverted (the original lost its fill and showed
                    // dark text on the page colour, which read as hidden)
                    let c = if is_clear { col::T3 } else { col::T2 };
                    if pg.button(("mcq", r, k), xx, yy, Btn::new(o).color(c).selected(selected)).clicked() {
                        pick = Some((r, if is_clear { String::new() } else { o.to_lowercase() }));
                    }
                    xx += w + 6.0;
                }
                yy += LINE + 2.0 + 10.0;
            }
            let (code_s, custom, italic) = hint_line(&q);
            let mut hh = 0.0;
            if italic {
                hh = pg.para(bx, yy, bw, &[s(custom.clone().unwrap_or_default(), col::T3)], Ws::Normal, WrapOpts::default(), LINE);
            } else if code_s.as_deref().is_some_and(|c| !c.is_empty()) || custom.is_some() {
                let cw = pg.text(bx, yy, code_s.as_deref().unwrap_or(""), col::T2);
                if let Some(c) = &custom {
                    pg.text(bx + cw + if cw > 0.0 { 12.0 } else { 0.0 }, yy, c, col::T2);
                }
                hh = LINE;
            }
            yy += hh;
            y = yy + 1.0 + LINE + 8.0;
        }
        y -= 8.0; // the form ends after the last block's 19px margin (no gap after it)
        let by = y + 20.0;
        if pg.button("recover", 8.0, by, Btn::new("Recover Vault")).clicked() {
            submit = true;
        }
        // the empty .spacer divs' bottom margins collapse into one 100px gap
        by + LINE + 100.0 + 8.0
    });
    if let Some((r, v)) = pick {
        st.answers[r] = v;
    }
    if submit {
        start(st, ui.ctx());
    }
}

fn rh_modal(st: &mut Recover, sh: &mut Shared, ui: &mut egui::Ui) {
    let (t1, t2) = (col::T1, col::T2);
    let spans: Vec<Span> = vec![
        s("One of these questions might be a ", t2), s("Red Herring", t1), s(" — a trick question that’s ", t2), s("not supposed to be answered correctly", t2).underline(), s(".\n\nIf a Red Herring is present, you must ", t2),
        s("leave it blank", t1), s(". Entering an answer—even the real one—will prevent you from unlocking the vault.\n\n", t2), s("How do I spot it?", t1),
        s("\n• For text-entry questions: check the character hint beneath each question. If the expected character count ", t2), s("differs from the real answer by 2 or more characters", t2).underline(),
        s(", it’s likely the Red Herring.\n• For multiple-choice (MCQ) questions: if none of the options are correct, it is likely the Red Herring and ", t2), s("must be left unanswered", t2).underline(),
        s(".\n\n", t2), s("Example:", t1), s(" If your favorite dog’s name was “", t2), code("Samantha"), s("” (8 letters), but the hint only shows 4 characters, ", t2), s("do not answer that question", t2).underline(),
        s(".\n\nThere is either ", t2), s("exactly one", t1), s(" Red Herring or ", t2), s("none at all", t1), s("—never more than one.", t2),
    ];
    let mut ok = false;
    layer_over(ui, sh, "rh", Some("recover"), true, |pg, width| {
        let w = width - 18.0;
        let (x, mut y) = (9.0, 9.0);
        let title = "Pay Attention Before You Begin";
        let tw = pg.measure(title, 16.0).min(w);
        pg.text(x + (w - tw) / 2.0, y, title, col::WHITE);
        y += 2.0 * LINE + 12.0;
        y += pg.para(x, y, w, &spans, Ws::PreLine, WrapOpts::default(), LINE) + 12.0 + 6.0;
        let bw = pg.measure("[Got it]", 16.0);
        ok = pg.button("ok", x + (w - bw) / 2.0, y, Btn::new("Got it")).clicked();
        y + LINE + 9.0
    });
    if ok {
        st.rh_modal = false;
    }
}

// -------------------------------------------------------------------------------------------
// Recovery on a worker thread
// -------------------------------------------------------------------------------------------
fn start(st: &mut Recover, ctx: &egui::Context) {
    let mut by_index = std::collections::HashMap::new();
    for (r, &oi) in st.order.iter().enumerate() {
        by_index.insert(oi, mvault_core::jsstr::normalize_answer(&st.answers[r]));
    }
    if by_index.values().all(|a| a.is_empty()) {
        st.toast = Some(("Please fill out the recovery answers first.".into(), true, f64::NAN));
        return;
    }
    let (tx, rx) = channel();
    let zip = st.h.zip.clone();
    let l1 = st.h.layer1.clone();
    let unlock = st.h.unlock_concat.clone();
    let ctx = ctx.clone();
    st.terminal.clear();
    st.blocks = (0, 40);
    std::thread::spawn(move || {
        let send = |m: Msg| {
            let _ = tx.send(m);
            ctx.request_repaint();
        };
        let r = work(&zip, &l1, &unlock, &by_index, &send);
        send(Msg::Done(r));
    });
    st.running = Some(rx);
}

/// The brute force and decryption (recover.js handleRecovery), reporting through `send`.
fn work(zip: &[u8], l1: &Layer1, unlock: &zeroize::Zeroizing<String>, by_index: &std::collections::HashMap<usize, String>, send: &dyn Fn(Msg)) -> Result<Recovered, String> {
    send(Msg::Slow("[*] Loading vault...".into()));
    let vz = vault::read_vault_zip(zip).map_err(|e| e.to_string())?;
    send(Msg::Slow("[*] Brute-forcing 720 permutations...".into()));
    let at = |i: usize| by_index.get(&i).cloned().unwrap_or_default();
    let un = recover::find_order(&vz.vault_json, vz.meta_bytes(), l1, &at, &mut |p| match p {
        Progress::Attempt { n, total } => {
            send(Msg::Line(format!("[{n}/{total}] trying answer order #{n}...")));
            send(Msg::Progress(n, total));
        }
        Progress::Unlocked { n } => send(Msg::Slow(format!("[+] Vault unlocked on attempt {n}."))),
        _ => {}
    }, &|| false).map_err(|e| e.to_string())?;
    let files = recover::decrypt_files(&vz, &un.meta, &un.permuted_concat, &mut |p| match p {
        Progress::FileOk(f) => send(Msg::Line(format!("[+] Decrypted and verified {f}"))),
        Progress::FileSkipped(f) => send(Msg::Line(format!("[!] Missing {f}, skipping."))),
        Progress::FileFailed(f, e) => send(Msg::Line(format!("[!] Failed to decrypt {f}: {e}"))),
        _ => {}
    });
    let failed = un.meta.files.len() - files.len();
    let session = vault::Session {
        vault_json: vz.vault_json.clone(),
        vault_enc: vz.vault_enc.clone(),
        unlock_concat: unlock.clone(),
        gate_answers: l1.gates.iter().map(|&i| at(i)).collect(),
        remaining_answers: l1.remaining.iter().map(|&i| at(i)).collect(),
        selected_answers: un.permutation.clone(),
    };
    let contents = vault::split_contents(&un.meta, files);
    Ok(Recovered { contents, meta: un.meta, failed, session: Arc::new(session) })
}

/// Snapshot states, set up the way the reference capture reached them.
impl Recover {
    pub fn snap(&mut self, what: &str, answers: &[&str], trap: usize, mcq_order: &[&str]) {
        if let Some(path) = what.strip_prefix("viewer-file:") {
            self.rh_modal = false;
            let name = path.rsplit('/').next().unwrap_or("file").to_string();
            let bytes = Arc::new(std::fs::read(path).unwrap_or_default());
            let mut v = viewer::Viewer::new(&name, crate::pages::answers::guess_mime(&name), bytes, None);
            v.snap_play = true;
            self.viewer = Some(v);
            return;
        }
        // the reference capture's shuffle of the multiple-choice options
        let want: Vec<String> = mcq_order.iter().map(|s| s.to_string()).collect();
        for m in &mut self.mcq {
            let (mut a, mut b) = (m.clone(), want.clone());
            a.sort();
            b.sort();
            if a == b {
                *m = want.clone();
            }
        }
        if what == "rh" {
            return;
        }
        self.rh_modal = false;
        if what == "form" {
            return;
        }
        for (r, &oi) in self.order.iter().enumerate() {
            self.answers[r] = if oi == trap { String::new() } else { answers[oi].to_lowercase() };
        }
        if what == "filled" {
            return;
        }
        if what == "pow" {
            self.terminal = vec!["[*] Loading vault...".into(), "[*] Brute-forcing 720 permutations...".into()];
            self.terminal.extend((1..=27).map(|n| format!("[{n}/720] trying answer order #{n}...")));
            self.blocks = ((27 * 40) / 720, 40);
            let (tx, rx) = channel();
            std::mem::forget(tx); // keeps the overlay up: the channel never closes
            self.running = Some(rx);
            return;
        }
        let mut by_index = std::collections::HashMap::new();
        for (r, &oi) in self.order.iter().enumerate() {
            by_index.insert(oi, mvault_core::jsstr::normalize_answer(&self.answers[r]));
        }
        match work(&self.h.zip, &self.h.layer1, &self.h.unlock_concat, &by_index, &|_| {}) {
            Ok(rec) => self.recovered = Some(rec),
            Err(e) => {
                eprintln!("snapshot recovery failed: {e}");
                return;
            }
        }
        let rec = self.recovered.as_ref().unwrap();
        if what == "toast" {
            // captured after the editor saved: one file renamed, a new final message
            if let Some(r) = &mut self.recovered {
                if let Some(u) = r.contents.uploads.iter_mut().find(|u| u.name == "accounts.txt") {
                    u.name = "accounts-and-contacts.txt".into();
                }
                r.contents.final_message = "A changed final message.".into();
            }
            self.toast = Some(("Opening the donation page in your browser.".into(), false, f64::NAN));
        } else if let Some(file) = what.strip_prefix("viewer:") {
            if let Some(it) = items(&rec.contents).into_iter().find(|i| i.name == file) {
                self.viewer = Some(viewer::Viewer::new(&it.name, &it.mime, it.bytes.clone(), None));
            }
        } else if let Some(state) = what.strip_prefix("editor") {
            let mut ed = editor::Editor::new(rec.session.clone(), &rec.contents, &rec.meta);
            ed.snap(state.trim_start_matches('-'));
            self.editor = Some(ed);
        }
    }
}

fn poll(st: &mut Recover, ctx: &egui::Context, now: f64) {
    // typed lines (printLineSlow, 30ms per character)
    if let Some((line, shown, started)) = &mut st.slow {
        let want = (((now - *started) / 0.03) as usize).min(line.chars().count());
        *shown = want;
        if want >= line.chars().count() {
            let l = line.clone();
            st.terminal.push(l);
            st.slow = None;
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
            return;
        }
    }
    let Some(rx) = &st.running else { return };
    loop {
        if st.slow.is_some() {
            return;
        }
        let Ok(m) = rx.try_recv() else { return };
        match m {
            Msg::Line(l) => {
                st.terminal.push(l);
                if st.terminal.len() > 400 {
                    st.terminal.drain(..st.terminal.len() - 400);
                }
            }
            Msg::Slow(l) => {
                st.slow = Some((l, 0, now));
                ctx.request_repaint();
            }
            Msg::Progress(n, t) => st.blocks = ((n * 40) / t.max(1), 40),
            Msg::Done(r) => {
                st.running = None;
                match r {
                    Ok(rec) => st.recovered = Some(rec),
                    Err(e) => st.alert = Some(format!("Error: {e}")),
                }
                return;
            }
        }
    }
}

fn pow_overlay(st: &mut Recover, sh: &mut Shared, ui: &mut egui::Ui) {
    let lines = st.terminal.clone();
    let slow = st.slow.clone();
    let (filled, total) = st.blocks;
    layer(ui, sh, "pow", false, |pg, width| {
        let mut y = 8.0;
        pg.text(8.0, y, "Trying permutations...", col::T1);
        y += LINE + 4.0 + 16.0;
        // the terminal: 300px tall, kept scrolled to the bottom
        let th = 300.0;
        let (tx, tw) = (9.0, width - 18.0);
        let mut all: Vec<String> = lines.clone();
        if let Some((l, n, _)) = &slow {
            all.push(l.chars().take(*n).collect());
        }
        let inner = th - 2.0;
        let content = all.len() as f32 * LINE;
        let first = if content > inner { content - inner } else { 0.0 };
        let clip = pg.rect(tx, y + 1.0, tw, inner);
        let old = pg.ui.clip_rect();
        pg.ui.set_clip_rect(clip.intersect(old));
        for (k, l) in all.iter().enumerate() {
            let ly = y + 1.0 + k as f32 * LINE - first;
            if ly + LINE >= y && ly <= y + th {
                pg.text(tx, ly, l, col::T2);
            }
        }
        pg.ui.set_clip_rect(old);
        if content > inner {
            let thumb = (inner * inner / content).max(18.0);
            pg.fill(tx + tw - 6.0 + 1.0, y + 1.0 + inner - thumb, 6.0, thumb, col::LINE_HI);
        }
        y += th + 16.0;
        pg.text(8.0, y, "Processing...", col::T2);
        y += LINE + 16.0;
        crate::pages::build::progress_chars(pg, 8.0, y, total, (filled * 100 / total.max(1)) as u32);
        y + 21.0 + 8.0
    });
}

// -------------------------------------------------------------------------------------------
// The recovered vault
// -------------------------------------------------------------------------------------------
pub struct Item {
    pub name: String,
    pub mime: String,
    pub bytes: Arc<Vec<u8>>,
}

fn items(c: &Contents) -> Vec<Item> {
    let mut v: Vec<Item> = c.uploads.iter().map(|u| Item { name: u.name.clone(), mime: u.mime.clone(), bytes: u.bytes.clone() }).collect();
    v.extend(c.seeds.iter().map(|sd| Item { name: sd.name.clone(), mime: "text/plain".into(), bytes: Arc::new(sd.content.as_bytes().to_vec()) }));
    if !c.final_message.is_empty() {
        v.push(Item { name: vault::FINAL_MESSAGE_NAME.into(), mime: "text/plain".into(), bytes: Arc::new(c.final_message.as_bytes().to_vec()) });
    }
    v
}

pub fn badge(kind: viewer::Kind) -> &'static str {
    match kind {
        viewer::Kind::Image => "IMG",
        viewer::Kind::Pdf => "PDF",
        viewer::Kind::Text => "TXT",
        viewer::Kind::Audio => "AUD",
        viewer::Kind::Video => "VID",
        viewer::Kind::Other => "BIN",
    }
}

fn recovered(st: &mut Recover, sh: &mut Shared, ui: &mut egui::Ui) {
    let rec = st.recovered.as_ref().unwrap();
    let list = items(&rec.contents);
    let count = list.len();
    let failed = rec.failed;
    let fm = rec.contents.final_message.clone();
    let mut action: Option<(&'static str, usize)> = None;
    layer_over(ui, sh, "seed", Some("recover"), true, |pg, width| {
        let w = width - 18.0;
        // support section: as wide as its text, centred
        let t2 = "If you find it valuable, consider sending a small tip in XMR or your preferred cryptocurrency.";
        let sw = pg.measure(t2, 16.0).min(w - 2.0) + 2.0;
        let sx = 9.0 + (w - sw) / 2.0 + 1.0;
        let mut y = 10.0;
        pg.text(sx, y, "SUPPORT MEMORO VAULT", col::T3);
        y += LINE + 10.0;
        pg.text(sx, y, "Thanks for using Memoro Vault.", col::T1);
        y += LINE;
        y += pg.para(sx, y, sw - 2.0, &[s(t2, col::T2)], Ws::Normal, WrapOpts::default(), LINE) + 12.0;
        if pg.button("donate", sx, y, Btn::new("Donate with preferred crypto")).clicked() {
            action = Some(("donate", 0));
        }
        y += LINE + 1.0 + 12.0;
        // recovered section
        let (x, iw) = (10.0, w - 2.0);
        y += 1.0;
        pg.text(x, y, "Vault Recovered!", col::WHITE);
        y += 2.0 * LINE;
        let summary = if count > 0 { format!("{count} file(s) decrypted and verified.{}", if failed > 0 { format!(" {failed} file(s) could not be decrypted.") } else { String::new() }) } else { "No files recovered.".into() };
        y += pg.para(x, y, iw, &[s(&summary, col::T2)], Ws::PreWrap, WrapOpts { anywhere: true }, LINE);
        if !fm.is_empty() {
            y += 14.0 + 1.0;
            y += pg.para(x + 1.0, y, iw - 2.0, &[s(format!("Final message:\n\n{fm}"), col::T1)], Ws::PreWrap, WrapOpts { anywhere: true }, LINE) + 1.0;
        }
        y += 18.0;
        pg.text(x, y, "DECRYPTED FILES:", col::WHITE);
        y += 2.0 * LINE + 9.0;
        for (k, it) in list.iter().enumerate() {
            let kind = viewer::kind(&it.name, &it.mime, &it.bytes);
            pg.fill(x + 1.0, y, 42.0, 30.0, col::BG);
            pg.text(x + 2.0, y + 5.5, badge(kind), col::T2);
            let size = mvault_core::format_bytes(it.bytes.len() as u64);
            let dl_w = pg.measure("[Download]", 16.0);
            let open_w = pg.measure("[Open]", 16.0);
            let right = x + iw - 1.0;
            let dl_x = right - dl_w;
            let open_x = dl_x - 10.0 - open_w;
            let size_w = pg.measure(&size, 16.0);
            let size_x = open_x - 10.0 - size_w;
            let name_x = x + 1.0 + 42.0 + 10.0;
            let name_max = (size_x - 10.0 - name_x).max(CH);
            let nw = pg.measure(&it.name, 16.0);
            let shown = if nw > name_max { crate::pages::common::ellipsize(pg, &it.name, name_max) } else { it.name.clone() };
            pg.text(name_x, y + 5.5, &shown, col::T1);
            pg.text(size_x, y + 5.5, &size, col::T2);
            if pg.button(("open", k), open_x, y + 5.5, Btn::new("Open")).clicked() {
                action = Some(("open", k));
            }
            if pg.button(("dl", k), dl_x, y + 5.5, Btn::new("Download")).clicked() {
                action = Some(("download", k));
            }
            y += 48.0;
        }
        y += 25.0 - 18.0;
        let btns = [Btn::new("Open Vault Editor").main(), Btn::new("Download All as ZIP"), Btn::new("Return to Dashboard")];
        let mut bx = x;
        for (k, b) in btns.iter().enumerate() {
            if pg.button(("act", k), bx, y, *b).clicked() {
                action = Some((["editor", "zipall", "back"][k], 0));
            }
            bx += pg.measure(&format!("[{}]", b.label), 16.0) + 2.0;
        }
        y += LINE + 12.0;
        y += pg.para(x, y, iw, &[s("The vault editor lets you view, add, rename and remove files, change seed phrases and the final message, then save a new copy that opens with the same answers.", col::T3)], Ws::Normal, WrapOpts::default(), LINE);
        y + 16.0 + 1.0 + 9.0
    });
    toast(st, sh, ui);
    let Some((what, k)) = action else { return };
    let rec = st.recovered.as_ref().unwrap();
    match what {
        "donate" => {
            const URL: &str = "https://trocador.app/en/anonpay/?ticker_to=xmr&network_to=Mainnet&address=83czGNh6SKbhmjg3wPzeiDRQbN7gkLLqTYSvfMGRQRmKQf1SyQTG88Db67NoBdEvpCii6Qzcxq3BxNt94FDeJutmJ3xBXc6&donation=True&amount=0.1&name=Kasmaristo+Delvakto&description=Memoro+Vault+is+funded+by+donations+only.+Thanks+for+your+support!&ticker_from=xmr&network_from=Mainnet&bgcolor=";
            if std::process::Command::new("open").arg(URL).spawn().is_ok() {
                st.toast = Some(("Opening the donation page in your browser.".into(), false, f64::NAN));
            } else {
                st.toast = Some(("Could not open the donation page.".into(), true, f64::NAN));
            }
        }
        "open" => {
            let list = items(&rec.contents);
            let it = &list[k];
            st.viewer = Some(viewer::Viewer::new(&it.name, &it.mime, it.bytes.clone(), None));
        }
        "download" => {
            let list = items(&rec.contents);
            let it = &list[k];
            let ext = it.name.rsplit('.').next().unwrap_or("").to_string();
            let _ = crate::pages::common::save_bytes(&it.name, &it.bytes, "File", &ext);
        }
        "zipall" => {
            let list = items(&rec.contents);
            match zip_all(&list) {
                Ok(z) => {
                    if let Err(e) = crate::pages::common::save_bytes("Memoro_Recovered_Files.zip", &z, "ZIP", "zip") {
                        if !e.is_empty() {
                            st.toast = Some((format!("Could not save the files: {e}"), true, f64::NAN));
                        }
                    }
                }
                Err(e) => st.toast = Some((format!("Could not save the files: {e}"), true, f64::NAN)),
            }
        }
        "editor" => {
            if rec.failed > 0 {
                st.alert = Some(format!("Editing unavailable: {} file(s) in this vault could not be decrypted. Saving from the editor would leave them out, so editing is turned off for this vault.", rec.failed));
            } else {
                st.editor = Some(editor::Editor::new(rec.session.clone(), &rec.contents, &rec.meta));
            }
        }
        "back" => {
            wipe_clipboard();
            sh.flow.wipe();
            sh.nav = Some(crate::app::Nav::Replace(Page::Dashboard(Dashboard::default())));
        }
        _ => {}
    }
}

/// "Download All as ZIP": names made unique with "n_" prefixes, as recover.js does.
fn zip_all(list: &[Item]) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let mut used = std::collections::HashSet::new();
    for it in list {
        let mut name = if it.name.is_empty() { "file.bin".to_string() } else { it.name.clone() };
        let mut n = 1;
        while used.contains(&name.to_lowercase()) {
            name = format!("{n}_{}", it.name);
            n += 1;
        }
        used.insert(name.to_lowercase());
        w.start_file(name.as_str(), zip::write::SimpleFileOptions::default()).map_err(|e| e.to_string())?;
        w.write_all(&it.bytes).map_err(|e| e.to_string())?;
    }
    Ok(w.finish().map_err(|e| e.to_string())?.into_inner())
}

/// wipeClipboard(): leave a single space on the clipboard.
pub fn wipe_clipboard() {
    if let Ok(mut c) = arboard::Clipboard::new() {
        let _ = c.set_text(" ");
    }
}

fn toast(st: &mut Recover, sh: &mut Shared, ui: &mut egui::Ui) {
    let Some((text, err, started)) = &mut st.toast else { return };
    if started.is_nan() {
        *started = sh.time;
    }
    let age = sh.time - *started;
    if age > 2.3 {
        st.toast = None;
        return;
    }
    ui.ctx().request_repaint();
    let alpha = if age > 1.8 { (1.0 - (age - 1.8) / 0.5).clamp(0.0, 1.0) } else { 1.0 } as f32;
    let full = ui.max_rect();
    let tw = crate::ui::measure(ui.ctx(), text, 16.0);
    let (w, h) = (tw + 2.0, LINE + 2.0);
    let x = full.center().x - w / 2.0;
    let y = full.max.y - 28.0 - h;
    let p = ui.painter();
    p.rect_filled(egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h)), 0.0, col::DIALOG.gamma_multiply(alpha));
    let c = if *err { col::RED } else { col::T1 };
    let g = ui.fonts_mut(|f| f.layout_no_wrap(text.clone(), crate::ui::font(16.0), c));
    let b = g.rows.first().and_then(|r| r.row.glyphs.first().map(|gl| r.pos.y + gl.pos.y)).unwrap_or(0.0);
    ui.painter().galley(egui::pos2(x + 1.0, y + 1.0 + 15.0 - b), g, c.gamma_multiply(alpha));
}
