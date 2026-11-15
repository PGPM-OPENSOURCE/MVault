//! answer-questions.html: answers with hint options, a red herring, live strength estimate,
//! seed files, uploads and the final message; then on to the builder.

use std::collections::BTreeMap;
use std::sync::Arc;

use egui::Color32;
use mvault_core::build::{Hint, UserData};
use mvault_core::entropy::{self, Level};
use mvault_core::jsstr::{js_trim, space_indexes, strip_js_space, utf16_len};
use mvault_core::vault;

use crate::app::{Flow, Page, Shared, layer, layer_over};
use crate::pages::build::Build;
use crate::pages::common::{button_row, code, max_content};
use crate::pages::setup::select;
use crate::ui::{Btn, CH, InputOpts, LINE, Pg, Span, Ws, WrapOpts, col, s};

#[derive(Clone)]
pub struct UploadedFile {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub bytes: Arc<Vec<u8>>,
}

pub fn level_color(l: Level) -> Color32 {
    match l {
        Level::Weak => col::RED,
        Level::Warn => col::AMBER,
        Level::Mid => col::MID,
        Level::Good => col::T1,
        Level::Strong => col::WHITE,
    }
}

/// What the "Recovery Preview" line shows (empty until the answer is first edited, as in JS).
#[derive(Clone, Default)]
enum Preview {
    #[default]
    Empty,
    Map(String, String),
    Italic(String),
}

#[derive(Default)]
struct HintModal {
    index: usize,
    show_length: bool,
    show_length_disabled: bool,
    num: usize,
    letter_sel: Vec<usize>,
    mcq: bool,
    correct: String,
    count: usize,
    decoys: Vec<String>,
    custom: String,
    open: Option<String>,
}

pub struct Answers {
    pub questions: Vec<String>,
    pub gates: Vec<usize>,
    pub answers: Vec<String>,
    pub hint_checked: Vec<bool>,
    pub trap: Vec<bool>,
    pub hints: BTreeMap<usize, Hint>,
    entropy: Vec<(String, Level)>,
    preview: Vec<Preview>,
    details: bool,
    seed_name: String,
    seed_content: String,
    seeds: Vec<(String, String)>,
    uploads: Vec<UploadedFile>,
    final_message: String,
    timer_start: f64,
    message: Option<(String, String)>,
    trap_modal: bool,
    hint: Option<HintModal>,
    num_options: usize,
    expired: bool,
}

const ENTROPY_BODY: &[&str] = &[
    "Use answers that only trusted people would know — not Google, Facebook, or public records",
    "Avoid names, birthplaces, and dates — these can often be socially engineered",
    "Use symbolic, metaphorical, or abstract memories, not surface facts",
    "Mix short and long structures and avoid patterns (like “john2024”)",
];

const ASCII_TABLE: &str = "╔═════════════════════════════════════════════════════════════════════════╗
║  ASSUMED ATTACKER HARDWARE         |   GUESSES/SEC   |   TIME TO CRACK     ║
╠═════════════════════════════════════════════════════════════════════════╣
║  Consumer GPU (RTX 4090)           |   45B/sec       |   40-bit: ~24 sec   ║
║                                    |                 |   80-bit: ~27M yrs  ║
║                                    |                 |   120-bit: 10²⁴ yrs ║
╟─────────────────────────────────────────────────────────────────────────╢
║  Enterprise Cluster (A100s)        |   1T/sec        |   40-bit: ~1.1 sec  ║
║                                    |                 |   80-bit: ~38K yrs  ║
║                                    |                 |   120-bit: 10¹¹ yrs ║
╟─────────────────────────────────────────────────────────────────────────╢
║  Nation-State Supercluster         |  100T/sec       |   40-bit: ~0.01 sec ║
║                                    |                 |   80-bit: ~384 yrs  ║
║                                    |                 |   120-bit: 10¹⁹ yrs ║
║                                    |                 |   180-bit: 10⁴⁰ yrs ║
╚═════════════════════════════════════════════════════════════════════════╝
        ";

impl Answers {
    pub fn new(flow: &Flow) -> Answers {
        let n = flow.question_count.unwrap_or(12);
        let saved = flow.questions.clone().unwrap_or_default();
        const DEF: [&str; 24] = [
            "What was the name of your childhood best friend?", "What was the street name where you grew up?", "What is your oldest sibling’s middle name?",
            "What was the name of your first pet?", "What city were you born in?", "What is the name of your favorite childhood teacher?",
            "What was the make of your first car?", "What is the middle name of one of your grandparents?", "What is the title of your favorite childhood book?",
            "What was the mascot of your high school?", "What was your favorite vacation spot as a kid?", "What was the name of your first boss?",
            "What was the color of your childhood bedroom walls?", "What was your childhood dream job?", "What’s the first concert you ever attended?",
            "What’s the name of your favorite stuffed animal or toy?", "What was the first videogame you remember playing?", "What was your childhood nickname?",
            "What is your mother's mother's maiden name?", "What is the name of the first street you lived on?", "What was your favorite childhood meal?",
            "What was your favorite cartoon growing up?", "Who was your favorite childhood hero?", "What is a weird family tradition you had growing up?",
        ];
        let questions: Vec<String> = (0..n).map(|i| saved.get(i).cloned().filter(|q| !q.is_empty()).unwrap_or_else(|| DEF.get(i).map(|q| q.to_string()).unwrap_or_else(|| "undefined".into()))).collect();
        let mut a = Answers {
            gates: flow.unlock_questions.clone().unwrap_or(vec![0, 1]),
            answers: vec![String::new(); n],
            hint_checked: vec![false; n],
            trap: vec![false; n],
            hints: BTreeMap::new(),
            entropy: vec![(String::new(), Level::Weak); n],
            preview: vec![Preview::Empty; n],
            details: false,
            seed_name: String::new(),
            seed_content: String::new(),
            seeds: flow.named_seeds.clone(),
            uploads: flow.uploads.clone(),
            final_message: String::new(),
            timer_start: f64::NAN,
            message: None,
            trap_modal: false,
            hint: None,
            num_options: 0,
            expired: false,
            questions,
        };
        for i in 0..n {
            a.update_entropy(i);
        }
        a
    }

    fn trap_index(&self) -> Option<usize> {
        self.trap.iter().rposition(|t| *t)
    }

    fn update_entropy(&mut self, i: usize) {
        let is_trap = self.trap_index() == Some(i);
        if let Some(e) = entropy::answer_entropy(&self.answers[i], self.hints.get(&i), is_trap) {
            self.entropy[i] = (e.text, e.level);
        }
    }

    fn update_all_entropy(&mut self) {
        for i in 0..self.answers.len() {
            self.update_entropy(i);
        }
    }

    fn update_preview(&mut self, i: usize) {
        let raw = js_trim(&self.answers[i]).to_string();
        let h = self.hints.get(&i).cloned().unwrap_or_default();
        let custom = js_trim(&h.custom).to_string();
        if h.mcq_enabled {
            let n = if h.mcq_options.is_empty() { "—".to_string() } else { h.mcq_options.len().to_string() };
            self.preview[i] = Preview::Italic(format!("Multiple choice ({n} options){}", if custom.is_empty() { String::new() } else { format!(" — Hint: {custom}") }));
            return;
        }
        if !h.show_length.unwrap_or(true) {
            self.preview[i] = Preview::Italic(if custom.is_empty() { "(No length will be shown during recovery)".into() } else { format!("Hint: {custom}") });
            return;
        }
        let units: Vec<u16> = raw.encode_utf16().collect();
        let cleaned: Vec<u16> = strip_js_space(&raw).encode_utf16().collect();
        let spaces = space_indexes(&raw);
        let values: Vec<Option<u16>> = h.letters.iter().map(|&p| (p as usize).checked_sub(1).and_then(|k| cleaned.get(k).copied())).collect();
        let mut out: Vec<u16> = Vec::new();
        let (mut pos, mut vi) = (1u32, 0usize);
        for j in 0..units.len() {
            if spaces.contains(&(j as u32)) {
                out.push(b' ' as u16);
            } else if h.letters.contains(&pos) {
                let v = values.get(vi).copied().flatten();
                vi += 1;
                out.push(v.unwrap_or(b'_' as u16));
                pos += 1;
            } else {
                out.push(b'_' as u16);
                pos += 1;
            }
        }
        let mut spaced = Vec::new();
        for (k, u) in out.iter().enumerate() {
            if k > 0 {
                spaced.push(b' ' as u16);
            }
            spaced.push(*u);
        }
        self.preview[i] = Preview::Map(String::from_utf16_lossy(&spaced), custom);
    }

    fn is_gate(&self, i: usize) -> bool {
        self.gates.contains(&i)
    }
}

impl Answers {
    /// Snapshot scenarios: answers typed in, optionally a red herring.
    pub fn snap_fill(&mut self, answers: &[&str], trap: Option<usize>) {
        for (i, a) in answers.iter().enumerate() {
            self.answers[i] = a.to_lowercase();
            self.update_preview(i);
        }
        if let Some(t) = trap {
            self.trap[t] = true;
        }
        self.update_all_entropy();
    }
    pub fn snap_modal(&mut self, which: &str, i: usize) {
        match which {
            "hint" => {
                self.hint_checked[i] = true;
                open_hint(self, i);
            }
            "trap" => self.trap_modal = true,
            "msg" => self.message = Some(("Invalid Red Herring".into(), "You cannot select one of the Layer 1 unlock questions as a red herring. Please choose a different question.".into())),
            _ => {}
        }
    }
}

// -------------------------------------------------------------------------------------------
// Layout helpers for the question grid (grid-template-columns: 3fr 1fr auto; gaps 16 / 10)
// -------------------------------------------------------------------------------------------
struct BlockLayout {
    c1: f32,
    c2: f32,
    c3: f32,
    label_lines: usize,
}

/// One `.question-block` grid (3fr 1fr auto, 16px column gaps), shrink-to-fit in `avail`. When the
/// block must narrow, the 1fr column stops at its content (the "[ ] Hint Options" cell) and the
/// 3fr column takes what is left.
fn block_layout(pg: &Pg<'_>, label: &[Span], entropy_text: &str, avail: f32) -> BlockLayout {
    let label_w = max_content(pg, &[label.to_vec()]);
    let hint_w = 4.0 * CH + pg.measure("Hint Options", 16.0);
    let trap_w = 4.0 * CH + pg.measure("Use as", 16.0) + 4.0 + pg.measure("Red Herring", 16.0) + 6.0 + 3.0 * CH;
    let c3 = trap_w.max(pg.measure(entropy_text, 16.0));
    let want = (label_w / 3.0).max(hint_w);
    let room = avail - 2.0 - 32.0 - c3;
    let (c1, c2) = if 4.0 * want <= room {
        (3.0 * want, want)
    } else if room / 4.0 >= hint_w {
        (3.0 * room / 4.0, room / 4.0)
    } else {
        ((room - hint_w).max(CH), hint_w)
    };
    let lines = crate::ui::wrap(&pg.ctx(), label, c1, Ws::Normal, WrapOpts { anywhere: true }).len().max(1);
    BlockLayout { c1, c2, c3, label_lines: lines }
}

pub fn show(st: &mut Answers, sh: &mut Shared, ui: &mut egui::Ui) {
    if st.timer_start.is_nan() {
        st.timer_start = sh.time;
    }
    let remaining = 1800 - (sh.time - st.timer_start).floor() as i64;
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
    if remaining <= 0 && !st.expired {
        // session expired: wipe, alert, back to the dashboard
        st.expired = true;
        sh.flow.wipe();
        rfd::MessageDialog::new().set_description("Session expired for your security. Sensitive data has been cleared.").set_buttons(rfd::MessageButtons::Ok).show();
        sh.nav = Some(crate::app::Nav::Replace(Page::Dashboard(Default::default())));
        return;
    }
    if st.message.is_some() {
        return message(st, sh, ui);
    }
    if st.trap_modal {
        if !crate::app::has_geom(ui.ctx(), "answers") {
            page(st, sh, ui, remaining.max(0));
        }
        return trap_modal(st, sh, ui);
    }
    if st.hint.is_some() {
        return hint_modal(st, sh, ui);
    }
    page(st, sh, ui, remaining.max(0));
}

#[derive(Default)]
struct Events {
    changed: Vec<usize>,
    hint_toggle: Option<usize>,
    trap_toggle: Option<usize>,
    info: bool,
    details: bool,
    save_seed: bool,
    remove_seed: Option<usize>,
    choose_files: bool,
    remove_upload: Option<usize>,
    nav: Option<usize>,
}

fn page(st: &mut Answers, sh: &mut Shared, ui: &mut egui::Ui, remaining: i64) {
    let mut ev = Events::default();
    let n = st.answers.len();
    let total = entropy::total(&st.entropy.iter().map(|e| e.0.clone()).collect::<Vec<_>>());
    layer(ui, sh, "answers", true, |pg, width| {
        let (x, w) = (8.0, width - 16.0);
        let mut y = 8.0;
        y += pg.para(x, y, w, &[s("Answer Your Questions and Upload Files", col::WHITE)], Ws::Normal, WrapOpts::default(), LINE) + LINE;
        pg.text(x, y, &format!("Sensitive Data Will Be Securely Erased In: {}:{:02}", remaining / 60, remaining % 60), col::T3);
        y += LINE + 22.0;
        for i in 0..n {
            let label = vec![s(format!("{}.", i + 1), col::T3).margin_r(2.0), s(format!(" {}", st.questions[i]), col::T1)];
            let bl = block_layout(pg, &label, &st.entropy[i].0, w);
            let (bx, by) = (x + 1.0, y + 1.0);
            let c1 = bl.c1;
            // row 1: label + input | hint options | red herring
            let lh = bl.label_lines as f32 * LINE;
            pg.para(bx, by, c1, &label, Ws::Normal, WrapOpts { anywhere: true }, LINE);
            let iy = by + lh + 6.0;
            let before = st.answers[i].clone();
            let out = pg.input(("answer", i), bx, iy, c1, &mut st.answers[i], "Your answer", InputOpts { paste_append_lower: true, ..InputOpts::lower() });
            if out.response.changed() || st.answers[i] != before {
                ev.changed.push(i);
            }
            let row1 = lh + 6.0 + LINE;
            let cy = by + (row1 - LINE) / 2.0;
            let x2 = bx + c1 + 16.0;
            if pg.checkbox(("hint", i), x2, cy, st.hint_checked[i], true).clicked() {
                ev.hint_toggle = Some(i);
            }
            pg.text(x2 + 4.0 * CH, cy, "Hint Options", col::T2);
            let x3 = x2 + bl.c2 + 16.0;
            if pg.checkbox(("trap", i), x3, cy, st.trap[i], true).clicked() {
                ev.trap_toggle = Some(i);
            }
            let mut xx = x3 + 4.0 * CH;
            xx += pg.text(xx, cy, "Use as", col::T2) + 4.0;
            xx += pg.text(xx, cy, "Red Herring", col::T1) + 6.0;
            if pg.hovered(xx, cy, 3.0 * CH, LINE) {
                pg.ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            pg.text(xx, cy, "(i)", col::T2);
            let ir = pg.rect(xx, cy, 3.0 * CH, LINE);
            if pg.live && pg.ui.interact(ir, pg.wid(("info", i)), egui::Sense::click()).clicked() {
                ev.info = true;
            }
            // row 2: recovery preview | entropy (bottom right)
            let r2y = by + row1 + 10.0;
            pg.text(bx, r2y, "RECOVERY PREVIEW:", col::T3);
            let py = r2y + LINE + 4.0;
            let ph = match &st.preview[i] {
                Preview::Empty => 0.0,
                Preview::Map(code_s, custom) => {
                    let cw = pg.text_base(bx, py + 14.0, code_s, 14.0, col::T2);
                    if !custom.is_empty() {
                        pg.text_base(bx + cw + 10.0, py + 14.0, &format!("Hint: {custom}"), 13.0, col::T2);
                    }
                    LINE
                }
                Preview::Italic(t) => pg.para(bx, py, bl.c1 + 16.0 + bl.c2, &[s(t, col::T3)], Ws::Normal, WrapOpts::default(), LINE),
            };
            let row2 = LINE + 4.0 + ph;
            let et = &st.entropy[i];
            let ew = pg.measure(&et.0, 16.0);
            pg.text(x3 + bl.c3 - ew, r2y + row2 - LINE, &et.0, level_color(et.1));
            y += 2.0 + row1 + 10.0 + row2 + LINE;
        }
        // vault strength (the empty #extraSeedsSection's 28px margin collapses through it and wins)
        y += 28.0;
        let (sx, sy) = (x + 1.0, y + 1.0);
        let mut yy = sy;
        let mut xx = sx;
        xx += pg.text(xx, yy, "Estimated Vault Strength: ", col::T2);
        xx += pg.text(xx, yy, &total.bits_text, col::T1);
        xx += pg.text(xx, yy, " bits — ", col::T2);
        pg.text(xx, yy, total.assessment, level_color(total.assessment_level));
        yy += LINE;
        let mut xx = sx;
        xx += pg.text(xx, yy, "Total Combinations: ", col::T2);
        xx += pg.text(xx, yy, &total.combinations, col::T1);
        xx += pg.text(xx, yy, " ", col::T2);
        xx += pg.text(xx, yy, "Estimated Crack Time: ", col::T2);
        pg.text(xx, yy, &total.crack_time, col::AMBER);
        yy += LINE;
        pg.text(sx, yy, "(assuming 1 trillion guesses/sec — botnet-level offline attack)", col::T3);
        yy += LINE + 8.0;
        pg.text(sx, yy, total.tier, level_color(total.tier_level));
        yy += LINE;
        yy += pg.para(sx, yy, w - 2.0, &[s("This estimation does not factor in the potential for phishing, coercion, or social engineering — never share personal details.", col::T3)], Ws::Normal, WrapOpts::default(), LINE);
        y = yy + 1.0 + 20.0;
        // <details>
        let sw = pg.measure("What is entropy and how is my vault protected?", 16.0);
        let hot = pg.hovered(x, y, w, LINE);
        if hot {
            pg.fill(x, y, w, LINE, col::FILL);
            pg.ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let tc = if hot { col::ON_FILL } else { col::T2 };
        triangle(pg, x + 2.0, y, st.details, tc);
        pg.para(x + 13.75, y, w - 13.75, &[s("What is entropy and how is my vault protected?", tc)], Ws::Normal, WrapOpts { anywhere: true }, LINE);
        let _ = sw;
        if pg.live && pg.ui.interact(pg.rect(x, y, w, LINE), pg.wid("details"), egui::Sense::click()).clicked() {
            ev.details = true;
        }
        y += LINE;
        if st.details {
            y += 16.0;
            y = entropy_body(pg, x, y, w) - 16.0; // its last margin collapses into the next section's
        }
        // seed files
        y += 28.0 + 1.0;
        pg.text(x, y, "SAVED SEED OR PASSWORD FILES", col::WHITE);
        y += 2.0 * LINE;
        pg.text(x, y, "Seed File/Password Name:", col::T2);
        y += LINE + 6.0;
        pg.input("seedname", x, y, w, &mut st.seed_name, "e.g. Primary Wallet", InputOpts::line());
        y += LINE + 10.0;
        pg.text(x, y, "Seed Phrase or Password:", col::T2);
        y += LINE + 6.0;
        pg.input("seedcontent", x, y, w, &mut st.seed_content, "type or paste seed phrase or private passphrase here", InputOpts::area(3));
        y += 3.0 * LINE + 12.0;
        if pg.button("saveseed", x, y, Btn::new("Save Password/Seed File")).clicked() {
            ev.save_seed = true;
        }
        y += LINE;
        if !st.seeds.is_empty() {
            y += 20.0;
            for (k, (name, content)) in st.seeds.iter().enumerate() {
                let (ex, ey) = (x + 1.0, y + 1.0);
                pg.text(ex, ey, &format!("{name}.txt"), col::WHITE);
                let collapsed: String = content.split_whitespace().collect::<Vec<_>>().join(" ");
                let ch = pg.para(ex, ey + LINE, w - 2.0, &[s(collapsed, col::T2)], Ws::Normal, WrapOpts { anywhere: true }, LINE);
                let by2 = ey + LINE + ch + 8.0;
                if pg.button(("rmseed", k), ex, by2, Btn::new("Remove").color(col::RED)).clicked() {
                    ev.remove_seed = Some(k);
                }
                y = by2 + LINE + 1.0 + LINE;
            }
            y -= LINE;
        }
        // uploads
        y += 28.0;
        pg.text(x, y, "Optional: Upload Documents to Include in Your Vault", col::T1);
        y += LINE + 6.0;
        let lw = pg.measure("Choose Files", 16.0) + 32.0;
        let hot = pg.hovered(x, y, lw, LINE);
        pg.fill(x, y, lw, LINE, if hot { col::FILL } else { col::BTN });
        if hot {
            pg.ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        pg.text(x + 16.0, y, "Choose Files", if hot { col::ON_FILL } else { col::T1 });
        if pg.live && pg.ui.interact(pg.rect(x, y, lw, LINE), pg.wid("choose"), egui::Sense::click()).clicked() {
            ev.choose_files = true;
        }
        let count = st.uploads.len();
        let status = if count == 0 { "No files uploaded".to_string() } else { format!("{count} file{} uploaded", if count != 1 { "s" } else { "" }) };
        pg.text(x + lw + 12.0, y, &status, col::T2);
        y += LINE + 6.0;
        y += pg.para(x, y, w, &[s("You can upload files like text notes, images, videos, or scanned documents.", col::T3)], Ws::Normal, WrapOpts::default(), LINE) + 16.0;
        y += 20.0;
        for (k, f) in st.uploads.iter().enumerate() {
            let (ex, ey) = (x + 1.0, y + 1.0);
            let label = format!("{} ({} KB)", f.name, (f.bytes.len() as f64 / 1024.0).round() as u64);
            let lw2 = pg.measure(&label, 16.0);
            let maxl = w - 2.0 - 12.0 - pg.measure("[Remove]", 16.0);
            let shown = if lw2 > maxl { crate::pages::common::ellipsize(pg, &label, maxl) } else { label.clone() };
            pg.text(ex, ey, &shown, col::T1);
            if pg.button(("rmup", k), ex + lw2.min(maxl) + 12.0, ey, Btn::new("Remove").color(col::RED)).clicked() {
                ev.remove_upload = Some(k);
            }
            y += LINE + 2.0 + 6.0;
        }
        if st.uploads.is_empty() {
            y -= 0.0;
        } else {
            y -= 6.0;
        }
        // final message
        y += 28.0 - if st.uploads.is_empty() { 20.0 } else { 0.0 };
        pg.text(x, y, "Optional: Final Message for Yourself or Your Family", col::T1);
        y += LINE + 6.0;
        pg.input("final", x, y, w, &mut st.final_message, "Example: 'Looks like someone was ACTUALLY listening to all those rants I gave at family dinner. Congratulations!'", InputOpts::area(4));
        y += 4.0 * LINE + 6.0;
        y += pg.para(x, y, w, &[s("This message will be stored securely with your vault.", col::T3)], Ws::Normal, WrapOpts::default(), LINE) + 16.0;
        y += 28.0;
        let (c, h) = button_row(pg, "nav", x, y, w, &[Btn::new("← Back"), Btn::new("Create Vault").main(), Btn::new("Reset Timer")]);
        ev.nav = c;
        y + h + 8.0
    });
    apply_events(st, sh, ev);
}

fn triangle(pg: &mut Pg<'_>, x: f32, y: f32, open: bool, c: Color32) {
    // the disclosure marker: a small solid triangle (right when closed, down when open)
    let p = pg.at(x, y);
    let pts = if open {
        vec![egui::pos2(p.x, p.y + 7.0), egui::pos2(p.x + 8.0, p.y + 7.0), egui::pos2(p.x + 4.0, p.y + 13.0)]
    } else {
        vec![egui::pos2(p.x + 1.0, p.y + 5.5), egui::pos2(p.x + 7.0, p.y + 9.5), egui::pos2(p.x + 1.0, p.y + 13.5)]
    };
    pg.ui.painter().add(egui::Shape::convex_polygon(pts, c, egui::Stroke::NONE));
}

fn entropy_body(pg: &mut Pg<'_>, x: f32, mut y: f32, w: f32) -> f32 {
    let (t1, t2) = (col::T1, col::T2);
    let p = |pg: &mut Pg<'_>, y: f32, spans: &[Span]| pg.para(x, y, w, spans, Ws::Normal, WrapOpts::default(), LINE);
    y += p(pg, y, &[s("Entropy", t1), s(" is a measure of unpredictability — how many guesses an attacker would need to crack your answers. Memoro Vault uses ", t2), s("zxcvbn", t1), s(", a password strength estimator from Dropbox, to evaluate the entropy of each response. We calculate this conservatively — assuming only 26 lowercase letters and no symbols — to ensure you're not overestimating security.", t2)]) + 16.0;
    y += p(pg, y, &[s("We assume ", t2), s("the worst-case scenario", t1), s(": global adversaries with the fastest hardware on Earth.", t2)]) + 16.0;
    y += 1.0;
    y += pg.para(x + 1.0, y, w - 2.0, &[s(ASCII_TABLE, t2)], Ws::Pre, WrapOpts::default(), LINE) + 1.0 + 16.0;
    y += p(pg, y, &[s("Memoro Vault uses this to classify your vault strength:", t2)]) + 16.0;
    let levels = [("Very Weak", ": under 40 bits — crackable in seconds to minutes"), ("Weak", ": 40–79 bits — crackable in minutes to months"),
        ("Moderate", ": 80–119 bits — could take decades to millions of years"), ("Strong", ": 120–179 bits — would take longer than the age of the universe using today’s best hardware"),
        ("Very Strong", ": 180+ bits — beyond the reach of any current or foreseeable brute-force attack")];
    for (a, b) in levels {
        y += crate::pages::common::li(pg, x + 44.0, y, w - 44.0, &[s(a, t1), s(b, t2)], t2);
    }
    y += 16.0;
    y += p(pg, y, &[s("Even one weak answer can collapse your vault. To strengthen it:", t2)]) + 16.0;
    for b in ENTROPY_BODY {
        y += crate::pages::common::li(pg, x + 44.0, y, w - 44.0, &[s(*b, t2)], t2);
    }
    y += 16.0;
    y += p(pg, y, &[s("Memoro Vault provides no hints, no partial credit, and no feedback on incorrect input. To unlock the vault, users must enter the exact correct answers for all questions, and for six of them, they must also provide the correct order. This creates a layered security model: the decryption key is only derived if every answer is 100% accurate, and the six ordered answers match the precise sequence used during vault creation. With no indication of which part is wrong, attackers must blindly guess both content and order across hundreds of possibilities—making brute-force attempts impractical and expensive.", t2)]) + 16.0;
    y
}

fn apply_events(st: &mut Answers, sh: &mut Shared, ev: Events) {
    for i in ev.changed {
        st.update_entropy(i);
        st.update_preview(i);
    }
    if let Some(i) = ev.hint_toggle {
        st.hint_checked[i] = !st.hint_checked[i];
        if st.hint_checked[i] {
            open_hint(st, i);
        }
    }
    if let Some(i) = ev.trap_toggle {
        if !st.trap[i] {
            if st.is_gate(i) {
                st.message = Some(("Invalid Red Herring".into(), "You cannot select one of the Layer 1 unlock questions as a red herring. Please choose a different question.".into()));
            } else if st.trap.iter().any(|t| *t) {
                st.message = Some(("Only One Red Herring Allowed".into(), "You can only select one question as the Red Herring.".into()));
            } else {
                st.trap[i] = true;
            }
        } else {
            st.trap[i] = false;
        }
        st.update_preview(i);
        st.update_all_entropy();
    }
    if ev.info {
        st.trap_modal = true;
    }
    if ev.details {
        st.details = !st.details;
    }
    if ev.save_seed {
        let name = js_trim(&st.seed_name).to_string();
        let content = js_trim(&st.seed_content).to_string();
        if name.is_empty() || content.is_empty() {
            st.message = Some(("Missing Info".into(), "You must provide both a name and a seed phrase.".into()));
        } else {
            st.seeds.push((vault::named_seed_base(&name), content));
            st.seed_name.clear();
            st.seed_content.clear();
            sh.flow.named_seeds = st.seeds.clone();
        }
    }
    if let Some(k) = ev.remove_seed {
        st.seeds.remove(k);
        sh.flow.named_seeds = st.seeds.clone();
    }
    if ev.choose_files {
        choose_files(st);
        sh.flow.uploads = st.uploads.clone();
    }
    if let Some(k) = ev.remove_upload {
        st.uploads.remove(k);
        sh.flow.uploads = st.uploads.clone();
    }
    match ev.nav {
        Some(0) => sh.nav = Some(crate::app::Nav::Back),
        Some(1) => submit(st, sh),
        Some(2) => st.timer_start = sh.time,
        _ => {}
    }
}

fn choose_files(st: &mut Answers) {
    let Some(paths) = rfd::FileDialog::new().pick_files() else { return };
    let sizes: Vec<(std::path::PathBuf, u64)> = paths.into_iter().map(|p| { let n = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0); (p, n) }).collect();
    let name = |p: &std::path::Path| p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let too_big: Vec<String> = sizes.iter().filter(|(_, n)| *n > vault::MAX_FILE_BYTES).map(|(p, _)| name(p)).collect();
    if !too_big.is_empty() {
        let one = too_big.len() == 1;
        st.message = Some(("File Too Large".into(), format!("{} {} larger than the 1.5 GB a single file in a vault can be, so {} not added.", too_big.join(", "), if one { "is" } else { "are" }, if one { "it was" } else { "they were" })));
        return;
    }
    let already: u64 = st.uploads.iter().map(|u| u.bytes.len() as u64).sum();
    if already + sizes.iter().map(|(_, n)| n).sum::<u64>() > vault::MAX_VAULT_BYTES {
        st.message = Some(("Vault Too Large".into(), "Adding these files would take the vault past 1.9 GB, which is more than Memoro Vault can seal and open. Nothing was added.".into()));
        return;
    }
    for (p, _) in sizes {
        if let Ok(bytes) = std::fs::read(&p) {
            let n = name(&p);
            st.uploads.push(UploadedFile { id: mvault_core::crypto::random_hex(16), mime: guess_mime(&n).to_string(), name: n, bytes: Arc::new(bytes) });
        }
    }
}

/// `File.type` as Chromium reports it for common extensions ("" when unknown).
pub fn guess_mime(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "txt" | "text" => "text/plain", "md" => "text/markdown", "csv" => "text/csv", "json" => "application/json", "xml" => "text/xml",
        "html" | "htm" => "text/html", "css" => "text/css", "js" => "text/javascript", "pdf" => "application/pdf",
        "png" => "image/png", "jpg" | "jpeg" => "image/jpeg", "gif" => "image/gif", "webp" => "image/webp", "bmp" => "image/bmp",
        "svg" => "image/svg+xml", "ico" => "image/vnd.microsoft.icon", "avif" => "image/avif", "mp3" => "audio/mpeg", "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg", "m4a" => "audio/x-m4a", "aac" => "audio/aac", "flac" => "audio/flac", "opus" => "audio/opus",
        "mp4" | "m4v" => "video/mp4", "webm" => "video/webm", "ogv" => "video/ogg", "mov" => "video/quicktime", "zip" => "application/zip",
        "doc" => "application/msword", "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "rtf" => "application/rtf", "asc" => "application/pgp-signature", _ => "",
    }
}

fn submit(st: &mut Answers, sh: &mut Shared) {
    let trap = st.trap_index();
    if let Some(t) = trap {
        if st.is_gate(t) {
            st.message = Some(("Invalid Red Herring".into(), "An unlock question cannot be the Red Herring. Please choose a different question.".into()));
            return;
        }
    }
    let answers: Vec<String> = st.answers.iter().map(|a| js_trim(a).to_string()).collect();
    if answers.iter().any(|a| a.is_empty()) {
        // no seed-word fields on this page, but the vault type is unset, so the message names them
        st.message = Some(("Missing Information".into(), "Please fill in all answers and all seed words.".into()));
        return;
    }
    let mut hints = st.hints.clone();
    if let Some(t) = trap {
        hints.entry(t).or_insert_with(|| Hint { show_length: Some(true), ..Default::default() }).show_length = Some(true);
    }
    let gates = sh.flow.unlock_questions.clone().unwrap_or(vec![0, 1]);
    let user = UserData {
        space_indexes: answers.iter().map(|a| space_indexes(a)).collect(),
        answers,
        hints,
        questions: sh.flow.questions.clone().unwrap_or_default(),
        unlock_indexes: [gates.first().copied().unwrap_or(0), gates.get(1).copied().unwrap_or(1)],
        final_message: js_trim(&st.final_message).to_string(),
        trap_index: trap,
    };
    sh.flow.user_data = Some(user);
    sh.flow.named_seed_files = st.seeds.iter().map(|(n, c)| vault::Seed { name: format!("{n}.txt"), content: c.clone() }).collect();
    sh.flow.named_seeds.clear();
    sh.flow.uploads = st.uploads.clone();
    sh.go(Page::Build(Build::new()));
}

// -------------------------------------------------------------------------------------------
// Dialogs
// -------------------------------------------------------------------------------------------
fn message(st: &mut Answers, sh: &mut Shared, ui: &mut egui::Ui) {
    let (title, text) = st.message.clone().unwrap();
    let mut ok = false;
    layer(ui, sh, "ans-msg", true, |pg, width| {
        let w = max_content(pg, &[vec![s(&title, col::WHITE)], vec![s(&text, col::T2)]]).min(width - 18.0);
        let (x, mut y) = (9.0, 9.0);
        y += pg.para(x, y, w, &[s(&title, col::WHITE)], Ws::Normal, WrapOpts::default(), LINE) + LINE;
        y += pg.para(x, y, w, &[s(&text, col::T2)], Ws::PreLine, WrapOpts::default(), LINE) + 20.0;
        ok = pg.button("ok", x, y, Btn::new("OK")).clicked();
        y + LINE + 9.0
    });
    if ok {
        st.message = None;
    }
}

fn trap_modal(st: &mut Answers, sh: &mut Shared, ui: &mut egui::Ui) {
    let (t1, t2) = (col::T1, col::T2);
    let paras: Vec<Vec<Span>> = vec![
        vec![s("A ", t2), s("Red Herring", t1), s(" is a decoy question: it appears like the others, but you answer it ", t2), s("incorrectly on purpose", t1), s(". During recovery, the correct action is to leave it ", t2), s("blank", t1), s(".", t2)],
        vec![s("Purpose:", t1), s("\nRed Herrings add a powerful layer of protection against brute force attacks and social engineering. An attacker has no way to know which question is fake, and any attempt to answer it results in failure.", t2)],
        vec![s("How to use it:", t1), s("\n• Choose a ", t2), s("real-looking question", t1), s(" that only close friends or family would recognize.\n• Enter a ", t2), s("wrong answer", t1), s(" that subtly signals it’s not genuine.\n• Make sure the fake answer is at least ", t2), s("two characters longer or shorter", t1), s(" than the real one.", t2)],
        vec![s("Why length matters:", t1), s("\nDuring recovery, Memoro Vault displays the expected number of characters for each answer by default. This lets trusted contacts recognize mismatches and know to leave the Red Herring blank.", t2)],
        vec![s("Example:", t1), s("\n", t2), s("Question:", t2), s(" “What was my childhood nickname?”\n", t2), s("Real Answer:", t2), s(" ", t2), code("Squirt"), s(" (6 characters)\n", t2), s("Red Herring Answer:", t2), s(" ", t2), code("Tuf"),
             s(" (3 characters)\nIf someone sees the vault is expecting a 3-character answer but knows the real answer has 6 characters, they'll know to leave it blank. Vault builders can also choose to give away letter/s to help those in their inner circle recognize the Red Herring.", t2)],
        vec![s("Psychological Advantage:", t1), s("\nThe beauty of the Red Herring is this: even if you choose not to use one, the attacker doesn’t know that.\nThey’re forced to ask themselves: ", t2), s("\"Do I leave one blank? If so, which one?\"", t2), s("\nThis uncertainty makes brute-force and social engineering attacks much more difficult, whether or not a Red Herring was actually used.", t2)],
        vec![s("Guidelines:", t1), s("\n• Do not use questions that can be profiled, guessed, or searched online.\n• Only one Red Herring is allowed.\n• Using one is optional — but even the possibility dramatically increases your security.", t2)],
    ];
    let mut ok = false;
    layer_over(ui, sh, "ans-trap", Some("answers"), true, |pg, width| {
        let w = (width - 18.0).min(max_content(pg, &paras.iter().map(|p| p.iter().map(|sp| Span { text: sp.text.replace('\n', " "), ..sp.clone() }).collect()).collect::<Vec<_>>()));
        let (x, mut y) = (9.0, 9.0);
        y += pg.para(x, y, w, &[s("What is a Red Herring?", col::WHITE)], Ws::Normal, WrapOpts::default(), LINE) + LINE;
        for (i, p) in paras.iter().enumerate() {
            if i > 0 {
                y += 16.0;
            }
            y += pg.para(x, y, w, p, Ws::PreLine, WrapOpts::default(), LINE);
        }
        y += 20.0;
        ok = pg.button("ok", x, y, Btn::new("Got it")).clicked();
        y + LINE + 9.0
    });
    if ok {
        st.trap_modal = false;
    }
}

fn cleaned_answer(st: &Answers, i: usize) -> Vec<u16> {
    strip_js_space(js_trim(&st.answers[i])).encode_utf16().collect()
}

fn open_hint(st: &mut Answers, i: usize) {
    let saved = st.hints.get(&i).cloned().unwrap_or_default();
    let trap = st.trap_index();
    let mut m = HintModal { index: i, ..Default::default() };
    m.show_length = saved.show_length.unwrap_or(true);
    if Some(i) == trap && !saved.mcq_enabled {
        m.show_length = true;
        m.show_length_disabled = true;
    }
    // numLetters: the saved count is only kept if the old option list already had it
    let max_hints = cleaned_answer(st, i).len() / 2;
    let mut num = 0;
    if saved.letters.len() < st.num_options {
        num = saved.letters.len();
    }
    if st.num_options != max_hints + 1 {
        st.num_options = max_hints + 1;
        num = 0;
    }
    m.num = num;
    m.letter_sel = vec![0; num];
    let gate = st.is_gate(i);
    m.mcq = saved.mcq_enabled && !gate;
    let opts = if gate { vec![] } else { saved.mcq_options.clone() };
    m.correct = opts.first().cloned().unwrap_or_else(|| st.answers[i].clone());
    let decoys: Vec<String> = opts.iter().skip(1).cloned().collect();
    let n = (decoys.len() + 1).clamp(6, 12);
    m.count = n;
    m.decoys = (0..n - 1).map(|k| decoys.get(k).cloned().unwrap_or_default()).collect();
    if m.mcq {
        m.show_length = false;
        m.show_length_disabled = true;
    }
    if saved.mcq_enabled && gate {
        st.message = Some(("MCQ not allowed".into(), "Multiple-choice is disabled for Layer-1 unlock questions. Use length/letters or a free-text hint instead.".into()));
    }
    st.hint = Some(m);
}

fn hint_modal(st: &mut Answers, sh: &mut Shared, ui: &mut egui::Ui) {
    let i = st.hint.as_ref().unwrap().index;
    let cleaned = cleaned_answer(st, i);
    let max_hints = cleaned.len() / 2;
    let trap = st.trap_index();
    let gate = st.is_gate(i);
    let mut m = st.hint.take().unwrap();
    let mut action: Option<usize> = None;
    let mut mcq_click = false;
    let mut len_click = false;
    let mut warn: Option<(String, String)> = None;
    if m.num > max_hints {
        m.num = max_hints;
        warn = Some(("Warning".into(), format!("You cannot reveal more than 50% of your answer's letters (Max {max_hints}).")));
    }
    m.letter_sel.resize(m.num, 0);
    let letters_disabled = m.mcq || !m.show_length;
    layer(ui, sh, "ans-hint", true, |pg, width| {
        let note = "MCQ hides length and letters. Enter 5–11 convincing decoys; the correct option is your real answer.";
        let mut w = pg.measure("Hint Options", 16.0)
            .max(4.0 * CH + pg.measure("Show answer length during unlock", 16.0))
            .max(pg.measure("Reveal specific letters from the answer", 16.0))
            .max(4.0 * CH + 8.0 + pg.measure("Make this a multiple-choice question", 16.0))
            .max(pg.measure("Optional: Custom Hint Text", 16.0));
        if m.mcq {
            w = w.max(pg.measure(note, 16.0));
        }
        let w = w.min(width - 18.0);
        let (x, mut y) = (9.0, 9.0);
        y += pg.para(x, y, w, &[s("Hint Options", col::WHITE)], Ws::Normal, WrapOpts::default(), LINE) + LINE;
        if pg.checkbox("len", x, y, m.show_length, !m.show_length_disabled).clicked() && !m.show_length_disabled {
            len_click = true;
        }
        pg.text(x + 4.0 * CH, y, "Show answer length during unlock", col::T2);
        y += LINE + 6.0 + 14.0;
        pg.text(x, y, "Reveal specific letters from the answer", col::T2);
        y += LINE + 6.0;
        let num_opts: Vec<String> = (0..=max_hints).map(|k| if k == 0 { "0 letters (only answer length)".into() } else { format!("{k} letter{}", if k > 1 { "s" } else { "" }) }).collect();
        let num_refs: Vec<&str> = num_opts.iter().map(String::as_str).collect();
        let num_y = y;
        y += LINE;
        let mut sel_ys = Vec::new();
        if m.num > 0 {
            y += 15.0 + 1.0 + 8.0;
            for _ in 0..m.num {
                sel_ys.push(y);
                y += LINE + 6.0;
            }
            y += 8.0 + 1.0 - 6.0;
        }
        y += 16.0 + 1.0 + 16.0;
        let mcq_y = y;
        if pg.checkbox("mcq", x, y, m.mcq, true).clicked() {
            mcq_click = true;
        }
        pg.text(x + 4.0 * CH + 8.0, y, "Make this a multiple-choice question", col::T2);
        y += LINE + 6.0;
        let mut count_y = None;
        if m.mcq {
            y += 10.0;
            pg.text(x, y, "Correct option (your real answer)", col::T2);
            let c2x = x + w - 180.0;
            pg.text(c2x, y, "(6–12)", col::T2);
            y += LINE + 6.0;
            pg.input("correct", x, y, w - 180.0 - 12.0, &mut m.correct, "Type the one true answer", InputOpts::line());
            count_y = Some((c2x, y));
            y += LINE + 10.0;
            for k in 0..m.decoys.len() {
                pg.text(x, y, &format!("Decoy {}", k + 1), col::T2);
                y += LINE + 6.0 + 2.0;
                pg.input(("decoy", k), x, y, w, &mut m.decoys[k], "convincing but wrong", InputOpts::line());
                y += LINE;
            }
            y += 16.0;
            y += pg.para(x, y, w, &[s(note, col::T2)], Ws::Normal, WrapOpts::default(), LINE) + 16.0;
        }
        y += 16.0 + 1.0 + 20.0;
        pg.text(x, y, "Optional: Custom Hint Text", col::T2);
        y += LINE + 6.0;
        pg.input("custom", x, y, w, &mut m.custom, "Example: 'At the park'...", InputOpts::area(2));
        y += 2.0 * LINE + 20.0;
        let (c, h) = button_row(pg, "hb", x, y, w, &[Btn::new("Save").main(), Btn::new("Cancel")]);
        action = c;
        // selects last (their lists open over what follows)
        let mut open_num = m.open.as_deref() == Some("num");
        let before = m.num;
        if !letters_disabled {
            select(pg, "num", x, num_y, w, &num_refs, &mut m.num, &mut open_num);
        } else {
            let mut o = false;
            let mut v = m.num;
            select(pg, "num-d", x, num_y, w, &num_refs, &mut v, &mut o);
        }
        if m.num != before {
            m.letter_sel = vec![0; m.num];
        }
        let letter_opts: Vec<String> = cleaned.iter().enumerate().map(|(j, u)| format!("{} ({})", String::from_utf16_lossy(&[*u]), j + 1)).collect();
        let letter_refs: Vec<&str> = letter_opts.iter().map(String::as_str).collect();
        let mut opened = if open_num { Some("num".to_string()) } else { None };
        for (k, sy) in sel_ys.iter().enumerate() {
            let key = format!("l{k}");
            let mut o = m.open.as_deref() == Some(key.as_str());
            if letters_disabled {
                o = false;
            }
            select(pg, &key, x + 9.0, *sy, w - 18.0, &letter_refs, &mut m.letter_sel[k], &mut o);
            if o {
                opened = Some(key);
            }
        }
        if let Some((cx, cy)) = count_y {
            let counts: Vec<String> = (6..=12).map(|n| format!("{n} options")).collect();
            let crefs: Vec<&str> = counts.iter().map(String::as_str).collect();
            let mut idx = m.count - 6;
            let mut o = m.open.as_deref() == Some("count");
            if select(pg, "count", cx, cy, 180.0, &crefs, &mut idx, &mut o) {
                let n = idx + 6;
                m.count = n;
                let mut resized: Vec<String> = m.decoys.iter().take(n - 1).cloned().collect();
                while resized.len() < (n - 1).max(5) {
                    resized.push(String::new());
                }
                m.decoys = resized;
            }
            if o {
                opened = Some("count".into());
            }
        }
        m.open = opened;
        let _ = mcq_y;
        y + h + 9.0
    });
    if len_click {
        m.show_length = !m.show_length;
    }
    if mcq_click {
        if !m.mcq && gate {
            warn = Some(("MCQ not allowed".into(), "Multiple-choice is disabled for Layer-1 unlock questions. Use length/letters or a free-text hint instead.".into()));
        } else {
            m.mcq = !m.mcq;
            if m.mcq {
                m.show_length = false;
                m.show_length_disabled = true;
            } else {
                m.show_length_disabled = Some(i) == trap;
            }
        }
    }
    st.hint = Some(m);
    if let Some(w) = warn {
        st.message = Some(w);
    }
    match action {
        Some(0) => save_hint(st),
        Some(1) => close_hint(st),
        _ => {}
    }
}

fn close_hint(st: &mut Answers) {
    st.hint = None;
}

fn save_hint(st: &mut Answers) {
    let m = st.hint.as_ref().unwrap();
    let i = m.index;
    let trap = st.trap_index();
    let custom = js_trim(&m.custom).to_string();
    if m.mcq && st.is_gate(i) {
        st.message = Some(("MCQ not allowed".into(), "Multiple-choice is disabled for Layer-1 unlock questions. Please use letter/length hints or a free-text hint instead.".into()));
        if let Some(m) = &mut st.hint {
            m.mcq = false;
        }
        return;
    }
    if m.mcq {
        let n = m.count.clamp(6, 12);
        let correct = js_trim(&m.correct).to_string();
        if correct.is_empty() {
            st.message = Some(("Missing correct option".into(), "Please enter the correct option for this question.".into()));
            return;
        }
        let decoys: Vec<String> = m.decoys.iter().map(|d| js_trim(d).to_string()).filter(|d| !d.is_empty()).collect();
        if decoys.len() != n - 1 {
            st.message = Some(("MCQ needs more options".into(), format!("Provide {} decoy option{}.", n - 1, if n - 1 > 1 { "s" } else { "" })));
            return;
        }
        let mut all = vec![correct.clone()];
        all.extend(decoys);
        let uniq: std::collections::HashSet<String> = all.iter().map(|a| a.to_lowercase()).collect();
        if uniq.len() != all.len() {
            st.message = Some(("Duplicate options".into(), "Please ensure all options are unique.".into()));
            return;
        }
        st.hints.insert(i, Hint { letters: vec![], custom, show_length: Some(false), mcq_enabled: true, mcq_options: all });
        st.answers[i] = correct;
    } else {
        let effective = if Some(i) == trap { true } else { m.show_length };
        if !effective {
            st.hints.insert(i, Hint { letters: vec![], custom, show_length: Some(false), mcq_enabled: false, mcq_options: vec![] });
        } else {
            let mut seen = std::collections::HashSet::new();
            let mut idx = Vec::new();
            for &sel in &m.letter_sel {
                let v = sel as u32 + 1;
                if !seen.insert(v) {
                    st.message = Some(("Duplicate Letter Chosen".into(), "Please choose different letters for your hints.".into()));
                    return;
                }
                idx.push(v);
            }
            idx.sort();
            st.hints.insert(i, Hint { letters: idx, custom, show_length: Some(effective), mcq_enabled: false, mcq_options: vec![] });
        }
    }
    st.update_preview(i);
    st.update_entropy(i);
    st.hint = None;
    let _ = utf16_len;
}
