//! dashboard.html: the four actions, loaded vault cards, unlock (with lockouts), rename/delete,
//! and Restore Physical Backup.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use serde_json::Value;

use mvault_core::vault::{self, Layer1, VaultJson};

use crate::app::{Page, Shared, layer, layer_over};
use crate::pages::common::{button_row, max_content};
use crate::pages::recover::{Handoff, Recover};
use crate::pages::{create::Create, how::How};
use crate::ui::{Btn, CH, InputOpts, LINE, Pg, Span, Ws, WrapOpts, col, s};

pub struct LoadedVault {
    pub id: u64,
    pub name: String,
    pub zip: Arc<Vec<u8>>,
    pub vault_json: VaultJson,
    pub vault_enc: Value,
}

#[derive(Default)]
pub struct Lockout {
    pub failed: u32,
    pub until: f64, // ms since epoch
    pub total: f64,
}

#[derive(Clone)]
pub enum Next {
    Create,
    Load,
}

pub struct Unlock {
    pub vault: u64,
    pub a1: String,
    pub a2: String,
    pub opening: bool,
    pub pending: Option<Receiver<Result<Layer1, String>>>,
    pub focus_once: bool,
}

#[derive(Default)]
pub struct Dashboard {
    pub vaults: Vec<LoadedVault>,
    pub menu: Option<u64>,
    pub offline: Option<Next>,
    pub unlock: Option<Unlock>,
    pub message: Option<(String, String)>,
    pub confirm: Option<(String, u64)>,
    pub rename: Option<(u64, String)>,
    pub paper: Option<crate::pages::paper::PaperModal>,
    pub lockouts: HashMap<u64, Lockout>,
    pub handoff_at: Option<(f64, Handoff)>,
}

pub fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
}

pub fn show(st: &mut Dashboard, sh: &mut Shared, ui: &mut egui::Ui) {
    poll_unlock(st, sh, ui.ctx());
    // dialogs take over the window; the topmost (DOM order) is what shows
    if st.paper.is_some() {
        let mut close = false;
        let mut imported = None;
        if let Some(pm) = &mut st.paper {
            crate::pages::paper::show(pm, sh, ui, &mut close, &mut imported);
        }
        if let Some((name, bytes)) = imported {
            import_bytes(st, name, bytes);
        }
        if close {
            st.paper = None;
        }
        if st.paper.is_some() {
            // a message raised while it is open (an import's "Success") shows over it; the
            // original stacked it underneath, where it stayed hidden until the dialog closed
            if st.message.is_some() {
                message_modal(st, sh, ui);
            }
            return;
        }
    }
    if st.rename.is_some() {
        return rename_modal(st, sh, ui);
    }
    if st.confirm.is_some() {
        return confirm_modal(st, sh, ui);
    }
    if st.message.is_some() {
        return message_modal(st, sh, ui);
    }
    if st.offline.is_some() {
        return offline_modal(st, sh, ui);
    }
    if st.unlock.is_some() {
        return unlock_modal(st, sh, ui);
    }
    page(st, sh, ui);
}

// -------------------------------------------------------------------------------------------
// The page
// -------------------------------------------------------------------------------------------
fn page(st: &mut Dashboard, sh: &mut Shared, ui: &mut egui::Ui) {
    let mut action: Option<usize> = None;
    let mut card_action: Option<(u64, &'static str)> = None;
    let menu = st.menu;
    let vaults: Vec<(u64, String)> = st.vaults.iter().map(|v| (v.id, v.name.clone())).collect();
    let mut menu_rect: Option<egui::Rect> = None;
    layer(ui, sh, "dashboard", true, |pg, width| {
        let w = width - 16.0;
        let btns = [Btn::new("+ Create New Vault").main(), Btn::new("Load Vault File"), Btn::new("Restore Physical Backup"), Btn::new("How Memoro Works")];
        let (clicked, row_h) = button_row(pg, "actions", 8.0, 8.0, w, &btns);
        action = clicked;
        let mut y = 8.0 + row_h;
        if vaults.is_empty() {
            return y + 8.0;
        }
        // .vault-list: grid repeat(auto-fit, minmax(280px, 1fr)), 12px gaps, 16px above
        y += 16.0;
        let gap = 12.0;
        let max_cols = (((w + gap) / (280.0 + gap)).floor() as usize).max(1);
        let cols = vaults.len().min(max_cols);
        let col_w = (w - gap * (cols as f32 - 1.0)) / cols as f32;
        let item_h = 98.0;
        let mut menu_draw: Option<(u64, f32, f32)> = None;
        for (i, (id, name)) in vaults.iter().enumerate() {
            let (c, r) = (i % cols, i / cols);
            let cx = 8.0 + c as f32 * (col_w + gap);
            let cy = y + r as f32 * (item_h + gap);
            let (x0, y0, inner) = (cx + 1.0, cy + 1.0, col_w - 2.0);
            let dots_w = pg.measure("[⋮]", 16.0);
            let title_max = (inner - dots_w).max(0.0);
            let tw = pg.measure(name, 16.0);
            let title_w = tw.min(title_max);
            let shown = if tw > title_max { crate::pages::common::ellipsize(pg, name, title_max) } else { name.clone() };
            pg.text(x0, y0, &shown, col::WHITE);
            let bx = x0 + title_w;
            let by = y0 + 9.5;
            if pg.button(("dots", *id), bx, by, Btn::new("⋮").color(col::T2)).clicked() {
                card_action = Some((*id, "menu"));
            }
            pg.text(x0, y0 + 48.0, "Status: Locked", col::T3);
            if pg.button(("unlock", *id), x0, y0 + 77.0, Btn::new("Unlock Vault").box_w(inner)).clicked() {
                card_action = Some((*id, "unlock"));
            }
            if menu == Some(*id) {
                menu_draw = Some((*id, bx + dots_w, by + LINE + 6.0));
            }
        }
        let rows = vaults.len().div_ceil(cols);
        let end = y + rows as f32 * item_h + (rows as f32 - 1.0) * gap + 12.0;
        // the open ⋮ menu floats above the cards
        if let Some((id, right, top)) = menu_draw {
            let (mx, mw, mh) = (right - 150.0, 150.0, 2.0 + 2.0 * LINE);
            pg.fill(mx, top, mw, mh, col::DIALOG);
            menu_rect = Some(pg.rect(mx, top, mw, mh));
            if pg.button(("rename", id), mx + 1.0 + CH, top + 1.0, Btn::new("Rename")).clicked() {
                card_action = Some((id, "rename"));
            }
            if pg.button(("delete", id), mx + 1.0 + CH, top + 1.0 + LINE, Btn::new("Delete").color(col::RED)).clicked() {
                card_action = Some((id, "delete"));
            }
        }
        end
    });
    // a click anywhere else closes the menu (dashboard.js document click handler)
    if st.menu.is_some() && ui.input(|i| i.pointer.primary_clicked()) && card_action.is_none() {
        let p = ui.input(|i| i.pointer.interact_pos());
        if !menu_rect.zip(p).is_some_and(|(r, p)| r.contains(p)) {
            st.menu = None;
        }
    }
    match action {
        Some(0) => st.offline = Some(Next::Create),
        Some(1) => st.offline = Some(Next::Load),
        Some(2) => st.paper = Some(crate::pages::paper::PaperModal::default()),
        Some(3) => sh.go(Page::How(How::default())),
        _ => {}
    }
    if let Some((id, what)) = card_action {
        match what {
            "menu" => st.menu = if st.menu == Some(id) { None } else { Some(id) },
            "unlock" => open_unlock(st, id),
            "rename" => {
                st.menu = None;
                if let Some(v) = st.vaults.iter().find(|v| v.id == id) {
                    st.rename = Some((id, v.name.clone()));
                }
            }
            "delete" => {
                st.menu = None;
                st.confirm = Some(("Are you sure you want to delete this vault?".into(), id));
            }
            _ => {}
        }
    }
}


// -------------------------------------------------------------------------------------------
// Dialogs (".modal-overlay > .modal": a flex column, 14px gaps, as wide as its widest line)
// -------------------------------------------------------------------------------------------
/// Input fields in a shrink-to-fit dialog contribute their intrinsic width (size=20).
pub const INPUT_INTRINSIC: f32 = 20.0 * CH + 2.0 * CH;

fn dialog_width(pg: &Pg<'_>, width: f32, blocks: &[Vec<Span>], extra: f32) -> f32 {
    let avail = width - 18.0;
    max_content(pg, blocks).max(extra).min(avail)
}

fn h2(pg: &mut Pg<'_>, x: f32, y: f32, w: f32, t: &str) -> f32 {
    pg.para(x, y, w, &[s(t, col::WHITE)], Ws::Normal, WrapOpts::default(), LINE) + LINE
}

fn message_modal(st: &mut Dashboard, sh: &mut Shared, ui: &mut egui::Ui) {
    let (title, text) = st.message.clone().unwrap();
    if message_dialog(ui, sh, "dash-message", &title, &text) {
        st.message = None;
    }
}

/// `showMessageModal(title, text)`; true once [OK] is pressed.
pub fn message_dialog(ui: &mut egui::Ui, sh: &mut Shared, key: &str, title: &str, text: &str) -> bool {
    let (title, text) = (title.to_string(), text.to_string());
    let mut ok = false;
    layer_over(ui, sh, key, Some("dashboard"), true, |pg, width| {
        let w = dialog_width(pg, width, &[vec![s(&title, col::WHITE)], text.split('\n').map(|l| s(l, col::T2)).collect()], pg.measure("[OK]", 16.0));
        let x = 9.0;
        let mut y = 9.0;
        y += h2(pg, x, y, w, &title) + 14.0;
        y += pg.para(x, y, w, &[s(&text, col::T2)], Ws::PreLine, WrapOpts::default(), LINE) + 14.0;
        if pg.button("ok", x, y, Btn::new("OK")).clicked() {
            ok = true;
        }
        y + LINE + 9.0
    });
    ok
}

fn confirm_modal(st: &mut Dashboard, sh: &mut Shared, ui: &mut egui::Ui) {
    let (text, id) = st.confirm.clone().unwrap();
    let mut res = None;
    layer_over(ui, sh, "dash-confirm", Some("dashboard"), true, |pg, width| {
        let bw = pg.measure("[No]  [Yes]", 16.0);
        let w = dialog_width(pg, width, &[vec![s("Confirm", col::WHITE)], vec![s(&text, col::T2)]], bw);
        let (x, mut y) = (9.0, 9.0);
        y += h2(pg, x, y, w, "Confirm") + 14.0;
        y += pg.para(x, y, w, &[s(&text, col::T2)], Ws::PreLine, WrapOpts::default(), LINE) + 14.0;
        let (c, h) = button_row(pg, "yn", x, y, w, &[Btn::new("No"), Btn::new("Yes").main()]);
        res = c;
        y + h + 9.0
    });
    match res {
        Some(0) => st.confirm = None,
        Some(1) => {
            st.confirm = None;
            st.vaults.retain(|v| v.id != id);
        }
        _ => {}
    }
}

fn rename_modal(st: &mut Dashboard, sh: &mut Shared, ui: &mut egui::Ui) {
    let mut res = None;
    let (id, mut value) = st.rename.clone().unwrap();
    layer_over(ui, sh, "dash-rename", Some("dashboard"), true, |pg, width| {
        let bw = pg.measure("[Rename]  [Cancel]", 16.0);
        let w = dialog_width(pg, width, &[vec![s("Rename Vault", col::WHITE)]], bw.max(INPUT_INTRINSIC));
        let (x, mut y) = (9.0, 9.0);
        y += h2(pg, x, y, w, "Rename Vault") + 14.0;
        pg.input("rename", x, y, w, &mut value, "New vault name", InputOpts::line());
        y += LINE + 14.0;
        let (c, h) = button_row(pg, "rb", x, y, w, &[Btn::new("Rename").main(), Btn::new("Cancel")]);
        res = c;
        y + h + 9.0
    });
    st.rename = Some((id, value.clone()));
    match res {
        Some(0) => {
            let name = mvault_core::jsstr::js_trim(&value).to_string();
            if name.is_empty() {
                st.message = Some(("Missing Name".into(), "Please enter a new name.".into()));
                return;
            }
            if let Some(v) = st.vaults.iter_mut().find(|v| v.id == id) {
                v.name = name;
            }
            st.rename = None;
        }
        Some(1) => st.rename = None,
        _ => {}
    }
}

fn offline_modal(st: &mut Dashboard, sh: &mut Shared, ui: &mut egui::Ui) {
    let text = "We recommend disabling your internet connection temporarily for maximum security.";
    let mut res = None;
    layer_over(ui, sh, "dash-offline", Some("dashboard"), true, |pg, width| {
        let w = dialog_width(pg, width, &[vec![s("Offline Mode Recommended", col::WHITE)], vec![s(text, col::T2)]], 0.0);
        let (x, mut y) = (9.0, 9.0);
        y += h2(pg, x, y, w, "Offline Mode Recommended") + 14.0;
        y += pg.para(x, y, w, &[s(text, col::T2)], Ws::Normal, WrapOpts::default(), LINE) + 14.0;
        let (c, h) = button_row(pg, "ob", x, y, w, &[Btn::new("Continue").main(), Btn::new("Cancel")]);
        res = c;
        y + h + 9.0
    });
    match res {
        Some(0) => {
            let next = st.offline.take().unwrap();
            match next {
                Next::Create => sh.go(Page::Create(Create::new(&sh.flow))),
                Next::Load => load_vault_file(st),
            }
        }
        Some(1) => st.offline = None,
        _ => {}
    }
}

// -------------------------------------------------------------------------------------------
// Loading a vault ZIP
// -------------------------------------------------------------------------------------------
fn load_vault_file(st: &mut Dashboard) {
    let Some(path) = rfd::FileDialog::new().add_filter("ZIP", &["zip"]).pick_file() else { return };
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if !name.to_lowercase().ends_with(".zip") {
        st.message = Some(("Invalid File".into(), "Please select a valid .zip vault file.".into()));
        return;
    }
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if size > vault::MAX_VAULT_BYTES {
        st.message = Some(("Vault Too Large".into(), "This file is larger than the 1.9 GB Memoro Vault can open.".into()));
        return;
    }
    match std::fs::read(&path) {
        Ok(bytes) => import_bytes(st, name, bytes),
        Err(e) => st.message = Some(("Load Failed".into(), e.to_string())),
    }
}

pub fn import_bytes(st: &mut Dashboard, file_name: String, bytes: Vec<u8>) {
    match vault::peek_vault_zip(&bytes) {
        Ok((vj, enc, _meta)) => {
            let mut id = now_ms() as u64;
            while st.vaults.iter().any(|v| v.id == id) {
                id += 1;
            }
            st.vaults.push(LoadedVault { id, name: vault::strip_zip_ext(&file_name), zip: Arc::new(bytes), vault_json: vj, vault_enc: enc });
            st.message = Some(("Success".into(), "Vault imported successfully!".into()));
        }
        Err(e) => st.message = Some(("Load Failed".into(), e.to_string())),
    }
}

// -------------------------------------------------------------------------------------------
// Unlock
// -------------------------------------------------------------------------------------------
fn open_unlock(st: &mut Dashboard, id: u64) {
    st.unlock = Some(Unlock { vault: id, a1: String::new(), a2: String::new(), opening: false, pending: None, focus_once: true });
}

fn submit_unlock(st: &mut Dashboard, ctx: &egui::Context) {
    let Some(u) = &mut st.unlock else { return };
    let lock = st.lockouts.entry(u.vault).or_default();
    if now_ms() < lock.until || u.pending.is_some() || u.opening {
        return;
    }
    let a1 = mvault_core::jsstr::normalize_answer(&u.a1);
    let a2 = mvault_core::jsstr::normalize_answer(&u.a2);
    if a1.is_empty() || a2.is_empty() {
        st.message = Some(("Missing Answers".into(), "Please answer both questions.".into()));
        return;
    }
    let Some(v) = st.vaults.iter().find(|v| v.id == u.vault) else {
        st.message = Some(("Vault Error".into(), "Vault is missing required data.".into()));
        return;
    };
    let (vj, enc) = (v.vault_json.clone(), v.vault_enc.clone());
    let (tx, rx) = channel();
    let ctx2 = ctx.clone();
    std::thread::spawn(move || {
        let concat = zeroize::Zeroizing::new(format!("{a1}{a2}"));
        let r = vault::decrypt_layer1(&concat, &vj, &enc).map_err(|e| e.to_string());
        let _ = tx.send(r);
        ctx2.request_repaint();
    });
    u.pending = Some(rx);
}

fn poll_unlock(st: &mut Dashboard, sh: &mut Shared, ctx: &egui::Context) {
    if let Some((at, _)) = &st.handoff_at {
        if sh.time >= *at {
            let (_, h) = st.handoff_at.take().unwrap();
            sh.nav = Some(crate::app::Nav::To(Page::Recover(Recover::new(h))));
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
    let Some(u) = &mut st.unlock else { return };
    let Some(rx) = &u.pending else { return };
    let Ok(r) = rx.try_recv() else {
        ctx.request_repaint_after(std::time::Duration::from_millis(30));
        return;
    };
    u.pending = None;
    let id = u.vault;
    match r {
        Ok(l1) => {
            u.opening = true;
            let v = st.vaults.iter().find(|v| v.id == id).unwrap();
            let unlock_concat = format!("{}{}", mvault_core::jsstr::normalize_answer(&u.a1), mvault_core::jsstr::normalize_answer(&u.a2));
            st.lockouts.remove(&id);
            st.handoff_at = Some((sh.time + 0.5, Handoff { zip: v.zip.clone(), vault_json: v.vault_json.clone(), layer1: l1, unlock_concat: zeroize::Zeroizing::new(unlock_concat) }));
        }
        Err(_) => {
            let lock = st.lockouts.entry(id).or_default();
            lock.failed += 1;
            let delay = 15000.0 * 2f64.powi(lock.failed as i32 - 1);
            lock.until = now_ms() + delay;
            lock.total = delay;
            st.message = Some(("Unlock Failed".into(), format!("Incorrect answers. Attempts: {}", lock.failed)));
        }
    }
}

fn unlock_modal(st: &mut Dashboard, sh: &mut Shared, ui: &mut egui::Ui) {
    let id = st.unlock.as_ref().unwrap().vault;
    let prompts = st.vaults.iter().find(|v| v.id == id).map(|v| v.vault_json.question_prompts()).unwrap_or_default();
    let q1 = prompts.first().cloned().unwrap_or_else(|| "Question 1:".into());
    let q2 = prompts.get(1).cloned().unwrap_or_else(|| "Question 2:".into());
    let lock = st.lockouts.get(&id).map(|l| (l.until, l.total)).unwrap_or((0.0, 0.0));
    let now = now_ms();
    let locked = now < lock.0;
    if locked {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }
    let mut res = None;
    let u = st.unlock.as_mut().unwrap();
    let busy = u.opening;
    let mut a1 = std::mem::take(&mut u.a1);
    let mut a2 = std::mem::take(&mut u.a2);
    let focus_once = std::mem::replace(&mut u.focus_once, false);
    layer_over(ui, sh, "dash-unlock", Some("dashboard"), true, |pg, width| {
        let bw = pg.measure("[Unlock]  [Cancel]", 16.0);
        let w = dialog_width(pg, width, &[vec![s("Unlock Vault", col::WHITE)], vec![s(&q1, col::WHITE)], vec![s(&q2, col::WHITE)]], bw.max(INPUT_INTRINSIC));
        let (x, mut y) = (9.0, 9.0);
        y += h2(pg, x, y, w, "Unlock Vault") + 14.0;
        y += h2(pg, x, y, w, &q1) + 14.0;
        let o1 = pg.input("a1", x, y, w, &mut a1, "Answer 1", InputOpts { disabled: locked, ..InputOpts::lower() });
        if focus_once {
            o1.response.request_focus();
        }
        y += LINE + 14.0;
        y += h2(pg, x, y, w, &q2) + 14.0;
        pg.input("a2", x, y, w, &mut a2, "Answer 2", InputOpts { disabled: locked, ..InputOpts::lower() });
        y += LINE + 14.0;
        if locked {
            let remaining = lock.0 - now;
            let secs = (remaining / 1000.0).ceil();
            y += 10.0 + 5.0;
            pg.text_sized(x, y, &format!("Locked for {secs}s"), 13.0, col::AMBER);
            y += LINE + 5.0;
            pg.fill(x, y, w, 4.0, col::LINE);
            pg.fill(x, y, w * (remaining / lock.1).clamp(0.0, 1.0) as f32, 4.0, col::T1);
            y += 4.0 + 14.0;
        }
        let label = if locked { "Locked" } else if busy { "Opening..." } else { "Unlock" };
        let (c, h) = button_row(pg, "ub", x, y, w, &[Btn::new(label).main().enabled(!locked && !busy), Btn::new("Cancel")]);
        res = c;
        y + h + 9.0
    });
    let u = st.unlock.as_mut().unwrap();
    u.a1 = a1;
    u.a2 = a2;
    match res {
        Some(0) => submit_unlock(st, ui.ctx()),
        Some(1) => st.unlock = None,
        _ => {}
    }
}
