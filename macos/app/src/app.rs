//! App state, navigation between the pages, and the full-window layers they draw on.

use egui::{Pos2, Rect, vec2};

use crate::pages::{answers, build, create, dashboard, how, index, license, recover, setup};
use crate::snap::Snap;
use crate::ui::{Pg, Rec, col};

/// What the JS pages kept in localStorage / IndexedDB while a vault is being made (memory only).
#[derive(Default, Clone)]
pub struct Flow {
    pub custom_question_count: Option<usize>,
    pub questions: Option<Vec<String>>,
    pub question_count: Option<usize>,
    pub unlock_questions: Option<Vec<usize>>,
    /// answer-questions -> build-vault hand-off
    pub user_data: Option<mvault_core::build::UserData>,
    pub uploads: Vec<answers::UploadedFile>,
    pub named_seed_files: Vec<mvault_core::vault::Seed>,
    pub named_seeds: Vec<(String, String)>,
}

impl Flow {
    /// nukeEverything(): everything but the theme.
    pub fn wipe(&mut self) {
        *self = Flow::default();
    }
}

pub enum Nav {
    To(Page),
    Back,
    /// navigate and forget history (window.location.href after a wipe)
    Replace(Page),
}

pub struct Shared {
    pub rec: Rec,
    pub snap: Option<Snap>,
    pub flow: Flow,
    pub nav: Option<Nav>,
    pub time: f64,
}

impl Shared {
    pub fn go(&mut self, p: Page) {
        self.nav = Some(Nav::To(p));
    }
}

pub enum Page {
    Index(index::Index),
    Dashboard(dashboard::Dashboard),
    How(how::How),
    Create(create::Create),
    Setup(setup::Setup),
    Answers(answers::Answers),
    Build(build::Build),
    Recover(recover::Recover),
    /// the first-run licence gate (its own small window until Accept)
    License(license::License),
}

pub const LICENSE_TITLE: &str = "Memoro Vault — License Agreement";

impl Page {
    /// The page's <title>: Electron shows it in the window's title bar.
    pub fn title(&self) -> &'static str {
        match self {
            Page::Index(_) => "Memoro Vault",
            Page::Dashboard(_) => "Memoro Vault - Dashboard",
            Page::How(_) => "How Memoro Vault Works",
            Page::Create(_) => "Create New Vault - Memoro Vault",
            Page::Setup(_) => "Setup Your Security Questions - Memoro Vault",
            Page::Answers(_) => "Answer Your Questions - Memoro Vault",
            Page::Build(_) => "Building Your Vault - Memoro Vault",
            Page::Recover(_) => "Recover Your Vault - Memoro Vault",
            Page::License(_) => LICENSE_TITLE,
        }
    }
}

pub struct App {
    pub page: Page,
    pub history: Vec<Page>,
    pub sh: Shared,
    pub shown_title: &'static str,
    menu_installed: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, snap: Option<Snap>, license_gate: bool) -> App {
        crate::fonts::install(&cc.egui_ctx);
        crate::ui::install_style(&cc.egui_ctx);
        mvault_core::zxcvbn::warm_up_in_background();
        let mut sh = Shared { rec: Rec::default(), snap: None, flow: Flow::default(), nav: None, time: 0.0 };
        let page = match &snap {
            _ if license_gate => Page::License(license::License::default()),
            Some(s) => crate::snap::setup(&s.scenario, &mut sh),
            None => Page::Index(index::Index::default()),
        };
        sh.snap = snap;
        let shown_title = page.title();
        App { page, history: Vec::new(), sh, shown_title, menu_installed: false }
    }

    fn apply_nav(&mut self, ctx: &egui::Context) {
        let Some(nav) = self.sh.nav.take() else { return };
        // leaving the recover page by any way blanks the clipboard (recover.js `onLeave`)
        if matches!(self.page, Page::Recover(_)) {
            crate::pages::recover::wipe_clipboard();
        }
        match nav {
            Nav::To(p) => {
                let old = std::mem::replace(&mut self.page, p);
                self.history.push(old);
            }
            Nav::Replace(p) => {
                self.history.clear();
                self.page = p;
            }
            Nav::Back => {
                if let Some(p) = self.history.pop() {
                    self.page = p;
                }
            }
        }
        // every page starts with nothing focused, like a fresh document
        ctx.memory_mut(|m| m.request_focus(egui::Id::NULL));
        ctx.request_repaint();
    }
}

impl eframe::App for App {
    fn clear_color(&self, _v: &egui::Visuals) -> [f32; 4] {
        col::BG.to_normalized_gamma_f32()
    }

    /// Quitting from the recover page blanks the clipboard (recover.js `onLeave`), so a copied
    /// seed phrase does not outlive the session.
    fn on_exit(&mut self) {
        if matches!(self.page, Page::Recover(_)) {
            crate::pages::recover::wipe_clipboard();
        }
    }

    /// Edit-menu commands arrive as the events their shortcuts would send.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        #[cfg(target_os = "macos")]
        raw_input.events.extend(crate::macos::take_menu_events());
        let _ = raw_input;
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        #[cfg(target_os = "macos")]
        if !self.menu_installed {
            // after winit's own default menu, which this replaces
            self.menu_installed = true;
            crate::macos::install_menu(&ctx);
        }
        self.sh.time = ctx.input(|i| i.time);
        self.sh.rec.on = self.sh.snap.as_ref().is_some_and(|s| s.recording());
        self.sh.rec.clear();
        ui.painter().rect_filled(ui.max_rect(), 0.0, col::BG);
        match &mut self.page {
            Page::Index(p) => index::show(p, &mut self.sh, ui),
            Page::Dashboard(p) => dashboard::show(p, &mut self.sh, ui),
            Page::How(p) => how::show(p, &mut self.sh, ui),
            Page::Create(p) => create::show(p, &mut self.sh, ui),
            Page::Setup(p) => setup::show(p, &mut self.sh, ui),
            Page::Answers(p) => answers::show(p, &mut self.sh, ui),
            Page::Build(p) => build::show(p, &mut self.sh, ui),
            Page::Recover(p) => recover::show(p, &mut self.sh, ui),
            Page::License(p) => match license::show(p, &mut self.sh, ui) {
                Some(true) => {
                    crate::prefs::mark_license_accepted();
                    // the main window: 1000x700 with its title bar, centred, resizable
                    ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(1000.0, 672.0)));
                    if let Some(m) = ctx.input(|i| i.viewport().monitor_size) {
                        let pos = egui::pos2(((m.x - 1000.0) / 2.0).max(0.0), ((m.y - 700.0) / 2.0).max(0.0));
                        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos));
                    }
                    self.sh.nav = Some(Nav::Replace(Page::Index(index::Index::default())));
                }
                Some(false) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                None => {}
            },
        }
        crate::snap::tick(&mut self.sh, &ctx);
        self.apply_nav(&ctx);
        let title = self.page.title();
        if title != self.shown_title {
            self.shown_title = title;
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.into()));
        }
    }
}

// -------------------------------------------------------------------------------------------
// Layers
// -------------------------------------------------------------------------------------------
/// A full-window layer (the page, or a dialog that takes over the window) that scrolls
/// vertically with the console's thin square scrollbar. `draw` gets a painter whose (0, 0) is
/// the layer's top-left (scrolled) and the content width (the window, less the scrollbar when
/// the content overflows); it returns the content height.
pub fn layer(ui: &mut egui::Ui, sh: &mut Shared, key: &str, live: bool, draw: impl FnOnce(&mut Pg<'_>, f32) -> f32) {
    layer_over(ui, sh, key, None, live, draw)
}

/// Scrolls layer `key` to `y` on its next frame (`element.focus()` bringing something into view).
pub fn request_scroll(ctx: &egui::Context, key: &str, y: f32) {
    ctx.data_mut(|d| d.insert_temp(layer_id(key).with("want"), y));
}

/// Whether the page layer `key` has been laid out (its scroll geometry is known).
pub fn has_geom(ctx: &egui::Context, key: &str) -> bool {
    ctx.data(|d| d.get_temp::<(f32, f32, f32)>(layer_id(key).with("geom")).is_some())
}

fn layer_id(key: &str) -> egui::Id {
    egui::Id::NULL.with(("layer", key))
}

/// Like [`layer`], for a dialog that covers the page `under` without locking its scrolling: the
/// page's scrollbar stays visible (and the dialog 6px narrower) while the page overflows.
pub fn layer_over(ui: &mut egui::Ui, sh: &mut Shared, key: &str, under: Option<&str>, live: bool, draw: impl FnOnce(&mut Pg<'_>, f32) -> f32) {
    let id = layer_id(key);
    let mut full = ui.max_rect();
    if let Some(page) = under {
        let pid = layer_id(page);
        if let Some((off, content_h, view_h)) = ui.ctx().data(|d| d.get_temp::<(f32, f32, f32)>(pid.with("geom"))) {
            if content_h > view_h + 0.5 {
                let gutter = egui::Rect::from_min_max(egui::pos2(full.max.x - 6.0, full.min.y), full.max);
                let thumb_h = (view_h * view_h / content_h).max(18.0);
                let t = if content_h > view_h { off / (content_h - view_h) } else { 0.0 };
                let ty = gutter.min.y + t * (view_h - thumb_h);
                ui.painter().rect_filled(egui::Rect::from_min_size(egui::pos2(gutter.min.x, ty), vec2(6.0, thumb_h)), 0.0, col::LINE_HI);
                full.max.x -= 6.0;
            }
        }
    }
    if under.is_some() {
        // the dialog covers the page (it may have been drawn underneath this frame)
        ui.painter().rect_filled(full, 0.0, col::BG);
    }
    let overflow_id = id.with("overflow");
    let overflowing = ui.ctx().data(|d| d.get_temp::<bool>(overflow_id)).unwrap_or(false);
    let width = full.width() - if overflowing { 6.0 } else { 0.0 };
    let mut builder = ui.new_child(egui::UiBuilder::new().max_rect(full).id_salt(id));
    builder.set_clip_rect(full);
    let mut area = egui::ScrollArea::vertical()
        .id_salt(id)
        .auto_shrink([false, false])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded);
    if let Some(y) = ui.ctx().data_mut(|d| d.remove_temp::<f32>(id.with("want"))) {
        area = area.vertical_scroll_offset(y);
    }
    // snapshots: scrolled like the reference capture, once the content height is known
    if let Some(&(_, want)) = sh.snap.as_ref().and_then(|s| s.scroll.iter().find(|(k, _)| *k == key)) {
        if let Some((_, content_h, view_h)) = ui.ctx().data(|d| d.get_temp::<(f32, f32, f32)>(id.with("geom"))) {
            area = area.vertical_scroll_offset(want.min((content_h - view_h).max(0.0)));
        }
    }
    let out = area
        .show_viewport(&mut builder, |ui, _viewport| {
            let origin = ui.max_rect().min;
            let rec_on = sh.rec.on;
            let mut pg = Pg { ui, o: origin, rec: &mut sh.rec, live, id };
            pg.rec.on = rec_on;
            let h = draw(&mut pg, width);
            ui.allocate_rect(Rect::from_min_size(origin, vec2(width, h.max(1.0))), egui::Sense::hover());
            h
        });
    let h = out.inner;
    let over = h > full.height() + 0.5;
    ui.ctx().data_mut(|d| d.insert_temp(id.with("geom"), (out.state.clamped_offset().y, h, full.height())));
    if over != overflowing {
        ui.ctx().data_mut(|d| d.insert_temp(overflow_id, over));
        ui.ctx().request_repaint();
    }
}

/// A layer that never scrolls (fixed overlays).
pub fn fixed<'a>(ui: &'a mut egui::Ui, sh: &'a mut Shared, key: &str, live: bool) -> Pg<'a> {
    let id = egui::Id::NULL.with(("fixed", key));
    let origin: Pos2 = ui.max_rect().min;
    Pg { ui, o: origin, rec: &mut sh.rec, live, id }
}
