//! build-vault.html: seal and verify the vault, save it, pick the paper backup contents, make the
//! QR PDF, then the "Please Read Carefully" instructions.

use std::sync::mpsc::{Receiver, channel};

use mvault_core::vault::{self, Seed, Upload};
use mvault_core::{build, paper};

use crate::app::{Page, Shared, layer};
use crate::pages::common::{button_row, code, li, li_num, save_bytes};
use crate::pages::dashboard::Dashboard;
use crate::ui::{Btn, CH, LINE, Pg, Span, Ws, WrapOpts, col, s};

enum Msg {
    Label(String, Option<u32>),
    Sealed(Result<Sealed, String>),
    Pdf(Result<Vec<u8>, String>),
}

struct Sealed {
    zip: Vec<u8>,
    plan: build::Plan,
    full: vault::Sealed,
}

enum Stage {
    Start,
    Working,
    Picker,
    Done,
}

pub struct Build {
    label: String,
    pct: u32,
    stage: Stage,
    rx: Option<Receiver<Msg>>,
    sealed: Option<Sealed>,
    picker: Picker,
    post: bool,
    started_at: f64,
}

#[derive(Default)]
struct Picker {
    seeds: Vec<bool>,
    uploads: Vec<bool>,
    final_msg: bool,
    error: String,
}

impl Build {
    pub fn new() -> Build {
        Build { label: "Initializing vault builder...".into(), pct: 0, stage: Stage::Start, rx: None, sealed: None, picker: Picker::default(), post: false, started_at: f64::NAN }
    }
}

impl Build {
    /// Snapshot scenarios: the picker (first seed ticked or not) or the instructions.
    pub fn snap(stage: &str, flow: &crate::app::Flow) -> Build {
        let mut b = Build::new();
        b.label = "[*] Choose what to include in your paper backup...".into();
        b.pct = 92;
        let fm = flow.user_data.as_ref().map(|u| mvault_core::jsstr::js_trim(&u.final_message).len()).unwrap_or(0);
        b.picker = Picker { seeds: vec![stage == "picker-checked"; flow.named_seed_files.len()], uploads: vec![false; flow.uploads.len()], final_msg: fm > 0, error: String::new() };
        b.stage = Stage::Picker;
        if stage == "post" {
            b.label = "[+] Full vault and paper PDF created.".into();
            b.pct = 100;
            b.stage = Stage::Done;
            b.post = true;
        }
        b
    }
}

fn alert(text: &str) {
    rfd::MessageDialog::new().set_title("Memoro Vault").set_description(text).set_buttons(rfd::MessageButtons::Ok).show();
}

fn start(st: &mut Build, sh: &Shared, ctx: &egui::Context) {
    let Some(user) = sh.flow.user_data.clone() else {
        st.label = "[!] Error: User data not found in IndexedDB.".into();
        st.stage = Stage::Done;
        alert("Something went wrong while creating your vault.");
        return;
    };
    let uploads: Vec<Upload> = sh.flow.uploads.iter().map(|u| Upload { name: u.name.clone(), mime: u.mime.clone(), bytes: u.bytes.clone() }).collect();
    let seeds = sh.flow.named_seed_files.clone();
    let (tx, rx) = channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let say = |t: &str, p: Option<u32>| {
            let _ = tx.send(Msg::Label(t.into(), p));
            ctx.request_repaint();
        };
        let pause = |ms| std::thread::sleep(std::time::Duration::from_millis(ms));
        say("[*] Loading user input...", Some(5));
        pause(200);
        say("[*] Preparing vault structure...", Some(15));
        say(&format!("[+] Found {} file(s) for encryption.", uploads.len()), Some(20));
        pause(150);
        say("[*] Selecting and permuting questions...", Some(30));
        let plan = build::plan(&user);
        say("[+] Encrypting FULL vault...", Some(45));
        let tx2 = tx.clone();
        let ctx2 = ctx.clone();
        let prog = move |done: usize, total: usize, _l: &str| {
            let _ = tx2.send(Msg::Label("[+] Encrypting FULL vault...".into(), Some(45 + (30.0 * done as f64 / total as f64).round() as u32)));
            ctx2.request_repaint();
        };
        let r = (|| -> Result<Sealed, String> {
            let full = build::seal_full(&plan, &uploads, &seeds, &user.final_message, Some(&prog)).map_err(|e| e.to_string())?;
            say("[*] Packaging FULL vault...", Some(76));
            let zip = vault::package_zip(&full, true).map_err(|e| e.to_string())?;
            say("[*] Verifying vault (re-opening and decrypting every file)...", Some(80));
            let fm = mvault_core::jsstr::js_trim(&user.final_message).to_string();
            let mut exp: Vec<vault::Expected<'_>> = uploads.iter().map(|u| vault::Expected { name: &u.name, bytes: &u.bytes }).collect();
            exp.extend(seeds.iter().map(|s| vault::Expected { name: &s.name, bytes: s.content.as_bytes() }));
            if !fm.is_empty() {
                exp.push(vault::Expected { name: vault::FINAL_MESSAGE_NAME, bytes: fm.as_bytes() });
            }
            vault::verify_vault_zip(&zip, &plan.unlock_concat, &full.permuted_concat, Some(&exp)).map_err(|e| e.to_string())?;
            Ok(Sealed { zip, plan, full })
        })();
        let _ = tx.send(Msg::Sealed(r));
        ctx.request_repaint();
    });
    st.rx = Some(rx);
    st.stage = Stage::Working;
}

fn build_lite(st: &mut Build, sh: &Shared, ctx: &egui::Context) {
    let Some(sealed) = &st.sealed else { return };
    let user = sh.flow.user_data.clone().unwrap_or_default();
    let uploads: Vec<Upload> = sh.flow.uploads.iter().zip(&st.picker.uploads).filter(|(_, on)| **on).map(|(u, _)| Upload { name: u.name.clone(), mime: u.mime.clone(), bytes: u.bytes.clone() }).collect();
    let seeds: Vec<Seed> = sh.flow.named_seed_files.iter().zip(&st.picker.seeds).filter(|(_, on)| **on).map(|(s, _)| s.clone()).collect();
    let fm = if st.picker.final_msg && !mvault_core::jsstr::js_trim(&user.final_message).is_empty() { user.final_message.clone() } else { String::new() };
    st.label = "[*] Building Lite Vault (using your selection)...".into();
    st.pct = 94;
    let plan = build::Plan {
        unlock_concat: sealed.plan.unlock_concat.clone(),
        layer1: sealed.plan.layer1.clone(),
        gate_answers: sealed.plan.gate_answers.clone(),
        remaining_answers: sealed.plan.remaining_answers.clone(),
        selected_answers: sealed.plan.selected_answers.clone(),
        question_prompts: sealed.plan.question_prompts.clone(),
    };
    let fixed = (sealed.full.layer1_salt.clone(), sealed.full.full_salt.clone(), sealed.full.permutation.clone());
    let (tx, rx) = channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let r = (|| -> Result<Vec<u8>, String> {
            let full_like = vault::Sealed {
                vault_json: serde_json::Value::Null, vault_enc: serde_json::Value::Null, meta_bytes: vec![], files: vec![], file_meta: vec![],
                layer1_salt: fixed.0, full_salt: fixed.1, permutation: fixed.2, permuted_concat: Default::default(), vault_hash: String::new(),
            };
            let lite = build::seal_lite(&plan, &full_like, &uploads, &seeds, &fm).map_err(|e| e.to_string())?;
            let lite_zip = vault::package_zip(&lite, false).map_err(|e| e.to_string())?;
            let _ = tx.send(Msg::Label("[*] Encrypting Lite ZIP and creating single-file QR PDF...".into(), Some(96)));
            ctx.request_repaint();
            Ok(paper::build_paper_pdf(&lite_zip).map_err(|e| e.to_string())?.bytes)
        })();
        let _ = tx.send(Msg::Pdf(r));
        ctx.request_repaint();
    });
    st.rx = Some(rx);
    st.stage = Stage::Working;
}

fn poll(st: &mut Build, sh: &mut Shared, ctx: &egui::Context) {
    let Some(rx) = &st.rx else { return };
    let mut msgs = Vec::new();
    while let Ok(m) = rx.try_recv() {
        msgs.push(m);
    }
    for m in msgs {
        match m {
            Msg::Label(t, p) => {
                st.label = t;
                if let Some(p) = p {
                    st.pct = p;
                }
            }
            Msg::Sealed(Err(e)) | Msg::Pdf(Err(e)) => {
                st.rx = None;
                st.label = format!("[!] Error: {e}");
                st.stage = Stage::Done;
                alert("Something went wrong while creating your vault.");
            }
            Msg::Sealed(Ok(sealed)) => {
                st.rx = None;
                if !save_until_saved("memoro-vault.zip", &sealed.zip) {
                    st.label = "[!] The vault was not saved. Use Retry Download to build it again.".into();
                    st.stage = Stage::Done;
                    alert("The vault was not saved. Use Retry Download to build it again.");
                    return;
                }
                st.label = "[+] Full vault verified and saved.".into();
                st.pct = 90;
                st.label = "[*] Choose what to include in your paper backup...".into();
                st.pct = 92;
                st.sealed = Some(sealed);
                let fm = mvault_core::jsstr::js_trim(&sh.flow.user_data.as_ref().map(|u| u.final_message.clone()).unwrap_or_default()).len();
                st.picker = Picker { seeds: vec![false; sh.flow.named_seed_files.len()], uploads: vec![false; sh.flow.uploads.len()], final_msg: fm > 0, error: String::new() };
                st.stage = Stage::Picker;
            }
            Msg::Pdf(Ok(pdf)) => {
                st.rx = None;
                // pdf.save() is an ordinary download: Electron asks where to save it
                if let Err(e) = save_bytes("memoro-lite-QR.pdf", &pdf, "PDF", "pdf") {
                    if !e.is_empty() {
                        alert(&format!("The paper backup could not be written: {e}"));
                    }
                }
                st.label = "[+] Full vault and paper PDF created.".into();
                st.pct = 100;
                st.stage = Stage::Done;
                st.post = true;
            }
        }
    }
    let _ = ctx;
}

/// saveVaultBytes(): keeps offering the save dialog until the file is written or declined.
fn save_until_saved(name: &str, bytes: &[u8]) -> bool {
    loop {
        match save_bytes(name, bytes, "ZIP", "zip") {
            Ok(_) => return true,
            Err(e) => {
                let why = if e.is_empty() { "The vault was not saved.".to_string() } else { format!("The vault could not be written ({e}).") };
                let again = rfd::MessageDialog::new().set_title("Memoro Vault").set_description(format!("{why}\n\nChoose a location again?"))
                    .set_buttons(rfd::MessageButtons::OkCancel).show();
                if again != rfd::MessageDialogResult::Ok {
                    return false;
                }
            }
        }
    }
}

pub fn show(st: &mut Build, sh: &mut Shared, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    if st.started_at.is_nan() {
        st.started_at = sh.time;
    }
    if matches!(st.stage, Stage::Start) && sh.snap.is_none() {
        start(st, sh, &ctx);
    }
    poll(st, sh, &ctx);
    if st.post {
        return post_modal(st, sh, ui);
    }
    if matches!(st.stage, Stage::Picker) {
        return picker(st, sh, ui);
    }
    let mut res = None;
    let (label, pct) = (st.label.clone(), st.pct);
    layer(ui, sh, "build", true, |pg, width| {
        let (x, w) = (8.0, width - 16.0);
        let mut y = 18.0;
        y += pg.para(x, y, w, &[s(&label, col::T2)], Ws::Normal, WrapOpts::default(), LINE) + 10.0;
        progress_chars(pg, x, y, 20, pct);
        y += 21.0 + 20.0 + 26.0;
        let (c, h) = button_row(pg, "actions", x, y, w, &[Btn::new("Back to Answers"), Btn::new("Retry Download"), Btn::new("Return to Dashboard")]);
        res = c;
        y + h + 8.0
    });
    match res {
        Some(0) => sh.nav = Some(crate::app::Nav::Back),
        Some(1) => *st = Build::new(),
        Some(2) => {
            sh.flow.wipe();
            sh.nav = Some(crate::app::Nav::Replace(Page::Dashboard(Dashboard::default())));
        }
        _ => {}
    }
}

/// `[####........]`: n blocks with 1px (invisible) borders; filled blocks are "#", the rest ".".
pub fn progress_chars(pg: &mut Pg<'_>, x: f32, y: f32, n: usize, pct: u32) {
    let filled = ((pct as f32 / 100.0) * n as f32).round() as usize;
    pg.text(x, y + 1.0, "[", col::T1);
    let mut bx = x + CH;
    for i in 0..n {
        let (t, c) = if i < filled { ("#", col::T1) } else { (".", col::T4) };
        pg.text(bx + 1.0, y + 1.0, t, c);
        bx += CH + 2.0;
    }
    pg.text(bx, y + 1.0, "]", col::T1);
}

fn picker(st: &mut Build, sh: &mut Shared, ui: &mut egui::Ui) {
    let seeds: Vec<(String, usize)> = sh.flow.named_seed_files.iter().map(|s| (s.name.clone(), s.content.len())).collect();
    let uploads: Vec<(String, usize)> = sh.flow.uploads.iter().map(|u| (u.name.clone(), u.bytes.len())).collect();
    let fm_len = mvault_core::jsstr::js_trim(&sh.flow.user_data.as_ref().map(|u| u.final_message.clone()).unwrap_or_default()).len();
    let total: usize = seeds.iter().zip(&st.picker.seeds).filter(|(_, on)| **on).map(|(s, _)| s.1).sum::<usize>()
        + if st.picker.final_msg { fm_len } else { 0 }
        + uploads.iter().zip(&st.picker.uploads).filter(|(_, on)| **on).map(|(u, _)| u.1).sum::<usize>();
    let cap = vault::LITE_CAP_BYTES as usize;
    let fmt = |n: usize| mvault_core::fmt_bytes_build(n as u64);
    let error = if !st.picker.error.is_empty() { st.picker.error.clone() } else if total > cap { format!("Too large. Reduce selection below {}.", fmt(cap)) } else { String::new() };
    let mut action = None;
    let mut flip_seed = None;
    let mut flip_up = None;
    let mut flip_fm = false;
    let picks = (st.picker.seeds.clone(), st.picker.uploads.clone(), st.picker.final_msg);
    layer(ui, sh, "lite-picker", true, |pg, width| {
        let (x0, y0) = (9.0, 9.0);
        let inner = width - 18.0;
        // header: padding 18px 22px, 1px bottom border
        pg.para(x0 + 22.0, y0 + 18.0, inner - 44.0, &[s("Choose what to include in your Paper Backup (≤ 150 KB)", col::WHITE)], Ws::Normal, WrapOpts::default(), LINE);
        let body_y = y0 + 18.0 + 2.0 * LINE + 18.0 + 1.0 + 18.0;
        let bw = inner - 44.0;
        let (lw, rw) = if bw >= 582.0 { let free = bw - 582.0; (340.0 + free / 2.0, 220.0 + free / 2.0) } else { (bw, bw) };
        let lx = x0 + 22.0;
        let rx = if bw >= 582.0 { lx + lw + 22.0 } else { lx };
        // left column
        let mut y = body_y;
        pg.text(lx, y, "SEEDS & FINAL MESSAGE", col::WHITE);
        y += 2.0 * LINE;
        let rows = |pg: &mut Pg<'_>, key: &str, y: f32, items: &[(String, usize)], on: &[bool], flip: &mut Option<usize>| -> f32 {
            let mut yy = y + 1.0 + 4.0;
            for (k, (name, size)) in items.iter().enumerate() {
                let (rx, ry) = (lx + 1.0 + 10.0 + 8.0, yy + 6.0);
                if pg.checkbox((key, k), rx, ry, on[k], true).clicked() {
                    *flip = Some(k);
                }
                let nx = rx + 4.0 * CH + 8.0;
                let nw = pg.text(nx, ry, name, col::T1);
                pg.text(nx + nw + 8.0, ry, &fmt(*size), col::T3);
                let rr = pg.rect(lx, yy, lw, 6.0 + LINE + 6.0);
                if pg.live && pg.ui.interact(rr, pg.wid((key, "row", k)), egui::Sense::click()).clicked() {
                    *flip = Some(k);
                }
                yy += 6.0 + LINE + 6.0 + if k + 1 < items.len() { 1.0 } else { 0.0 };
            }
            pg.fill(lx, y, lw, 0.0, col::BG);
            (yy + 4.0 + 1.0) - y
        };
        y += rows(pg, "seed", y, &seeds, &picks.0, &mut flip_seed);
        y += 16.0;
        if pg.checkbox("fm", lx, y, picks.2, true).clicked() {
            flip_fm = true;
        }
        pg.para(lx + 4.0 * CH, y, lw - 4.0 * CH, &[s("Include final message (as ", col::T2), code("final-message.txt"), s(")", col::T2)], Ws::Normal, WrapOpts::default(), LINE);
        y += LINE;
        pg.text(lx, y, "UPLOADS", col::WHITE);
        y += 2.0 * LINE;
        y += rows(pg, "up", y, &uploads, &picks.1, &mut flip_up);
        let left_end = y;
        // right column: summary box (padding 14, 1px border)
        let mut ry = if bw >= 582.0 { body_y } else { y + 22.0 };
        pg.text(rx, ry, "SUMMARY", col::WHITE);
        ry += 2.0 * LINE;
        let (sx, sw) = (rx + 15.0, rw - 30.0);
        let mut yy = ry + 15.0;
        yy += pg.para(sx, yy, sw, &[s("Total selected (raw): ", col::T2), s(format!("{total} ({})", fmt(total)), col::T1), s(" bytes", col::T2)], Ws::Normal, WrapOpts::default(), LINE) + 6.0;
        yy += pg.para(sx, yy, sw, &[s("Limit: 153,600 bytes (≈ 150 KB). Encryption adds overhead, but this raw cap keeps QR count reasonable.", col::T3)], Ws::Normal, WrapOpts::default(), LINE);
        yy += 12.0 + 1.0 + 12.0;
        if !error.is_empty() {
            pg.para(sx, yy, sw, &[s(&error, col::AMBER)], Ws::Normal, WrapOpts::default(), LINE);
        }
        yy += LINE.max(18.0) - 1.0 + 10.0;
        if pg.button("confirm", sx, yy, Btn::new("Build Paper Backup").main().box_w(sw)).clicked() {
            action = Some(0);
        }
        if pg.button("cancel", sx, yy + LINE, Btn::new("Cancel").box_w(sw)).clicked() {
            action = Some(1);
        }
        let right_end = yy + 2.0 * LINE + 14.0 + 1.0;
        left_end.max(right_end) + 22.0 + 1.0 + 8.0
    });
    if let Some(k) = flip_seed {
        st.picker.seeds[k] = !st.picker.seeds[k];
        st.picker.error.clear();
    }
    if let Some(k) = flip_up {
        st.picker.uploads[k] = !st.picker.uploads[k];
        st.picker.error.clear();
    }
    if flip_fm {
        st.picker.final_msg = !st.picker.final_msg;
        st.picker.error.clear();
    }
    match action {
        Some(0) => {
            if total == 0 {
                st.picker.error = "Select at least one item.".into();
            } else if total > cap {
                st.picker.error = format!("Too large. Reduce selection below {}.", fmt(cap));
            } else {
                build_lite(st, sh, ui.ctx());
            }
        }
        Some(1) => {
            st.label = "[i] Paper backup skipped by user. Proceeding to final instructions...".into();
            st.pct = 96;
            st.stage = Stage::Done;
            st.post = true;
        }
        _ => {}
    }
}

fn post_modal(st: &mut Build, sh: &mut Shared, ui: &mut egui::Ui) {
    let (t1, t2) = (col::T1, col::T2);
    let mut got_it = false;
    layer(ui, sh, "post", true, |pg, width| {
        let (x, w) = (9.0, width - 18.0);
        let p = |pg: &mut Pg<'_>, y: f32, spans: &[Span]| pg.para(x, y, w, spans, Ws::Normal, WrapOpts::default(), LINE);
        let mut y = 9.0;
        y += p(pg, y, &[s("Please Read Carefully", col::WHITE)]) + LINE;
        y += p(pg, y, &[s("Your Memoro Vault has been successfully built and downloaded.", t2)]) + 16.0;
        y += p(pg, y, &[s("To ensure it remains accessible for decades, it's critical to guard against ", t2), s("bit rot", t1), s(" — a slow form of data decay that can silently corrupt files over time.", t2)]) + 16.0;
        y += p(pg, y, &[s("Even if someone remembers all your answers perfectly, a single missing byte can render your vault irretrievable. That's why secure, redundant storage is essential.", t2)]) + 16.0;
        pg.text(x, y, "WHAT TO DO NEXT", col::WHITE);
        y += 2.0 * LINE;
        let (ox, ow) = (x + 22.0, w - 22.0);
        let (ux, uw) = (ox + 22.0, ow - 22.0);
        let items: Vec<(Vec<Span>, Vec<Vec<Span>>)> = vec![
            (vec![s("Create a folder (e.g., ", t2), code("J's_Memoro_Vault"), s(") on your computer.", t2)], vec![]),
            (vec![s("Place both of the following into that folder:", t2)], vec![
                vec![s("The downloaded vault ZIP file", t2)],
                vec![s("The Memoro Vault installer (e.g., ", t2), code("memoro-win.exe"), s(", ", t2), code(".AppImage"), s(", or ", t2), code(".dmg"), s(")", t2)],
            ]),
            (vec![s("Duplicate that folder to multiple destinations:", t1)], vec![
                vec![s("1× ", t2), s("M-DISC", t1), s(" (for long-term archival)", t2)],
                vec![s("1× ", t2), s("Cloud backup", t1), s(" (e.g., Proton Drive or Google Drive)", t2)],
                vec![s("1× ", t2), s("Offline USB/SSD", t1), s(" (stored in a fireproof safe or Faraday bag)", t2)],
            ]),
            (vec![s("Create a paper backup (recommended):", t1)], vec![
                vec![s("Memoro Vault can generate a printable PDF of QR codes that reassemble your encrypted vault.", t2)],
                vec![s("Print these QR tiles on archival paper and store them in a secure location.", t2)],
                vec![s("This ensures recovery even if all digital copies are lost or corrupted.", t2)],
            ]),
            (vec![s("Test access on another device while offline to verify integrity and function.", t2)], vec![]),
            (vec![s("Repeat this process whenever you update or rebuild your vault.", t2)], vec![]),
        ];
        for (k, (item, sub)) in items.iter().enumerate() {
            if k > 0 {
                y += 10.0;
            }
            y += li_num(pg, ox, y, ow, k + 1, item, col::T3);
            for sp in sub {
                y += 10.0;
                y += li(pg, ux, y, uw, sp, col::T3);
            }
        }
        y += 16.0;
        pg.text(x, y, "WHY M-DISC?", col::WHITE);
        y += 2.0 * LINE;
        y += p(pg, y, &[s("M-DISCs use a permanent, stone-like recording layer, unlike dye-based DVDs or flash memory. They're immune to bit rot, magnetic damage, EMPs, and most environmental degradation.", t2)]) + 16.0;
        y += p(pg, y, &[s("With proper storage, an M-DISC can last 100–1,000 years — making it ideal for preserving something irreplaceable.", t2)]) + 16.0;
        pg.text(x, y, "FINAL ADVICE", col::WHITE);
        y += 2.0 * LINE;
        for item in [
            vec![s("Do ", t2), s("not", t1), s(" rely on a single copy.", t2)],
            vec![s("Do ", t2), s("not", t1), s(" trust cloud providers to exist forever.", t2)],
            vec![s("Do ", t2), s("not", t1), s(" assume your future self will remember every step.", t2)],
        ] {
            y += li(pg, ox, y, ow, &item, col::T3) + 10.0;
        }
        y += 16.0 - 10.0;
        y += p(pg, y, &[s("Redundancy is key to ensuring your vault remains accessible for generations to come.", t2)]) + 16.0;
        y += p(pg, y, &[s("A copy of these instructions is also included inside your vault ZIP as ", t2), code("Vault_Builder_Instructions.txt"), s(".", t2)]) + 16.0;
        y += 26.0 - 16.0;
        got_it = pg.button("gotit", x, y, Btn::new("Got it")).clicked();
        y + LINE + 9.0
    });
    if got_it {
        // nukeEverything(), then the dashboard
        sh.flow.wipe();
        sh.nav = Some(crate::app::Nav::Replace(Page::Dashboard(Dashboard::default())));
        let _ = st;
    }
}
