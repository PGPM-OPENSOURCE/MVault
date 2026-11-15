//! setup-questions.html: edit the questions, pick exactly two unlock questions.

use crate::app::{Flow, Page, Shared, layer, layer_over};
use crate::pages::answers::Answers;
use crate::pages::common::{button_row, max_content};
use crate::ui::{Btn, CH, InputOpts, LINE, Pg, Span, Ws, WrapOpts, col, s};

pub const DEFAULT_QUESTIONS: [&str; 25] = [
    "What is my full name?",
    "What is the name of my favorite childhood dog?",
    "What is my wife's maiden name?",
    "What city was I born in?",
    "In what city did my wife and I compete in our first Ironman?",
    "What is the name of my wife's favorite childhood dog?",
    "What is the first name of my wife's biological brother?",
    "What is the mascot of the 2nd high school I attended?",
    "What is my father's middle name?",
    "In what city did my wife and I go on our first date?",
    "Journey before... (complete the phrase)",
    "What was the name of my yellow Suzuki V-Strom 650 motorcycle?",
    "What is the name of my childhood best friend?",
    "What was one of my family's Christmas traditions growing up? (Multiple choice)",
    "What is the make and model of the first rifle I ever owned? (Multiple choice)",
    "What is my favorite condiment?",
    "Which of the numbers below are significant to me? (Multiple choice)",
    "What is the 4 digit pin for my phone?",
    "In what year did we have our first child? (Multiple choice)",
    "Which Myers Briggs (MBTI) personality am I? (Multiple choice)",
    "My best score on the English portion of the ACT was...(Multiple choice)",
    "My wife and I met in high school (t/f)",
    "My wife is left-handed (t/f)",
    "I am left-handed (t/f)",
    "I will never forget this date (Multiple choice)",
];

pub const CATEGORIES: [(&str, &str); 12] = [
    ("", "-- Select Category --"), ("home", "Home"), ("family", "Family"), ("life", "Life Events"), ("Pets", "Pets"),
    ("favorites", "Favorites"), ("random", "Random"), ("school", "School"), ("career", "Career"), ("travel", "Travel"),
    ("hobbies", "Hobbies"), ("special", "Special Memories"),
];

pub fn suggestions(cat: &str) -> &'static [&'static str] {
    match cat {
        "home" => &["What was the name of the first street you lived on?", "What color was your childhood home?", "What was the name of your childhood best friend?", "What was the name of your first neighborhood?", "What was the first park you remember playing in?", "What landmark was closest to your childhood home?", "What was the name of your first grade teacher?", "What was your favorite hiding place as a child?", "What tree or plant do you remember from your backyard?", "What was the number of your childhood house?", "What was your favorite game to play outside?", "Who lived across the street from you growing up?", "What kind of fence (or no fence) did your house have?", "What was the name of a nearby lake or river you visited?", "What was your childhood bedroom color?", "What was the nickname of your childhood home?", "What road did you take to school as a kid?", "What local business do you remember near your home?"],
        "family" => &["What is your mother's maiden name?", "What was your grandfather's first name?", "What was your grandmother’s nickname for you?", "Which relative had the most unique hobby?", "What was the city your parents first met in?", "What was your sibling’s childhood nickname?", "What was your family dog’s name growing up?", "Where did your family go for reunions?", "What was your father’s first car?", "What was your mother’s favorite song?", "Who was the oldest cousin in your family?", "What tradition did your family have every year?", "What family recipe was handed down?", "What family member did you resemble most as a child?", "What state did your grandparents retire to?", "What was the middle name of your favorite aunt?", "What holiday was most celebrated in your family?", "Who had the loudest laugh in your family?"],
        "Pets" => &["What was the name of your first pet?", "What species was your first pet?", "What was the nickname you called your first pet?", "What trick did your first pet know?", "Where did you get your first pet from?", "What was your pet’s favorite toy?", "What was your pet's favorite sleeping spot?", "What was the color of your first pet?", "What unusual habit did your pet have?", "What was the breed of your childhood dog or cat?", "What was your pet’s favorite treat?", "What was the pet’s vet’s name?", "Where did your pet like to hide?", "What was the most mischievous thing your pet did?", "What was the name of the pet you wished you had?"],
        "life" => &["What was the first vacation you remember?", "What was your childhood home’s street number?", "What time did your school bus pick you up?", "What was the name of your childhood doctor?", "What chore did you hate the most growing up?", "What was your first broken bone or injury?", "What was the name of your kindergarten teacher?", "What was the first movie you saw in a theater?", "What was your favorite school subject?", "What was your bedtime routine as a child?", "What game did you play most at recess?", "What was your first Halloween costume?", "What was the first major news event you remember?", "What was your favorite childhood weekend activity?", "What was the name of your neighborhood or subdivision?"],
        "favorites" => &["What was your favorite book growing up?", "What was your favorite movie as a teenager?", "What was your favorite band in high school?", "What was your favorite childhood toy?", "What was your favorite sports team as a kid?", "What was your favorite outdoor activity?", "What was your favorite indoor game?", "What was your favorite holiday tradition?", "What was your favorite TV show growing up?", "What was your favorite subject in school?", "What was your favorite place to visit as a child?", "What was your favorite restaurant when you were young?", "What was your favorite color in elementary school?", "What was your favorite cartoon character?", "What was your favorite video game growing up?"],
        "random" => &["What was the name of your imaginary friend?", "What was your favorite hiding spot as a child?", "What was the first movie you remember seeing in a theater?", "What did you want to be when you grew up at age 5?", "What was your favorite board game growing up?", "What unusual pet did you want but never had?", "What was your favorite recess game?", "What costume did you wear for Halloween most often?", "What weird collection did you have as a kid?", "What was your favorite subject in elementary school?", "What fictional world did you wish was real?", "What was the first prize you ever won?", "What secret hiding place did you use as a kid?", "What song reminds you of your childhood?", "What superstition did you believe as a child?"],
        "school" => &["What was the name of your elementary school?", "Who was your favorite high school teacher?", "What was your favorite subject in middle school?", "What sport did you play at school?", "What was your favorite school event?", "Who was your childhood best friend at school?", "What was the mascot of your school?", "What was the first book you remember reading for class?", "What song reminds you of your school days?", "What was your favorite field trip?", "What was the first language class you took?", "What award did you first win at school?", "Who was your first school crush?", "What was the name of your bus driver?", "What was the color of your locker?", "What elective class did you enjoy the most?", "What was your school’s rival team name?", "What lunch item did you always trade?"],
        "career" => &["What was the name of your first boss?", "What was your very first job?", "Where did you earn your first paycheck?", "What was your dream job as a teenager?", "What company gave you your first job offer?", "What was your job title at your first job?", "What was the name of your favorite coworker at your first job?", "What tool or equipment did you first learn to use at work?", "What skill did you first learn on the job?", "What was the biggest mistake you made at your first job?", "What uniform did you wear at your first job?", "Where did you have your first job interview?", "What was the first career you seriously considered?", "What industry was your first job in?", "What was your first promotion at work?"],
        "travel" => &["What was your first vacation destination?", "What was the first country you visited?", "What is your favorite national park?", "What was the name of the first hotel you stayed in?", "Which beach do you remember visiting first?", "Where did you go on your first solo trip?", "What city did you love visiting most as a child?", "What was the first flight number you remember taking?", "Where did you go for a childhood road trip?", "What was the name of the amusement park you visited?", "What mode of transport do you associate with your first trip?", "What’s the name of a memorable airport you flew through?", "Which relative’s house did you visit farthest away?", "What river cruise or boat ride do you remember?", "What was your favorite souvenir from a trip?"],
        "hobbies" => &["What was your favorite childhood hobby?", "What hobby did you spend the most time on as a teenager?", "What instrument did you first learn to play?", "What was the first sport you participated in?", "What craft or art project did you enjoy as a kid?", "What was your favorite type of puzzle growing up?", "What collection did you start as a child?", "What board game did you always want to win?", "What was your favorite thing to build with Legos?", "What outdoor activity did you love most as a kid?", "What was your favorite arcade or video game growing up?", "What book series did you become obsessed with?", "What was your favorite science experiment you tried?", "What dance or performance hobby did you enjoy?", "What hobby have you always wanted to master?"],
        "special" => &["What holiday holds the most meaning to you?", "What tradition is most important to you?", "What event in your life made you feel truly proud?", "What family heirloom means the most to you?", "What was the most memorable gift you ever received?", "What location holds a special place in your heart?", "What song reminds you of home?", "What was your happiest childhood memory?", "What teacher had the biggest impact on your life?", "What was the most meaningful piece of advice you received?", "What special trip do you cherish the most?", "What was your first major personal achievement?", "What keepsake have you held onto the longest?", "What moment made you feel most connected to your family?", "What book or story changed your perspective growing up?"],
        _ => &[],
    }
}

pub struct Edit {
    pub index: usize,
    pub value: String,
    pub category: usize,
    pub open: bool,
}

#[derive(Default)]
pub struct Setup {
    pub questions: Vec<String>,
    pub unlock: Vec<bool>,
    pub edit: Option<Edit>,
    pub help: bool,
    pub message: Option<(String, String)>,
    pub disclaimer: bool,
}

impl Setup {
    pub fn new(flow: &mut Flow) -> Setup {
        let n = flow.custom_question_count.unwrap_or(12);
        flow.question_count = Some(n);
        let questions = match &flow.questions {
            Some(q) if q.len() >= n => q[..n].to_vec(),
            _ => DEFAULT_QUESTIONS[..n.min(25)].iter().map(|s| s.to_string()).collect(),
        };
        Setup { unlock: vec![false; questions.len()], questions, ..Default::default() }
    }
}

/// A `<select>` in the console look: a dim line with a chevron; opens a list below it.
pub fn select(pg: &mut Pg<'_>, key: &str, x: f32, y: f32, w: f32, options: &[&str], selected: &mut usize, open: &mut bool) -> bool {
    let id = pg.wid(key);
    let focused = pg.ui.memory(|m| m.has_focus(id));
    pg.fill(x, y, w, LINE, if focused || *open { col::INPUT_FOCUS } else { col::INPUT });
    pg.text(x + CH, y, options.get(*selected).copied().unwrap_or(""), col::T1);
    // chevron: M2.5 4.5 6 8l3.5-3.5 in a 12x12 box, 1ch from the right
    let c = pg.at(x + w - CH - 12.0, y + (LINE - 12.0) / 2.0);
    let pts = [egui::pos2(c.x + 2.5, c.y + 4.5), egui::pos2(c.x + 6.0, c.y + 8.0), egui::pos2(c.x + 9.5, c.y + 4.5)];
    pg.ui.painter().add(egui::Shape::line(pts.to_vec(), egui::Stroke::new(1.5, col::T2)));
    let r = pg.rect(x, y, w, LINE);
    let resp = pg.ui.interact(r, id, if pg.live { egui::Sense::click() } else { egui::Sense::hover() });
    if resp.hovered() && pg.live {
        pg.ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if resp.clicked() {
        *open = !*open;
    }
    let mut changed = false;
    if *open {
        let top = pg.at(x, y + LINE);
        let area = egui::Area::new(id.with("list")).fixed_pos(top).order(egui::Order::Foreground);
        let mut close = false;
        area.show(pg.ui.ctx(), |ui| {
            let h = options.len() as f32 * LINE;
            let rr = egui::Rect::from_min_size(top, egui::vec2(w, h));
            ui.painter().rect_filled(rr, 0.0, col::DIALOG);
            for (i, o) in options.iter().enumerate() {
                let ir = egui::Rect::from_min_size(egui::pos2(top.x, top.y + i as f32 * LINE), egui::vec2(w, LINE));
                let r = ui.interact(ir, id.with(("opt", i)), egui::Sense::click());
                let hot = r.hovered();
                if hot {
                    ui.painter().rect_filled(ir, 0.0, col::FILL);
                }
                let g = ui.fonts_mut(|f| f.layout_no_wrap(o.to_string(), crate::ui::font(16.0), col::T1));
                let b = g.rows.first().and_then(|r| r.row.glyphs.first().map(|gl| r.pos.y + gl.pos.y)).unwrap_or(0.0);
                ui.painter().galley(egui::pos2(ir.min.x + CH, ir.min.y + 15.0 - b), g, if hot { col::ON_FILL } else { col::T1 });
                if r.clicked() {
                    if *selected != i {
                        *selected = i;
                        changed = true;
                    }
                    close = true;
                }
            }
            ui.allocate_rect(rr, egui::Sense::hover());
        });
        let list_rect = egui::Rect::from_min_size(top, egui::vec2(w, options.len() as f32 * LINE));
        let outside = pg.ui.input(|i| i.pointer.any_click() && i.pointer.interact_pos().is_some_and(|p| !list_rect.contains(p) && !r.contains(p)));
        if close || outside {
            *open = false;
        }
    }
    changed
}

fn help_paras() -> Vec<Vec<Span>> {
    let (t1, t2) = (col::T1, col::T2);
    vec![
        vec![s("Memoro Vault protects your files with two unlock layers.", t1)],
        vec![s("Layer 1 – Quick Check:", t1).underline(), s(" You choose exactly ", t2), s("two questions", t1),
             s(" from your list. These act like your “front door.” If someone guesses wrong, the vault slows them down with increasing wait times after each failed try.", t2)],
        vec![s("Layer 2 – Full Unlock:", t1).underline(), s(" After passing Layer 1, you must answer ", t2), s("all of your remaining questions", t1),
             s(" correctly, in the right order. Only then is the vault able to build the secret key to open your files.", t2)],
        vec![s("Why two layers?", t1).underline(),
             s("\nLayer 1 is the strict gate: no hints, no partial credit — this blocks casual or social-engineering attempts outright.\nLayer 2 is where hints can appear, giving real users assistance while still enforcing full Argon2 + AES-GCM encryption so attackers can’t brute-force their way in.", t2)],
        vec![s("Tip:", t2), s(" Make your answers personal but memorable, and mix in a red herring if you want more protection.", t2)],
    ]
}

pub fn show(st: &mut Setup, sh: &mut Shared, ui: &mut egui::Ui) {
    let dialog = st.disclaimer || st.message.is_some() || st.edit.is_some() || st.help;
    if dialog && !crate::app::has_geom(ui.ctx(), "setup") {
        page(st, sh, ui);
    }
    if st.disclaimer {
        return disclaimer(st, sh, ui);
    }
    if st.message.is_some() {
        return message(st, sh, ui);
    }
    if st.edit.is_some() {
        return edit(st, sh, ui);
    }
    if st.help {
        return help(st, sh, ui);
    }
    page(st, sh, ui);
}

fn page(st: &mut Setup, sh: &mut Shared, ui: &mut egui::Ui) {
    let mut res = None;
    let mut toggled = None;
    let mut edit_i = None;
    let mut help_click = false;
    let checked = st.unlock.iter().filter(|c| **c).count();
    let questions = st.questions.clone();
    let unlock = st.unlock.clone();
    layer(ui, sh, "setup", true, |pg, width| {
        let (x, w) = (8.0, width - 16.0);
        let mut y = 8.0;
        y += pg.para(x, y, w, &[s("Setup Your Security Questions", col::WHITE)], Ws::Normal, WrapOpts::default(), LINE) + LINE;
        // p.instruction: 1px border, three lines, the help link as an inline-block (padding 5px 12px)
        let ix = x + 1.0;
        y += 1.0;
        pg.text(ix, y, "You can use the default questions or edit them.", col::T2);
        y += LINE;
        let mut xx = ix;
        xx += pg.text(xx, y, "Select exactly ", col::T2);
        xx += pg.text(xx, y, "two", col::T1);
        pg.text(xx, y, " questions to act as your first line of defense.", col::T2);
        y += LINE;
        let (bx, by) = (ix, y + 8.0);
        let lw = pg.measure("How does unlocking work?", 16.0);
        let (bw, bh) = (lw + 26.0, 31.0);
        let hov = pg.hovered(bx, by, bw, bh);
        if hov {
            pg.fill(bx, by, bw, bh, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 18));
            pg.ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        pg.text(bx + 13.0, by + 6.0, "How does unlocking work?", if hov { col::T1 } else { col::T2 });
        let r = pg.rect(bx, by, bw, bh);
        if pg.ui.interact(r, pg.wid("help"), egui::Sense::click()).clicked() {
            help_click = true;
        }
        y = by + bh + 24.0 + 1.0;
        // question rows: "n." question  [Edit]  [ ]  (shrink-to-fit, 12px gaps, a blank line apart)
        for (i, q) in questions.iter().enumerate() {
            let num = format!("{}.", i + 1);
            let spans = [s(&num, col::T3).margin_r(4.0), s(format!(" {q}"), col::T1)];
            let tail = 12.0 + pg.measure("[Edit]", 16.0) + 12.0 + 4.0 * CH;
            let max_text = (w - 2.0 - tail).max(CH * 4.0);
            let lines = crate::ui::wrap(&pg.ctx(), &spans, max_text, Ws::Normal, WrapOpts { anywhere: false });
            let text_w = lines.iter().map(|l| l.width).fold(0.0, f32::max);
            let rows = lines.len().max(1);
            let row_h = rows as f32 * LINE;
            let rx = x + 1.0;
            let ry = y + 1.0;
            pg.para(rx, ry, max_text, &spans, Ws::Normal, WrapOpts::default(), LINE);
            let tw = text_w.min(max_text);
            let mid = ry + (row_h - LINE) / 2.0;
            let ex = rx + tw + 12.0;
            if pg.button(("edit", i), ex, mid, Btn::new("Edit")).clicked() {
                edit_i = Some(i);
            }
            let cx = ex + pg.measure("[Edit]", 16.0) + 12.0;
            let enabled = unlock[i] || checked < 2;
            if pg.checkbox(("unlock", i), cx, mid, unlock[i], enabled).clicked() && enabled {
                toggled = Some(i);
            }
            y += row_h + 2.0 + LINE;
        }
        let y = y + 22.0; // .button-container margin-top, after the last row's 19px margin
        let (c, h) = button_row(pg, "nav", x, y, w, &[Btn::new("Back"), Btn::new("Continue").main()]);
        res = c;
        y + h + 8.0
    });
    if let Some(i) = toggled {
        st.unlock[i] = !st.unlock[i];
    }
    if help_click {
        st.help = true;
    }
    if let Some(i) = edit_i {
        st.edit = Some(Edit { index: i, value: st.questions[i].clone(), category: 0, open: false });
    }
    match res {
        Some(0) => sh.nav = Some(crate::app::Nav::Back),
        Some(1) => {
            if st.unlock.iter().filter(|c| **c).count() != 2 {
                st.message = Some(("Selection Error".into(), "Please select exactly TWO unlock questions.".into()));
            } else {
                st.disclaimer = true;
            }
        }
        _ => {}
    }
}

fn dialog_w(pg: &Pg<'_>, width: f32, blocks: &[Vec<Span>], extra: f32) -> f32 {
    max_content(pg, blocks).max(extra).min(width - 18.0)
}

fn help(st: &mut Setup, sh: &mut Shared, ui: &mut egui::Ui) {
    let mut close = false;
    let mut content_rect = None;
    layer_over(ui, sh, "setup-help", Some("setup"), true, |pg, width| {
        let paras = help_paras();
        let w = dialog_w(pg, width, &paras, 0.0);
        let (x, mut y) = (9.0, 9.0);
        y += pg.para(x, y, w, &[s("How Unlocking Works", col::WHITE)], Ws::Normal, WrapOpts::default(), LINE) + LINE;
        for (i, p) in paras.iter().enumerate() {
            if i > 0 {
                y += 16.0;
            }
            y += pg.para(x, y, w, p, Ws::PreLine, WrapOpts::default(), LINE);
        }
        y += 18.0;
        if pg.button("close", x, y, Btn::new("Close")).clicked() {
            close = true;
        }
        content_rect = Some(pg.rect(8.0, 8.0, w + 2.0, y + LINE + 1.0 - 8.0));
        y + LINE + 9.0
    });
    // a click on the overlay outside the dialog, or Esc, closes it
    let outside = ui.input(|i| i.pointer.primary_clicked()) && ui.input(|i| i.pointer.interact_pos()).zip(content_rect).is_some_and(|(p, r)| !r.contains(p));
    if close || outside || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        st.help = false;
    }
}

fn message(st: &mut Setup, sh: &mut Shared, ui: &mut egui::Ui) {
    let (title, text) = st.message.clone().unwrap();
    let mut ok = false;
    layer_over(ui, sh, "setup-msg", Some("setup"), true, |pg, width| {
        let w = dialog_w(pg, width, &[vec![s(&title, col::WHITE)], vec![s(&text, col::T1)]], 0.0);
        let (x, mut y) = (9.0, 9.0);
        y += pg.para(x, y, w, &[s(&title, col::WHITE)], Ws::Normal, WrapOpts::default(), LINE) + LINE;
        y += pg.para(x, y, w, &[s(&text, col::T1)], Ws::PreLine, WrapOpts::default(), LINE) + 18.0;
        ok = pg.button("ok", x, y, Btn::new("OK")).clicked();
        y + LINE + 9.0
    });
    if ok {
        st.message = None;
    }
}

fn disclaimer(st: &mut Setup, sh: &mut Shared, ui: &mut egui::Ui) {
    let text = "Your seed and memory answers will be handled only on this device.\n\nNothing is uploaded, transmitted, or stored externally.\n\nSensitive data is automatically erased after 30 minutes or after building your vault.\n\nPlease use a secure device and environment.";
    let mut go = false;
    layer_over(ui, sh, "setup-disc", Some("setup"), true, |pg, width| {
        let w = dialog_w(pg, width, &text.split('\n').map(|l| vec![s(l, col::T2)]).collect::<Vec<_>>(), 0.0);
        let (x, mut y) = (9.0, 9.0);
        y += pg.para(x, y, w, &[s("Before You Continue", col::WHITE)], Ws::Normal, WrapOpts::default(), LINE) + LINE;
        y += pg.para(x, y, w, &[s(text, col::T2)], Ws::PreLine, WrapOpts::default(), LINE) + 18.0;
        go = pg.button("go", x, y, Btn::new("I Understand — Continue")).clicked();
        y + LINE + 9.0
    });
    if go {
        st.disclaimer = false;
        sh.flow.questions = Some(st.questions.clone());
        sh.flow.unlock_questions = Some(st.unlock.iter().enumerate().filter(|(_, c)| **c).map(|(i, _)| i).collect());
        let a = Answers::new(&sh.flow);
        sh.go(Page::Answers(a));
    }
}

fn edit(st: &mut Setup, sh: &mut Shared, ui: &mut egui::Ui) {
    let mut res = None;
    let e = st.edit.as_mut().unwrap();
    let mut value = std::mem::take(&mut e.value);
    let mut cat = e.category;
    let mut open = e.open;
    let mut picked: Option<&'static str> = None;
    layer_over(ui, sh, "setup-edit", Some("setup"), true, |pg, width| {
        let list: &[&str] = suggestions(CATEGORIES[cat].0);
        let sugg_w = |pg: &Pg<'_>, q: &str| pg.measure(q, 14.0) + 2.0 * CH + 2.0;
        let list_w = list.iter().map(|q| sugg_w(pg, q)).fold(0.0, f32::max) + if list.len() as f32 * 27.0 > 150.0 { 6.0 } else { 0.0 };
        let select_w = pg.measure("-- Select Category --", 16.0) + 4.0 * CH;
        let w = dialog_w(pg, width, &[vec![s("Edit Question", col::WHITE)]], list_w.max(select_w).max(crate::pages::dashboard::INPUT_INTRINSIC));
        let (x, mut y) = (9.0, 9.0);
        y += pg.para(x, y, w, &[s("Edit Question", col::WHITE)], Ws::Normal, WrapOpts::default(), LINE) + LINE;
        pg.input("q", x, y, w, &mut value, "", InputOpts::line());
        y += LINE + 20.0;
        pg.text(x, y, "SUGGESTIONS:", col::T3);
        y += LINE + 6.0;
        let names: Vec<&str> = CATEGORIES.iter().map(|c| c.1).collect();
        let sel_y = y;
        y += LINE + 15.0;
        if !list.is_empty() {
            let h = (list.len() as f32 * 27.0).min(150.0);
            pg.scroll_box("sugg", x, y, w, h, |pg, _bw| {
                let mut yy = 5.0;
                for (i, q) in list.iter().enumerate() {
                    let bw = sugg_w(pg, q);
                    let r = pg.rect(0.0, yy, bw, 22.0);
                    let resp = pg.ui.interact(r, pg.wid(("s", i)), egui::Sense::click());
                    let hot = resp.hovered();
                    if hot {
                        pg.fill(0.0, yy, bw, 22.0, col::FILL);
                        pg.ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    let c = if hot { col::ON_FILL } else { col::T1 };
                    let base = yy + 1.0 + 15.0;
                    let mut xx = 1.0;
                    xx += pg.text_base(xx, base, "[", 16.0, c);
                    xx += pg.text_base(xx, base, q, 14.0, c);
                    pg.text_base(xx, base, "]", 16.0, c);
                    if resp.clicked() {
                        picked = Some(q);
                    }
                    yy += 27.0;
                }
                yy
            });
            y += h + 18.0;
        } else {
            y += 18.0 - 15.0;
        }
        let (c, h) = button_row(pg, "eb", x, y, w, &[Btn::new("Save").main(), Btn::new("Cancel")]);
        res = c;
        // the select's list opens over everything below it, so draw it last
        if select(pg, "cat", x, sel_y, w, &names, &mut cat, &mut open) {
            pg.ui.ctx().request_repaint();
        }
        y + h + 9.0
    });
    if let Some(q) = picked {
        value = q.to_string();
    }
    let e = st.edit.as_mut().unwrap();
    e.value = value;
    e.category = cat;
    e.open = open;
    match res {
        Some(0) => {
            let e = st.edit.take().unwrap();
            let q = mvault_core::jsstr::js_trim(&e.value).to_string();
            if !q.is_empty() {
                st.questions[e.index] = q;
                sh.flow.questions = Some(st.questions.clone());
            }
        }
        Some(1) => st.edit = None,
        _ => {}
    }
}
