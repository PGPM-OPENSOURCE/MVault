//! The PGPM console toolkit: one monospace font on 19px lines, text set from the top-left,
//! buttons as bracketed words that invert under the pointer, "[ ]" checkboxes, dim input lines.
//! Coordinates are CSS pixels (= egui points). Everything drawn can be recorded for snapshot
//! comparison with the Electron reference dumps.

use std::fmt::Debug;
use std::hash::Hash;

use egui::{Color32, CornerRadius, FontFamily, FontId, Id, Pos2, Rect, Response, Sense, Stroke, Vec2, pos2, vec2};
use serde_json::{Value, json};

use crate::fonts;

// -------------------------------------------------------------------------------------------
// PGPM palette ("Campbell" console colours) and metrics
// -------------------------------------------------------------------------------------------
pub mod col {
    use egui::Color32;
    pub const BG: Color32 = Color32::from_rgb(0x0c, 0x0c, 0x0c);
    pub const RAISED: Color32 = Color32::from_rgb(0x12, 0x12, 0x12);
    pub const INPUT: Color32 = Color32::from_rgb(0x1a, 0x1a, 0x1a);
    pub const INPUT_FOCUS: Color32 = Color32::from_rgb(0x26, 0x26, 0x26);
    pub const DIALOG: Color32 = Color32::from_rgb(0x16, 0x16, 0x16);
    pub const BTN: Color32 = Color32::from_rgb(0x26, 0x26, 0x26);
    pub const FILL: Color32 = Color32::from_rgb(0xcc, 0xcc, 0xcc);
    pub const ON_FILL: Color32 = Color32::from_rgb(0x0c, 0x0c, 0x0c);
    pub const LINE: Color32 = Color32::from_rgb(0x3a, 0x3a, 0x3a);
    pub const LINE_HI: Color32 = Color32::from_rgb(0x76, 0x76, 0x76);
    pub const T1: Color32 = Color32::from_rgb(0xcc, 0xcc, 0xcc);
    pub const T2: Color32 = Color32::from_rgb(0xa8, 0xa8, 0xa8);
    pub const T3: Color32 = Color32::from_rgb(0x76, 0x76, 0x76);
    pub const T4: Color32 = Color32::from_rgb(0x4a, 0x4a, 0x4a);
    pub const WHITE: Color32 = Color32::from_rgb(0xf2, 0xf2, 0xf2);
    pub const RED: Color32 = Color32::from_rgb(0xe7, 0x48, 0x56);
    pub const AMBER: Color32 = Color32::from_rgb(0xc1, 0x9c, 0x00);
    pub const MID: Color32 = Color32::from_rgb(0xf9, 0xf1, 0xa5);
}

/// One console line.
pub const LINE: f32 = 19.0;
/// Cascadia Mono advance at 16px.
pub const CH: f32 = 9.375;

pub fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

/// Baseline from the top of a line box, as Blink places it: ascent and descent rounded to whole
/// pixels, the leftover leading split above and below.
pub fn baseline(size: f32, line_h: f32) -> f32 {
    let a = (fonts::ASCENT / fonts::UPEM * size).round();
    let d = (fonts::DESCENT / fonts::UPEM * size).round();
    (line_h - (a + d)) / 2.0 + a
}

pub fn css(c: Color32) -> String {
    format!("rgb({}, {}, {})", c.r(), c.g(), c.b())
}

// -------------------------------------------------------------------------------------------
// Snapshot recorder (same shape as reference/capture dumps)
// -------------------------------------------------------------------------------------------
#[derive(Default)]
pub struct Rec {
    pub on: bool,
    pub texts: Vec<Value>,
    pub boxes: Vec<Value>,
}

impl Rec {
    pub fn clear(&mut self) {
        self.texts.clear();
        self.boxes.clear();
    }
}

// -------------------------------------------------------------------------------------------
// Inline text with styles, and CSS-like wrapping
// -------------------------------------------------------------------------------------------
#[derive(Clone, Debug)]
pub struct Span {
    pub text: String,
    pub color: Color32,
    pub size: f32,
    /// inline box padding (left and right), e.g. `code { padding: 1px 5px }`
    pub pad_x: f32,
    /// inline box padding (top and bottom)
    pub pad_y: f32,
    pub bg: Option<Color32>,
    pub underline: bool,
    /// margin after the inline box (e.g. `b { margin-right: 4px }`)
    pub margin_r: f32,
}

impl Span {
    pub fn new(text: impl Into<String>, color: Color32) -> Span {
        Span { text: text.into(), color, size: 16.0, pad_x: 0.0, pad_y: 0.0, bg: None, underline: false, margin_r: 0.0 }
    }
    pub fn margin_r(mut self, m: f32) -> Span {
        self.margin_r = m;
        self
    }
    pub fn sized(mut self, size: f32) -> Span {
        self.size = size;
        self
    }
    pub fn code(mut self, bg: Color32, pad_x: f32, pad_y: f32) -> Span {
        self.bg = Some(bg);
        self.pad_x = pad_x;
        self.pad_y = pad_y;
        self
    }
    pub fn underline(mut self) -> Span {
        self.underline = true;
        self
    }
}

pub fn s(text: impl Into<String>, color: Color32) -> Span {
    Span::new(text, color)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ws {
    /// collapse spaces, wrap
    Normal,
    /// keep spaces and newlines, wrap
    PreWrap,
    /// keep spaces and newlines, never wrap
    Pre,
    /// keep newlines, collapse spaces, wrap
    PreLine,
    /// collapse spaces, never wrap
    NoWrap,
}

#[derive(Clone, Copy, Default)]
pub struct WrapOpts {
    /// overflow-wrap: anywhere / word-break: break-word
    pub anywhere: bool,
}

/// One laid-out line: pieces of spans with their x offsets.
#[derive(Clone, Debug, Default)]
pub struct Line {
    pub pieces: Vec<Piece>,
    pub width: f32,
}

#[derive(Clone, Debug)]
pub struct Piece {
    pub span: usize,
    pub text: String,
    pub x: f32,
    pub w: f32,
    /// this piece starts / ends its span (inline box padding is drawn there)
    pub first: bool,
    pub last: bool,
}

struct Item {
    span: usize,
    ch: char,
    w: f32,
}

/// Prepares the text of each span per `white-space` (collapsing runs of spaces across spans).
fn prepare(spans: &[Span], ws: Ws) -> Vec<String> {
    let mut out = Vec::with_capacity(spans.len());
    let mut prev_space = true; // a line starts collapsed
    for sp in spans {
        let mut t = String::with_capacity(sp.text.len());
        for c in sp.text.chars() {
            match ws {
                Ws::Normal | Ws::NoWrap => {
                    if c == ' ' || c == '\n' || c == '\t' || c == '\r' {
                        if !prev_space {
                            t.push(' ');
                            prev_space = true;
                        }
                    } else {
                        t.push(c);
                        prev_space = false;
                    }
                }
                Ws::PreLine => {
                    if c == '\n' {
                        while t.ends_with(' ') { t.pop(); }
                        t.push('\n');
                        prev_space = true;
                    } else if c == ' ' || c == '\t' || c == '\r' {
                        if !prev_space {
                            t.push(' ');
                            prev_space = true;
                        }
                    } else {
                        t.push(c);
                        prev_space = false;
                    }
                }
                Ws::PreWrap | Ws::Pre => {
                    if c == '\t' { t.push_str("        "); } else if c != '\r' { t.push(c); }
                }
            }
        }
        out.push(t);
    }
    out
}

/// Lays text out in lines no wider than `max_w`, breaking where UAX #14 allows (as Blink does).
pub fn wrap(ctx: &egui::Context, spans: &[Span], max_w: f32, ws: Ws, opts: WrapOpts) -> Vec<Line> {
    let texts = prepare(spans, ws);
    // flatten to items with widths
    let mut items: Vec<Item> = Vec::new();
    let mut joined = String::new();
    let mut byte_to_item: Vec<usize> = Vec::new();
    ctx.fonts_mut(|f| {
        for (si, t) in texts.iter().enumerate() {
            let fid = font(spans[si].size);
            for c in t.chars() {
                let w = if c == '\n' { 0.0 } else { f.glyph_width(&fid, c) };
                for _ in 0..c.len_utf8() {
                    byte_to_item.push(items.len());
                }
                joined.push(c);
                items.push(Item { span: si, ch: c, w });
            }
        }
    });
    byte_to_item.push(items.len());
    let n = items.len();
    // break opportunities: allowed[k] => a line may start at item k
    let mut allowed = vec![false; n + 1];
    let mut mandatory = vec![false; n + 1];
    let wraps = !matches!(ws, Ws::Pre | Ws::NoWrap);
    for (b, op) in unicode_linebreak::linebreaks(&joined) {
        let k = byte_to_item[b];
        match op {
            unicode_linebreak::BreakOpportunity::Mandatory => {
                if matches!(ws, Ws::PreWrap | Ws::Pre | Ws::PreLine) {
                    mandatory[k] = true;
                }
                allowed[k] = wraps;
            }
            unicode_linebreak::BreakOpportunity::Allowed => {
                // Chromium (ICU) keeps "page/PDF" and paths together: no break after a solidus
                // that a letter or digit follows
                let after_slash = k > 0 && items[k - 1].ch == '/' && items.get(k).is_some_and(|i| i.ch.is_alphanumeric());
                allowed[k] = wraps && !after_slash;
            }
        }
    }
    let pad = |si: usize| spans[si].pad_x;
    let mr = |si: usize| spans[si].margin_r;
    let mut lines = Vec::new();
    let mut start = 0usize;
    while start < n || (start == n && lines.is_empty()) {
        if start == n {
            lines.push(Line::default());
            break;
        }
        // skip collapsible spaces at a soft line start
        let mut s0 = start;
        if matches!(ws, Ws::Normal | Ws::NoWrap | Ws::PreLine) {
            while s0 < n && items[s0].ch == ' ' {
                s0 += 1;
            }
        }
        // greedy: furthest break point that fits (trailing spaces hang)
        let mut w = 0.0;
        let mut last_fit: Option<usize> = None;
        let mut k = s0;
        let mut end = n;
        while k < n {
            if items[k].ch == '\n' {
                end = k;
                break;
            }
            if k > s0 && mandatory[k] {
                end = k;
                break;
            }
            let si = items[k].span;
            let first_of_span = k == s0 || items[k - 1].span != si;
            let last_of_span = k + 1 == n || items[k + 1].span != si;
            let add = items[k].w + if first_of_span { pad(si) } else { 0.0 } + if last_of_span { pad(si) + mr(si) } else { 0.0 };
            // width up to k+1 not counting trailing spaces
            let new_w = w + add;
            let visible_w = {
                let mut j = k + 1;
                let mut vw = new_w;
                while j > s0 && items[j - 1].ch == ' ' {
                    vw -= items[j - 1].w;
                    j -= 1;
                }
                vw
            };
            if k + 1 <= n && allowed[k + 1] && k + 1 > s0 && visible_w <= max_w + 0.01 {
                last_fit = Some(k + 1);
            }
            if visible_w > max_w + 0.01 && wraps {
                if let Some(lf) = last_fit {
                    end = lf;
                } else if opts.anywhere && k > s0 {
                    end = k;
                } else {
                    // overflow until the next break opportunity
                    let mut j = k + 1;
                    while j < n && !allowed[j] && items[j].ch != '\n' {
                        j += 1;
                    }
                    end = j;
                }
                break;
            }
            w = new_w;
            k += 1;
        }
        // build the line s0..end (drop trailing collapsible spaces)
        let mut e = end;
        if matches!(ws, Ws::Normal | Ws::NoWrap | Ws::PreLine | Ws::PreWrap) {
            while e > s0 && items[e - 1].ch == ' ' {
                e -= 1;
            }
        }
        let mut line = Line::default();
        let mut x = 0.0;
        let mut i = s0;
        while i < e {
            let si = items[i].span;
            let mut j = i;
            let mut text = String::new();
            let mut pw = 0.0;
            while j < e && items[j].span == si {
                text.push(items[j].ch);
                pw += items[j].w;
                j += 1;
            }
            let first = i == 0 || items[i - 1].span != si || i == s0;
            let span_starts_here = i == 0 || items[i - 1].span != si;
            let span_ends_here = j == n || items[j].span != si;
            if span_starts_here {
                x += pad(si);
            }
            line.pieces.push(Piece { span: si, text, x, w: pw, first: span_starts_here || first, last: span_ends_here });
            x += pw;
            if span_ends_here {
                x += pad(si) + mr(si);
            }
            i = j;
        }
        line.width = x;
        lines.push(line);
        // next line
        start = end;
        if start < n && items[start].ch == '\n' {
            start += 1;
            if start == n {
                lines.push(Line::default());
            }
        }
    }
    lines
}

pub fn measure(ctx: &egui::Context, text: &str, size: f32) -> f32 {
    let fid = font(size);
    ctx.fonts_mut(|f| text.chars().map(|c| f.glyph_width(&fid, c)).sum())
}

pub fn text_transform_upper(s: &str) -> String {
    s.to_uppercase()
}

// -------------------------------------------------------------------------------------------
// The painter for one layer (page or full-window dialog)
// -------------------------------------------------------------------------------------------
pub struct Pg<'a> {
    pub ui: &'a mut egui::Ui,
    /// screen position of this layer's (0, 0) (moves with scrolling)
    pub o: Pos2,
    pub rec: &'a mut Rec,
    /// false while something above this layer takes the pointer
    pub live: bool,
    pub id: Id,
}

#[derive(Clone, Copy)]
pub struct Btn<'s> {
    pub label: &'s str,
    pub color: Color32,
    /// the button's box when wider than its text (e.g. stretched in a flex column)
    pub box_w: Option<f32>,
    pub enabled: bool,
    pub size: f32,
    /// a chosen option: shown inverted, like a console selection
    pub selected: bool,
}

impl<'s> Btn<'s> {
    pub fn new(label: &'s str) -> Btn<'s> {
        Btn { label, color: col::T1, box_w: None, enabled: true, size: 16.0, selected: false }
    }
    pub fn selected(mut self, on: bool) -> Self {
        self.selected = on;
        self
    }
    pub fn main(mut self) -> Self {
        self.color = col::WHITE;
        self
    }
    pub fn color(mut self, c: Color32) -> Self {
        self.color = c;
        self
    }
    pub fn box_w(mut self, w: f32) -> Self {
        self.box_w = Some(w);
        self
    }
    pub fn enabled(mut self, e: bool) -> Self {
        self.enabled = e;
        self
    }
}

impl<'a> Pg<'a> {
    pub fn ctx(&self) -> egui::Context {
        self.ui.ctx().clone()
    }

    pub fn at(&self, x: f32, y: f32) -> Pos2 {
        self.o + vec2(x, y)
    }

    pub fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(self.at(x, y), vec2(w, h))
    }

    pub fn wid(&self, key: impl Hash + Debug) -> Id {
        self.id.with(key)
    }

    pub fn fill(&mut self, x: f32, y: f32, w: f32, h: f32, c: Color32) {
        let r = self.rect(x, y, w, h);
        self.ui.painter().rect_filled(r, CornerRadius::ZERO, c);
        if self.rec.on {
            self.rec.boxes.push(json!({"x": r.min.x, "y": r.min.y, "w": w, "h": h, "bg": css(c)}));
        }
    }

    /// One line of text whose line box (19px unless `line_h`) has its top at `y`. Returns the width.
    pub fn text_lh(&mut self, x: f32, y: f32, t: &str, size: f32, color: Color32, line_h: f32) -> f32 {
        if t.is_empty() {
            return 0.0;
        }
        let galley = self.ui.fonts_mut(|f| f.layout_no_wrap(t.to_owned(), font(size), color));
        let b_galley = galley.rows.first().and_then(|r| r.row.glyphs.first().map(|g| r.pos.y + g.pos.y)).unwrap_or(0.0);
        let pos = self.at(x, y) + vec2(0.0, baseline(size, line_h) - b_galley);
        let w = galley.rect.width();
        self.ui.painter().galley(pos, galley, color);
        if self.rec.on {
            // Range rects (what the reference dumps hold) start at the text's content area
            let asc = (fonts::ASCENT / fonts::UPEM * size).round();
            let p = self.at(x, y + baseline(size, line_h) - asc);
            self.rec.texts.push(json!({"t": t, "x": p.x, "y": p.y, "w": w, "h": line_h, "c": css(color), "fs": format!("{size}px")}));
        }
        w
    }

    /// Text whose baseline is at `base_y` (mixed sizes on one line share a baseline).
    pub fn text_base(&mut self, x: f32, base_y: f32, t: &str, size: f32, color: Color32) -> f32 {
        let top = base_y - baseline(size, LINE);
        self.text_lh(x, top, t, size, color, LINE)
    }

    /// A clipped, independently scrolling box (CSS `overflow-y: auto` with a max height).
    pub fn scroll_box(&mut self, key: impl Hash + Debug, x: f32, y: f32, w: f32, h: f32, draw: impl FnOnce(&mut Pg<'_>, f32) -> f32) {
        let id = self.wid(key);
        let rect = self.rect(x, y, w, h);
        let live = self.live;
        let rec_on = self.rec.on;
        let rec: &mut Rec = self.rec;
        let mut child = self.ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt(id));
        child.set_clip_rect(rect.intersect(self.ui.clip_rect()));
        egui::ScrollArea::vertical().id_salt(id).max_height(h).auto_shrink([false, false]).show_viewport(&mut child, |ui, _vp| {
            let origin = ui.max_rect().min;
            let mut pg = Pg { ui, o: origin, rec, live, id };
            pg.rec.on = rec_on;
            let ch = draw(&mut pg, w);
            ui.allocate_rect(Rect::from_min_size(origin, vec2(w, ch.max(1.0))), Sense::hover());
        });
    }

    pub fn text(&mut self, x: f32, y: f32, t: &str, color: Color32) -> f32 {
        self.text_lh(x, y, t, 16.0, color, LINE)
    }

    pub fn text_sized(&mut self, x: f32, y: f32, t: &str, size: f32, color: Color32) -> f32 {
        self.text_lh(x, y, t, size, color, LINE)
    }

    pub fn measure(&self, t: &str, size: f32) -> f32 {
        measure(self.ui.ctx(), t, size)
    }

    /// Wrapped inline text in a box `w` wide starting at (x, y). Returns the height used.
    pub fn para(&mut self, x: f32, y: f32, w: f32, spans: &[Span], ws: Ws, opts: WrapOpts, line_h: f32) -> f32 {
        let lines = wrap(self.ui.ctx(), spans, w, ws, opts);
        let mut yy = y;
        for line in &lines {
            for p in &line.pieces {
                let sp = &spans[p.span];
                if let Some(bg) = sp.bg {
                    let left = if p.first { sp.pad_x } else { 0.0 };
                    let right = if p.last { sp.pad_x } else { 0.0 };
                    self.fill(x + p.x - left, yy - sp.pad_y, p.w + left + right, line_h + 2.0 * sp.pad_y, bg);
                }
                self.text_lh(x + p.x, yy, &p.text, sp.size, sp.color, line_h);
                if sp.underline {
                    let base = yy + baseline(sp.size, line_h) + 2.0;
                    let a = self.at(x + p.x, base);
                    self.ui.painter().hline(a.x..=a.x + p.w, a.y, Stroke::new(1.0, sp.color));
                }
            }
            yy += line_h;
        }
        yy - y
    }

    pub fn para_h(&self, w: f32, spans: &[Span], ws: Ws, opts: WrapOpts) -> usize {
        wrap(self.ui.ctx(), spans, w, ws, opts).len()
    }

    // ---------------------------------------------------------------------------------------
    // Widgets
    // ---------------------------------------------------------------------------------------
    fn sense(&mut self, id: Id, r: Rect, sense: Sense) -> Response {
        if self.live {
            self.ui.interact(r, id, sense)
        } else {
            self.ui.interact(r, id, Sense::hover())
        }
    }

    /// `[label]`: inverted under the pointer (and with keyboard focus), darker while pressed.
    pub fn button(&mut self, key: impl Hash + Debug, x: f32, y: f32, b: Btn<'_>) -> Response {
        let label = format!("[{}]", b.label);
        let tw = self.measure(&label, b.size);
        let bw = b.box_w.unwrap_or(tw);
        let r = self.rect(x, y, bw, LINE);
        let id = self.wid(key);
        let resp = self.sense(id, r, if b.enabled { Sense::click() } else { Sense::hover() });
        let live = self.live && b.enabled;
        let (bg, fg) = if !b.enabled {
            (None, col::T4)
        } else if live && resp.is_pointer_button_down_on() {
            (Some(col::T2), col::ON_FILL)
        } else if (live && (resp.hovered() || resp.has_focus())) || b.selected {
            (Some(col::FILL), col::ON_FILL)
        } else {
            (None, b.color)
        };
        if let Some(bg) = bg {
            self.ui.painter().rect_filled(r, CornerRadius::ZERO, bg);
        }
        if live && resp.hovered() {
            self.ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        } else if !b.enabled && resp.hovered() {
            self.ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
        }
        // the reference dumps record a button's text without its ::before/::after brackets
        let on = self.rec.on;
        self.rec.on = false;
        self.text_sized(x, y, &label, b.size, fg);
        self.rec.on = on;
        if self.rec.on {
            let lx = r.min.x + self.measure("[", b.size);
            let lw = self.measure(b.label, b.size);
            self.rec.texts.push(json!({"t": b.label, "x": lx, "y": r.min.y, "w": lw, "h": LINE, "c": css(fg), "fs": format!("{}px", b.size)}));
            self.rec.boxes.push(json!({"x": r.min.x, "y": r.min.y, "w": bw, "h": LINE, "tag": "BUTTON", "text": b.label, "c": css(fg)}));
        }
        resp
    }

    /// `[ ] ` / `[x] ` (4 cells). Returns the response; the caller flips the value on click.
    pub fn checkbox(&mut self, key: impl Hash + Debug, x: f32, y: f32, checked: bool, enabled: bool) -> Response {
        let r = self.rect(x, y, 4.0 * CH, LINE);
        let id = self.wid(key);
        let resp = self.sense(id, r, if enabled { Sense::click() } else { Sense::hover() });
        let hot = self.live && enabled && resp.hovered();
        if hot {
            self.ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let c = if checked || hot { col::WHITE } else { col::T2 };
        let on = self.rec.on;
        self.rec.on = false;
        self.text(x, y, if checked { "[x] " } else { "[ ] " }, c);
        self.rec.on = on;
        if self.rec.on {
            self.rec.boxes.push(json!({"x": r.min.x, "y": r.min.y, "w": r.width(), "h": LINE, "tag": "INPUT", "type": "checkbox", "checked": checked}));
        }
        resp
    }

    /// A text field: a dim line with 1ch of padding; brighter while focused.
    pub fn input(&mut self, key: impl Hash + Debug, x: f32, y: f32, w: f32, value: &mut String, ph: &str, o: InputOpts) -> egui::text_edit::TextEditOutput {
        let id = self.wid(key);
        let focused = self.ui.memory(|m| m.has_focus(id));
        let h = o.rows.max(1) as f32 * LINE;
        self.fill(x, y, w, h, if focused { col::INPUT_FOCUS } else { col::INPUT });
        let b = self.galley_baseline(16.0);
        let inner = Rect::from_min_size(self.at(x + CH, y + baseline(16.0, LINE) - b), vec2((w - 2.0 * CH).max(1.0), h));
        let rows = o.rows;
        let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_w: f32| {
            let mut job = egui::text::LayoutJob::simple(buf.as_str().to_owned(), font(16.0), col::T1, wrap_w);
            for s in &mut job.sections {
                s.format.line_height = Some(LINE);
            }
            job.wrap.max_width = if rows > 1 { wrap_w } else { f32::INFINITY };
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let mut te = if o.rows > 1 {
            egui::TextEdit::multiline(value).desired_rows(o.rows)
        } else {
            egui::TextEdit::singleline(value)
        };
        te = te
            .id(id)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .font(font(16.0))
            .text_color(if o.readonly { col::T2 } else { col::T1 })
            .desired_width(inner.width())
            .interactive(self.live && !o.disabled)
            .hint_text(egui::RichText::new(ph).color(col::T3).font(font(16.0)))
            .layouter(&mut layouter);
        if o.password {
            te = te.password(true);
        }
        let mut pasted = String::new();
        if o.paste_append_lower && focused {
            self.ui.input_mut(|i| {
                i.events.retain(|e| match e {
                    egui::Event::Paste(t) => {
                        pasted.push_str(t);
                        false
                    }
                    _ => true,
                })
            });
        }
        let out = self.ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| te.show(ui)).inner;
        if !pasted.is_empty() {
            value.push_str(&pasted.to_lowercase());
        }
        if o.lowercase && out.response.changed() {
            let lower = value.to_lowercase();
            if lower != *value {
                // JS `input.value = input.value.toLowerCase()` puts the caret at the end
                *value = lower;
                let mut st = out.state.clone();
                let end = egui::text::CCursor::new(value.chars().count());
                st.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                st.store(self.ui.ctx(), id);
            }
        }
        if self.rec.on {
            let r = self.rect(x, y, w, h);
            self.rec.boxes.push(json!({"x": r.min.x, "y": r.min.y, "w": w, "h": h, "tag": "INPUT", "value": value.as_str(), "ph": ph}));
        }
        out
    }

    fn galley_baseline(&self, size: f32) -> f32 {
        let g = self.ui.fonts_mut(|f| f.layout_no_wrap("x".into(), font(size), col::T1));
        g.rows.first().and_then(|r| r.row.glyphs.first().map(|gl| r.pos.y + gl.pos.y)).unwrap_or(0.0)
    }

    pub fn hovered(&self, x: f32, y: f32, w: f32, h: f32) -> bool {
        self.live && self.ui.rect_contains_pointer(self.rect(x, y, w, h))
    }
}

#[derive(Clone, Copy, Default)]
pub struct InputOpts {
    pub rows: usize,
    /// answer-questions.js handlePaste: a paste is appended, lowercased, at the end
    pub paste_append_lower: bool,
    pub lowercase: bool,
    pub readonly: bool,
    pub disabled: bool,
    pub password: bool,
}

impl InputOpts {
    pub fn line() -> Self {
        InputOpts { rows: 1, ..Default::default() }
    }
    pub fn lower() -> Self {
        InputOpts { rows: 1, lowercase: true, ..Default::default() }
    }
    pub fn area(rows: usize) -> Self {
        InputOpts { rows, ..Default::default() }
    }
}

/// Global egui style for the console look.
pub fn install_style(ctx: &egui::Context) {
    ctx.global_style_mut(|style| {
        style.spacing.item_spacing = Vec2::ZERO;
        style.spacing.button_padding = Vec2::ZERO;
        style.spacing.interact_size = Vec2::ZERO;
        style.interaction.selectable_labels = false;
        style.interaction.tooltip_delay = f32::INFINITY;
        let v = &mut style.visuals;
        v.dark_mode = true;
        v.panel_fill = col::BG;
        v.window_fill = col::BG;
        v.extreme_bg_color = col::BG;
        v.override_text_color = None;
        v.selection.bg_fill = col::FILL;
        v.selection.stroke = Stroke::new(1.0, col::ON_FILL);
        v.text_cursor.stroke = Stroke::new(1.0, col::WHITE);
        v.text_cursor.blink = true;
        v.text_cursor.on_duration = 0.5;
        v.text_cursor.off_duration = 0.5;
        for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            w.corner_radius = CornerRadius::ZERO;
            w.bg_stroke = Stroke::NONE;
            w.expansion = 0.0;
        }
        v.widgets.inactive.bg_fill = col::LINE_HI;
        v.widgets.hovered.bg_fill = col::T2;
        v.widgets.active.bg_fill = col::T2;
        v.widgets.inactive.weak_bg_fill = col::LINE_HI;
        v.widgets.hovered.weak_bg_fill = col::T2;
        v.widgets.active.weak_bg_fill = col::T2;
        let sc = &mut style.spacing.scroll;
        sc.floating = true;
        sc.bar_width = 6.0;
        sc.floating_width = 6.0;
        sc.floating_allocated_width = 0.0;
        sc.handle_min_length = 18.0;
        sc.bar_inner_margin = 0.0;
        sc.bar_outer_margin = 0.0;
        sc.foreground_color = false;
        sc.dormant_background_opacity = 0.0;
        sc.active_background_opacity = 0.0;
        sc.interact_background_opacity = 0.0;
        sc.dormant_handle_opacity = 1.0;
        sc.active_handle_opacity = 1.0;
        sc.interact_handle_opacity = 1.0;
    });
}

pub fn pos(x: f32, y: f32) -> Pos2 {
    pos2(x, y)
}
