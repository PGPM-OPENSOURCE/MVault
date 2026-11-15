//! create-vault.html: choose how many security questions (4-25), then Next.

use egui::{CornerRadius, Sense, vec2};

use crate::app::{Flow, Page, Shared, layer};
use crate::pages::common::button_row;
use crate::pages::setup::Setup;
use crate::ui::{Btn, LINE, Pg, col};

pub struct Create {
    pub count: usize,
}

impl Create {
    pub fn new(flow: &Flow) -> Create {
        Create { count: flow.custom_question_count.unwrap_or(14).clamp(4, 25) }
    }
}

/// `<input type="range">` in the console look: a 2px track and a 9x17 block thumb.
pub fn slider(pg: &mut Pg<'_>, key: &str, x: f32, y: f32, w: f32, value: &mut usize, min: usize, max: usize) -> bool {
    let id = pg.wid(key);
    let r = pg.rect(x, y, w, LINE);
    let resp = pg.ui.interact(r, id, if pg.live { Sense::click_and_drag() } else { Sense::hover() });
    let span = (max - min) as f32;
    let (tw, th) = (9.0, 17.0);
    let mut changed = false;
    if let Some(p) = resp.interact_pointer_pos() {
        if resp.dragged() || resp.clicked() || resp.drag_started() {
            let t = ((p.x - r.min.x - tw / 2.0) / (w - tw)).clamp(0.0, 1.0);
            let v = (min as f32 + (t * span).round()) as usize;
            if v != *value {
                *value = v;
                changed = true;
            }
        }
    }
    if resp.hovered() {
        pg.ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if resp.has_focus() {
        let (l, rt) = pg.ui.input(|i| (i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::ArrowDown), i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::ArrowUp)));
        if l && *value > min {
            *value -= 1;
            changed = true;
        }
        if rt && *value < max {
            *value += 1;
            changed = true;
        }
    }
    let p = pg.ui.painter().clone();
    let mid = r.center().y;
    p.rect_filled(egui::Rect::from_min_size(egui::pos2(r.min.x, mid - 1.0), vec2(w, 2.0)), CornerRadius::ZERO, col::T3);
    let t = (*value - min) as f32 / span;
    let tx = r.min.x + t * (w - tw);
    p.rect_filled(egui::Rect::from_min_size(egui::pos2(tx, mid - th / 2.0), vec2(tw, th)), CornerRadius::ZERO, col::T1);
    changed
}

pub fn show(st: &mut Create, sh: &mut Shared, ui: &mut egui::Ui) {
    let mut res = None;
    let mut count = st.count;
    layer(ui, sh, "create", true, |pg, width| {
        // .content keeps its 1px (invisible) border; the count is centred over the slider
        let (x, w) = (9.0, width - 18.0);
        let mut y = 9.0;
        y += pg.para(x, y, w, &[crate::ui::s("Select the number of security questions", col::WHITE)], crate::ui::Ws::Normal, Default::default(), LINE) + LINE;
        let label = count.to_string();
        let lw = pg.measure(&label, 16.0);
        pg.text(x + (w - lw) / 2.0, y, &label, col::WHITE);
        y += LINE + 8.0 + 2.0;
        slider(pg, "count", x, y, w, &mut count, 4, 25);
        y += LINE + 2.0 + 8.0 + 20.0;
        let (c, h) = button_row(pg, "nav", x, y, w, &[Btn::new("← Back"), Btn::new("Next").main()]);
        res = c;
        y + h + 1.0
    });
    st.count = count;
    match res {
        Some(0) => sh.nav = Some(crate::app::Nav::Back),
        Some(1) => {
            sh.flow.custom_question_count = Some(st.count);
            let setup = Setup::new(&mut sh.flow);
            sh.go(Page::Setup(setup));
        }
        _ => {}
    }
}
