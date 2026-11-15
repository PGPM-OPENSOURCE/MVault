//! Restore Physical Backup (dashboard.html #paperDecryptModal, dashboard.js "Physical Backup
//! Scanner / Rebuilder"): read the MVKEY, MVHDR and MVCT codes from the printed PDF or photos of
//! its pages, rebuild memoro-lite.zip, save it and add it to the vault list.
//!
//! The camera (macOS AVFoundation) shows a live picture in the dialog and is searched for codes
//! every 300ms, or once with [Scan Key], like the original's getUserMedia loop.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use mvault_core::paper::PaperState;

use crate::app::{Shared, layer_over};
use crate::pages::common::{button_row, clock_now, code};
use crate::qrscan;
use crate::ui::{Btn, CH, LINE, Span, Ws, WrapOpts, col, s};

pub struct PaperModal {
    state: Arc<Mutex<PaperState>>,
    /// #paperStatus: "[time] message" lines
    log: Arc<Mutex<Vec<String>>>,
    /// what the file field says next to [Choose files]
    files_label: String,
    scanning: Arc<AtomicBool>,
    /// `window.__mvAutoRebuildDone`
    auto_done: Arc<AtomicBool>,
    /// set by the scan when the last piece arrives; the rebuild follows 50ms later
    ready_signal: Arc<AtomicBool>,
    rebuild_at: Option<f64>,
    close_at: Option<f64>,
    message: Option<(String, String)>,
    cam: Cam,
}

/// The camera part: running, or waiting on the system's permission prompt.
#[derive(Default)]
struct Cam {
    #[cfg(target_os = "macos")]
    running: Option<crate::macos::Camera>,
    asking: bool,
    tex: Option<egui::TextureHandle>,
    /// a frame search in flight (the original's `ticking` guard)
    busy: Arc<AtomicBool>,
    last_tick: f64,
}

impl Cam {
    fn on(&self) -> bool {
        #[cfg(target_os = "macos")]
        return self.running.is_some();
        #[cfg(not(target_os = "macos"))]
        return false;
    }

    /// `stopPaperCamera`
    fn stop(&mut self) {
        #[cfg(target_os = "macos")]
        {
            self.running = None;
        }
        self.asking = false;
        self.tex = None;
    }

    #[cfg(target_os = "macos")]
    fn latest(&self) -> Option<crate::macos::Frame> {
        self.running.as_ref().and_then(|c| c.latest())
    }
}

impl Default for PaperModal {
    /// `openPaperDecrypt()`
    fn default() -> Self {
        PaperModal {
            state: Arc::default(),
            log: Arc::new(Mutex::new(vec!["Waiting for uploads…".into()])),
            files_label: "No file chosen".into(),
            scanning: Arc::default(),
            auto_done: Arc::default(),
            ready_signal: Arc::default(),
            rebuild_at: None,
            close_at: None,
            message: None,
            cam: Cam::default(),
        }
    }
}

/// `setPaperMsg`
fn log(lines: &Mutex<Vec<String>>, msg: impl AsRef<str>) {
    if let Ok(mut l) = lines.lock() {
        l.push(format!("[{}] {}", clock_now(), msg.as_ref()));
    }
}

/// What a scan shares with the window.
#[derive(Clone)]
struct Scan {
    state: Arc<Mutex<PaperState>>,
    log: Arc<Mutex<Vec<String>>>,
    auto_done: Arc<AtomicBool>,
    ready_signal: Arc<AtomicBool>,
    ctx: Option<egui::Context>,
}

impl Scan {
    /// `ingestQR` + `updatePaperProgress`
    fn ingest(&self, raw: &str) {
        let Ok(mut st) = self.state.lock() else { return };
        if let Some(msg) = st.ingest(raw) {
            log(&self.log, msg);
        }
        if st.ready() && !self.auto_done.swap(true, Ordering::SeqCst) {
            log(&self.log, "All pieces captured — rebuilding ZIP automatically…");
            self.ready_signal.store(true, Ordering::SeqCst);
        }
        drop(st);
        if let Some(c) = &self.ctx {
            c.request_repaint();
        }
    }

    fn complete(&self) -> bool {
        self.state.lock().map(|s| s.ready()).unwrap_or(false)
    }

    /// `handlePaperFiles`, after the reset: every PDF page along the backup's grid, every
    /// picture searched all over, stopping once every piece is in.
    fn files(&self, paths: &[PathBuf]) {
        for p in paths {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let Ok(bytes) = std::fs::read(p) else {
                log(&self.log, format!("Could not read {name}."));
                continue;
            };
            if name.to_lowercase().ends_with(".pdf") || bytes.starts_with(b"%PDF") {
                match qrscan::pdf_open(&bytes) {
                    Ok(pdf) => {
                        for i in 0..pdf.pages().len() {
                            if self.complete() {
                                break;
                            }
                            if let Some(g) = qrscan::pdf_page(&pdf, i) {
                                qrscan::scan_pdf_page(&g, &mut |t| self.ingest(&t), &|| self.complete());
                            }
                        }
                    }
                    Err(_) => log(&self.log, format!("Could not read {name} as a PDF.")),
                }
            } else {
                let found = match image::load_from_memory(&bytes) {
                    Ok(img) => {
                        let g = qrscan::Gray::from_image(&img);
                        qrscan::scan_image(&g, &mut |t| self.ingest(&t))
                    }
                    Err(_) => 0,
                };
                if found == 0 {
                    log(&self.log, format!("No QR detected in {name}."));
                }
            }
            if let Some(c) = &self.ctx {
                c.request_repaint();
            }
        }
    }
}

impl PaperModal {
    /// `startPaperCamera`: the camera, or the message getUserMedia's failure gave.
    fn start_camera(&mut self, ctx: &egui::Context) {
        #[cfg(target_os = "macos")]
        match crate::macos::start_camera(ctx) {
            crate::macos::CameraStart::Running(c) => {
                self.cam.asking = false;
                self.cam.running = Some(c);
            }
            crate::macos::CameraStart::Asking => self.cam.asking = true,
            crate::macos::CameraStart::Failed(e) => {
                self.cam.asking = false;
                self.message = Some(("Camera Blocked".into(), format!("Couldn’t start camera. {e}\n\nCheck OS privacy settings and try again.")));
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = ctx;
            self.message = Some(("Camera Unavailable".into(), "This environment does not support camera access.".into()));
        }
    }

    /// One search of the newest camera frame for codes (`findAllQRCodesOnCanvas`), off the UI thread.
    fn scan_camera_frame(&mut self, ctx: &egui::Context) {
        #[cfg(target_os = "macos")]
        {
            if self.cam.busy.swap(true, Ordering::SeqCst) {
                return;
            }
            let Some(f) = self.cam.latest() else {
                self.cam.busy.store(false, Ordering::SeqCst);
                return;
            };
            let scan = self.scan(Some(ctx.clone()));
            let busy = self.cam.busy.clone();
            std::thread::spawn(move || {
                let g = qrscan::Gray { w: f.w, h: f.h, px: f.luma() };
                qrscan::scan_image(&g, &mut |t| scan.ingest(&t));
                busy.store(false, Ordering::SeqCst);
            });
        }
        #[cfg(not(target_os = "macos"))]
        let _ = ctx;
    }

    fn scan(&self, ctx: Option<egui::Context>) -> Scan {
        Scan { state: self.state.clone(), log: self.log.clone(), auto_done: self.auto_done.clone(), ready_signal: self.ready_signal.clone(), ctx }
    }

    /// A new set of files replaces whatever was read before.
    fn begin(&mut self, paths: &[PathBuf]) {
        if let Ok(mut st) = self.state.lock() {
            *st = PaperState::default();
        }
        if let Ok(mut l) = self.log.lock() {
            l.clear();
        }
        self.files_label = match paths {
            [one] => one.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            many => format!("{} files", many.len()),
        };
        log(&self.log, format!("Processing {} file(s)...", paths.len()));
        // the original never reset this, so a second set of files was read but not rebuilt
        self.auto_done.store(false, Ordering::SeqCst);
    }

    fn start(&mut self, paths: Vec<PathBuf>, ctx: &egui::Context) {
        if paths.is_empty() || self.scanning.load(Ordering::SeqCst) {
            return;
        }
        self.begin(&paths);
        let scan = self.scan(Some(ctx.clone()));
        let busy = self.scanning.clone();
        busy.store(true, Ordering::SeqCst);
        std::thread::spawn(move || {
            scan.files(&paths);
            busy.store(false, Ordering::SeqCst);
            if let Some(c) = &scan.ctx {
                c.request_repaint();
            }
        });
    }

    /// `rebuildLiteFromPaper`: decrypt and check the ZIP, offer it as memoro-lite.zip, add it to
    /// the vault list, close five seconds later.
    fn rebuild(&mut self, sh: &Shared, imported: &mut Option<(String, Vec<u8>)>, dialogs: bool) {
        let r = self.state.lock().map_err(|_| "busy".to_string()).and_then(|st| st.rebuild().map_err(|e| e.to_string()));
        match r {
            Ok(zip) => {
                let name = "memoro-lite.zip";
                if dialogs {
                    let _ = crate::pages::common::save_bytes(name, &zip, "ZIP", "zip");
                }
                log(&self.log, format!("Rebuild complete. Download started: {name}"));
                *imported = Some((name.to_string(), zip));
                log(&self.log, "ZIP auto-imported into your vault list.");
                self.cam.stop();
                self.close_at = Some(sh.time + 5.0);
            }
            Err(e) => {
                log(&self.log, format!("Rebuild failed: {e}"));
                if dialogs {
                    rfd::MessageDialog::new().set_title("Memoro Vault").set_description(format!("Rebuild failed: {e}")).set_buttons(rfd::MessageButtons::Ok).show();
                }
                self.auto_done.store(false, Ordering::SeqCst);
            }
        }
    }

    /// Snapshots: the reference capture's restore of its printed backup, done in place.
    pub fn snap_restore(&mut self, pdf: PathBuf, label: &str, sh: &Shared) -> Option<(String, Vec<u8>)> {
        let paths = vec![pdf];
        self.begin(&paths);
        self.files_label = label.into();
        self.scan(None).files(&paths);
        let mut imported = None;
        if self.ready_signal.swap(false, Ordering::SeqCst) {
            self.rebuild(sh, &mut imported, false);
        }
        self.close_at = None;
        imported
    }
}

pub fn show(pm: &mut PaperModal, sh: &mut Shared, ui: &mut egui::Ui, close: &mut bool, imported: &mut Option<(String, Vec<u8>)>) {
    let ctx = ui.ctx().clone();
    if pm.ready_signal.swap(false, Ordering::SeqCst) {
        pm.rebuild_at = Some(sh.time + 0.05);
    }
    if let Some(at) = pm.rebuild_at {
        if sh.time >= at {
            pm.rebuild_at = None;
            let dialogs = sh.snap.is_none();
            pm.rebuild(sh, imported, dialogs);
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(20));
        }
    }
    if let Some(at) = pm.close_at {
        if sh.time >= at {
            *close = true;
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
    if pm.scanning.load(Ordering::SeqCst) {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
    // the system's camera prompt answered: start (or report) now
    if pm.cam.asking {
        pm.start_camera(&ctx);
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
    // the scan loop: every 300ms while the camera runs
    if pm.cam.on() {
        if sh.time - pm.cam.last_tick >= 0.3 {
            pm.cam.last_tick = sh.time;
            pm.scan_camera_frame(&ctx);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
        #[cfg(target_os = "macos")]
        if let Some(f) = pm.cam.latest() {
            let img = egui::ColorImage::from_rgba_unmultiplied([f.w, f.h], &f.rgba());
            match &mut pm.cam.tex {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => pm.cam.tex = Some(ctx.load_texture("paper-cam", img, egui::TextureOptions::LINEAR)),
            }
        }
    }
    let cam_view = if pm.cam.on() { pm.cam.tex.as_ref().map(|t| (t.id(), t.size_vec2())) } else { None };
    let cam_on = pm.cam.on();
    // a message raised here shows over the restore dialog
    if let Some((t, m)) = pm.message.clone() {
        if crate::pages::dashboard::message_dialog(ui, sh, "paper-msg", &t, &m) {
            pm.message = None;
        }
        return;
    }
    let lines = pm.log.lock().map(|l| l.clone()).unwrap_or_default();
    let (status, pct, ready) = pm.state.lock().map(|st| (st.status_text(), st.percent(), st.ready())).unwrap_or_default();
    let files_label = pm.files_label.clone();
    let mut act: Option<&'static str> = None;
    layer_over(ui, sh, "paper", Some("dashboard"), true, |pg, width| {
        // .modal: a flex column, 14px apart
        let (x, w) = (9.0, width - 18.0);
        let mut y = 9.0;
        y += pg.para(x, y, w, &[s("Restore Physical Backup", col::WHITE)], Ws::Normal, WrapOpts::default(), LINE) + LINE + 14.0;
        let intro: [Vec<Span>; 4] = [
            vec![s("Upload the full PDF or individual page images. We’ll extract:", col::T2)],
            vec![s("• 1× ", col::T2), code("MVKEY"), s(" (single key QR)", col::T2)],
            vec![s("• 1× ", col::T2), code("MVHDR"), s(" (header) and all ", col::T2), code("MVCT"), s(" tiles (ciphertext)", col::T2)],
            vec![s("Then we rebuild ", col::T2), code("memoro-lite.zip"), s(".", col::T2)],
        ];
        for l in &intro {
            y += pg.para(x, y, w, l, Ws::Normal, WrapOpts::default(), LINE);
        }
        y += 14.0;
        // the file field: 10px padding, [Choose files] two spaces before what was picked
        let (bx, by) = (x + 11.0, y + 11.0);
        let bw = pg.measure("Choose files", 16.0);
        let field = pg.rect(x, y, w, 41.0);
        let hit = pg.ui.interact(field, pg.wid("paper-files"), egui::Sense::click());
        let over_btn = hit.hovered() && hit.hover_pos().is_some_and(|p| pg.rect(bx, by, bw, LINE).contains(p));
        if hit.hovered() {
            pg.ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if over_btn {
            pg.fill(bx, by, bw, LINE, col::FILL);
        }
        pg.text(bx, by, "Choose files", if over_btn { col::ON_FILL } else { col::T1 });
        let lw = (w - 22.0 - bw - 2.0 * CH).max(0.0);
        let label = if pg.measure(&files_label, 16.0) > lw { crate::pages::common::ellipsize(pg, &files_label, lw) } else { files_label.clone() };
        pg.text(bx + bw + 2.0 * CH, by, &label, col::T2);
        if hit.clicked() {
            act = Some("choose");
        }
        y += 41.0 + 12.0 + 14.0 + 4.0;
        // the camera box: 14px padding inside a 1px (unseen) border
        let (cx, cw) = (x + 15.0, w - 30.0);
        let mut cy = y + 15.0;
        cy += pg.para(cx, cy, cw, &[s("SCAN KEY SHARES (OPTIONAL)", col::WHITE)], Ws::Normal, WrapOpts::default(), LINE) + LINE;
        cy += pg.para(cx, cy, cw, &[s("If you prefer, aim the camera at an MVKEY QR (one at a time). We still recommend uploading a page/PDF for the tiles.", col::T2)], Ws::Normal, WrapOpts::default(), LINE) + 10.0;
        // #paperCam: the full width, at most 42% of the window tall, the picture fitted on black
        if cam_on {
            let max_h = (pg.ui.ctx().content_rect().height() * 0.42).floor();
            let (iw, ih) = cam_view.map(|(_, sz)| (sz.x, sz.y)).unwrap_or((16.0, 9.0));
            let vh = (cw * ih / iw).min(max_h);
            pg.fill(cx, cy, cw, vh, egui::Color32::BLACK);
            if let Some((tid, _)) = cam_view {
                let sc = (cw / iw).min(vh / ih);
                let (dw, dh) = (iw * sc, ih * sc);
                let r = pg.rect(cx + (cw - dw) / 2.0, cy + (vh - dh) / 2.0, dw, dh);
                pg.ui.painter().image(tid, r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
            }
            cy += vh;
        }
        let (c, h) = button_row(pg, "cam", cx, cy, cw, &[Btn::new("Start Camera"), Btn::new("Stop Camera"), Btn::new("Scan Key")]);
        if let Some(k) = c {
            act = Some(["start", "stop", "scan"][k]);
        }
        cy += h + 14.0 + 1.0;
        y = cy + 4.0 + 14.0;
        // progress: the status line and a 4px bar
        y += pg.para(x, y, w, &[s(&status, col::T2)], Ws::Normal, WrapOpts::default(), LINE) + 6.0;
        pg.fill(x, y, w, 4.0, col::LINE);
        pg.fill(x, y, w * pct as f32 / 100.0, 4.0, col::T1);
        y += 4.0 + 14.0;
        for l in &lines {
            y += pg.para(x, y, w, &[s(l, col::T2)], Ws::Normal, WrapOpts::default(), LINE);
        }
        y += 14.0 + 10.0;
        let (c, h) = button_row(pg, "pb", x, y, w, &[Btn::new("Close"), Btn::new("Rebuild Lite ZIP").main().enabled(ready)]);
        if let Some(k) = c {
            act = Some(["close", "rebuild"][k]);
        }
        y + h + 9.0
    });
    match act {
        Some("choose") => {
            let picked = rfd::FileDialog::new().add_filter("PDF or images", &["pdf", "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "heic"]).pick_files();
            if let Some(paths) = picked {
                pm.start(paths, &ctx);
            }
        }
        Some("start") => {
            pm.cam.stop();
            pm.start_camera(&ctx);
        }
        Some("stop") => pm.cam.stop(),
        Some("scan") => pm.scan_camera_frame(&ctx),
        Some("rebuild") => {
            let dialogs = sh.snap.is_none();
            pm.rebuild(sh, imported, dialogs);
        }
        Some("close") => {
            pm.cam.stop();
            *close = true;
        }
        _ => {}
    }
}
