//! license.html: the first-run licence gate. The licence (LICENSE.txt) in a 380px box with its own
//! scrollbar, then [Decline] [Accept], Accept bright.

use crate::app::{Shared, layer};
use crate::pages::common::button_row;
use crate::ui::{Btn, LINE, Ws, WrapOpts, col, s, wrap};

const TEXT: &str = include_str!("../../assets/LICENSE.txt");

#[derive(Default)]
pub struct License {
    /// the wrapped licence for one width: (width, lines)
    lines: Option<(f32, Vec<String>)>,
}

/// Some(true) once accepted, Some(false) when declined.
pub fn show(l: &mut License, sh: &mut Shared, ui: &mut egui::Ui) -> Option<bool> {
    let mut res = None;
    let ctx = ui.ctx().clone();
    layer(ui, sh, "license", true, |pg, width| {
        pg.text(8.0, 8.0, "License Agreement", col::WHITE);
        let bw = width - 16.0;
        // pre-wrap inside the box, beside its 6px scrollbar
        let tw = bw - 6.0;
        if l.lines.as_ref().is_none_or(|(w, _)| (*w - tw).abs() > 0.5) {
            let spans = [s(TEXT, col::T1)];
            let lines = wrap(&ctx, &spans, tw, Ws::PreWrap, WrapOpts::default()).into_iter()
                .map(|line| line.pieces.iter().map(|p| p.text.as_str()).collect::<String>())
                .collect();
            l.lines = Some((tw, lines));
        }
        let lines = &l.lines.as_ref().unwrap().1;
        pg.scroll_box("license-text", 8.0, 46.0, bw, 380.0, |pg, _w| {
            let clip = pg.ui.clip_rect();
            let top = clip.min.y - pg.o.y;
            let first = ((top / LINE).floor().max(0.0)) as usize;
            let last = (((clip.max.y - pg.o.y) / LINE).ceil().max(0.0) as usize + 1).min(lines.len());
            for (k, t) in lines.iter().enumerate().take(last).skip(first) {
                pg.text(0.0, k as f32 * LINE, t, col::T1);
            }
            lines.len() as f32 * LINE
        });
        let (c, h) = button_row(pg, "lb", 8.0, 46.0 + 380.0 + 19.0, bw, &[Btn::new("Decline"), Btn::new("Accept").main()]);
        res = c.map(|k| k == 1);
        46.0 + 380.0 + 19.0 + h + 8.0
    });
    res
}
