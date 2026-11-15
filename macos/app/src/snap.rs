//! Snapshot mode: `mvault --snap <scenario> --out <dir>` renders one screen in a known state,
//! saves `<scenario>.png` and `<scenario>.json` (every text run and box drawn, in the same shape
//! as the Electron reference dumps), and quits. Used to compare the Rust UI with the original.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::json;

use crate::app::{Page, Shared};
use crate::pages::{answers, build, create, dashboard, how, index, recover, setup};

pub struct Snap {
    pub scenario: String,
    pub out: PathBuf,
    /// (layer key, offset): the layers the reference capture had scrolled, and how far
    pub scroll: Vec<(&'static str, f32)>,
    /// some scenarios need time, not just frames (media loading and playing)
    min_secs: f64,
    frame: u32,
    saved: Arc<AtomicBool>,
}

const WARM_FRAMES: u32 = 8;

impl Snap {
    pub fn new(scenario: String, out: PathBuf) -> Snap {
        let scroll = scroll_for(&scenario);
        let min_secs = if scenario.starts_with("viewer-file:") { 2.5 } else { 0.0 };
        Snap { scenario, out, scroll, min_secs, frame: 0, saved: Arc::new(AtomicBool::new(false)) }
    }
    /// The file name the outputs get (a viewer-file:<path> scenario is named after the file).
    fn name(&self) -> String {
        match self.scenario.strip_prefix("viewer-file:") {
            Some(p) => format!("viewer-{}", p.rsplit('/').next().unwrap_or("file")),
            None => self.scenario.clone(),
        }
    }
    pub fn recording(&self) -> bool {
        self.frame == WARM_FRAMES
    }
}

/// Scenarios the reference capture scrolled: the layers and their offsets (END = as far as it goes).
fn scroll_for(scenario: &str) -> Vec<(&'static str, f32)> {
    const END: f32 = 1e7;
    match scenario {
        "index-safety-bottom" => vec![("safety", END)],
        "setup-bottom" => vec![("setup", END)],
        "answers-bottom" => vec![("answers", END)],
        "answers-scroll1" => vec![("answers", 600.0)],
        "answers-scroll2" => vec![("answers", 1200.0)],
        "answers-scroll3" => vec![("answers", 1800.0)],
        "how-scroll1" => vec![("how", 600.0)],
        "how-scroll2" => vec![("how", 1200.0)],
        "recover-scroll1" => vec![("recover", 600.0)],
        "recover-scroll2" => vec![("recover", 1200.0)],
        // filling the last answer scrolled it into view
        "recover-filled" => vec![("recover", 500.0)],
        // [Recover Vault] sits at the bottom, so the page stays scrolled there underneath
        "recover-done-bottom" => vec![("recover", END), ("seed", END)],
        "editor-bottom" => vec![("recover", END), ("editor", END)],
        "build-post-bottom" => vec![("post", END)],
        sc if sc.starts_with("recover-done") || sc == "recover-toast" || sc.starts_with("viewer") || sc.starts_with("editor") => vec![("recover", END)],
        _ => vec![],
    }
}

/// The recover page for the fixture vault, unlocked with its two gate answers.
fn fixture_recover() -> Option<recover::Recover> {
    let (_, bytes) = fixture_vault()?;
    let vz = mvault_core::vault::read_vault_zip(&bytes).ok()?;
    let a1 = mvault_core::jsstr::normalize_answer(ANSWERS[GATES[0]]);
    let a2 = mvault_core::jsstr::normalize_answer(ANSWERS[GATES[1]]);
    let concat = zeroize::Zeroizing::new(format!("{a1}{a2}"));
    let l1 = mvault_core::vault::decrypt_layer1(&concat, &vz.vault_json, &vz.vault_enc).ok()?;
    Some(recover::Recover::new(recover::Handoff { zip: Arc::new(bytes), vault_json: vz.vault_json.clone(), layer1: l1, unlock_concat: concat }))
}

pub const GATES: [usize; 2] = [0, 3];
pub const TRAP: usize = 8;

/// The fixture vault the reference capture built (known answers), for scenarios that need one.
pub fn fixture_vault() -> Option<(String, Vec<u8>)> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../reference/pgpm/_fixture-vault.zip");
    std::fs::read(p).ok().map(|b| ("Family Vault.zip".to_string(), b))
}

pub const QUESTIONS: [&str; 12] = [
    "What was the name of your childhood best friend?", "What was the street name where you grew up?",
    "What is your oldest sibling’s middle name?", "What was the name of your first pet?",
    "What city were you born in?", "What is the name of your favorite childhood teacher?",
    "What was the make of your first car?", "What is the middle name of one of your grandparents?",
    "What is the title of your favorite childhood book?", "What was the mascot of your high school?",
    "What was your favorite vacation spot as a kid?", "What was the name of your first boss?",
];
pub const ANSWERS: [&str; 12] = ["Tomasz Wiley", "Juniper Lane", "Everett", "Biscuit", "Springfield", "Mrs Halvorsen",
    "Subaru", "Augustine", "Hatchet", "Falcons", "Grandma Ruth’s lake house", "Ruth Ann"];

/// The answers page's inputs as the reference capture set them.
fn answers_flow(sh: &mut Shared) {
    sh.flow.questions = Some(QUESTIONS.iter().map(|q| q.to_string()).collect());
    sh.flow.question_count = Some(12);
    sh.flow.unlock_questions = Some(vec![0, 3]);
}

/// The builder's inputs from reference/pgpm/_fixture-inputs.json.
fn build_flow(sh: &mut Shared) {
    use base64::Engine;
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../reference/pgpm/_fixture-inputs.json");
    let Ok(v) = std::fs::read(p).map(|b| serde_json::from_slice::<serde_json::Value>(&b).unwrap()) else { return };
    let ud = &v["userData"];
    let answers: Vec<String> = ud["answers"].as_array().unwrap().iter().map(|a| a.as_str().unwrap().to_string()).collect();
    sh.flow.user_data = Some(mvault_core::build::UserData {
        space_indexes: answers.iter().map(|a| mvault_core::jsstr::space_indexes(a)).collect(),
        answers,
        questions: QUESTIONS.iter().map(|q| q.to_string()).collect(),
        unlock_indexes: [0, 3],
        final_message: ud["finalMessage"].as_str().unwrap().into(),
        trap_index: Some(8),
        ..Default::default()
    });
    sh.flow.named_seed_files = v["namedSeedFiles"].as_array().unwrap().iter().map(|s| mvault_core::vault::Seed { name: s["name"].as_str().unwrap().into(), content: s["content"].as_str().unwrap().into() }).collect();
    sh.flow.uploads = v["uploads"].as_array().unwrap().iter().map(|u| answers::UploadedFile {
        id: u["id"].as_str().unwrap().into(), name: u["name"].as_str().unwrap().into(), mime: u["type"].as_str().unwrap().into(),
        bytes: std::sync::Arc::new(base64::engine::general_purpose::STANDARD.decode(u["bytes"].as_str().unwrap()).unwrap()),
    }).collect();
}

pub fn setup(scenario: &str, sh: &mut Shared) -> Page {
    let with_vault = || {
        let mut d = dashboard::Dashboard::default();
        if let Some((n, b)) = fixture_vault() {
            dashboard::import_bytes(&mut d, n, b);
            d.message = None;
        }
        d
    };
    match scenario {
        "index" => Page::Index(index::Index::default()),
        "index-safety" | "index-safety-bottom" => Page::Index(index::Index { safety: true }),
        "dashboard" => Page::Dashboard(dashboard::Dashboard::default()),
        "dashboard-vault" => Page::Dashboard(with_vault()),
        "dashboard-imported" => {
            let mut d = with_vault();
            d.message = Some(("Success".into(), "Vault imported successfully!".into()));
            Page::Dashboard(d)
        }
        "dashboard-menu" => {
            let mut d = with_vault();
            d.menu = d.vaults.first().map(|v| v.id);
            Page::Dashboard(d)
        }
        "dashboard-offline" => Page::Dashboard(dashboard::Dashboard { offline: Some(dashboard::Next::Create), ..Default::default() }),
        "dashboard-unlock" => {
            let mut d = with_vault();
            let id = d.vaults[0].id;
            d.unlock = Some(dashboard::Unlock { vault: id, a1: String::new(), a2: String::new(), opening: false, pending: None, focus_once: true });
            Page::Dashboard(d)
        }
        "dashboard-lockout" => {
            let mut d = with_vault();
            let id = d.vaults[0].id;
            d.unlock = Some(dashboard::Unlock { vault: id, a1: "wrong".into(), a2: "answers".into(), opening: false, pending: None, focus_once: false });
            d.lockouts.insert(id, dashboard::Lockout { failed: 1, until: dashboard::now_ms() + 13_500.0, total: 15_000.0 });
            Page::Dashboard(d)
        }
        "dashboard-rename" => {
            let mut d = with_vault();
            let v = &d.vaults[0];
            d.rename = Some((v.id, v.name.clone()));
            Page::Dashboard(d)
        }
        "dashboard-confirm" => {
            let mut d = with_vault();
            let id = d.vaults[0].id;
            d.confirm = Some(("Are you sure you want to delete this vault?".into(), id));
            Page::Dashboard(d)
        }
        "dashboard-paper" => Page::Dashboard(dashboard::Dashboard { paper: Some(crate::pages::paper::PaperModal::default()), ..Default::default() }),
        "dashboard-paper-restored" => {
            // the reference capture fed its printed backup (copied as 1-memoro-lite-QR.pdf) to the dialog
            let mut d = dashboard::Dashboard::default();
            let mut pm = crate::pages::paper::PaperModal::default();
            let pdf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../reference/pgpm/_fixture-lite-QR.pdf");
            if let Some((n, b)) = pm.snap_restore(pdf, "1-memoro-lite-QR.pdf", sh) {
                dashboard::import_bytes(&mut d, n, b);
            }
            d.paper = Some(pm);
            Page::Dashboard(d)
        }
        "how" | "how-scroll1" | "how-scroll2" => Page::How(how::How),
        "create" => Page::Create(create::Create { count: 14 }),
        "create-20" => Page::Create(create::Create { count: 20 }),
        "setup" | "setup-checked" | "setup-bottom" | "setup-help" | "setup-edit" | "setup-edit-suggest" | "setup-msg" | "setup-disclaimer" => {
            sh.flow.custom_question_count = Some(12);
            let mut st = setup::Setup::new(&mut sh.flow);
            if scenario != "setup" {
                st.unlock[0] = true;
                st.unlock[3] = true;
            }
            match scenario {
                "setup-help" => st.help = true,
                "setup-edit" => st.edit = Some(setup::Edit { index: 1, value: st.questions[1].clone(), category: 0, open: false }),
                "setup-edit-suggest" => st.edit = Some(setup::Edit { index: 1, value: st.questions[1].clone(), category: 4, open: false }),
                "setup-msg" => {
                    st.unlock[3] = false;
                    st.message = Some(("Selection Error".into(), "Please select exactly TWO unlock questions.".into()));
                }
                "setup-disclaimer" => st.disclaimer = true,
                _ => {}
            }
            Page::Setup(st)
        }
        sc if sc.starts_with("answers") => {
            answers_flow(sh);
            let mut st = answers::Answers::new(&sh.flow);
            if sc != "answers" {
                st.snap_fill(&ANSWERS, Some(8));
            }
            match sc {
                "answers-hint" => st.snap_modal("hint", 4),
                "answers-trap" => st.snap_modal("trap", 0),
                "answers-msg" => st.snap_modal("msg", 0),
                _ => {}
            }
            Page::Answers(st)
        }
        "build-picker" | "build-picker-checked" | "build-post" | "build-post-bottom" => {
            build_flow(sh);
            Page::Build(build::Build::snap(scenario.trim_start_matches("build-").trim_end_matches("-bottom"), &sh.flow))
        }
        sc if sc.starts_with("viewer-file:") => {
            // any file in the viewer, played if it is audio or video
            let Some(mut r) = fixture_recover() else { std::process::exit(2) };
            r.snap(sc, &ANSWERS, TRAP, &[]);
            Page::Recover(r)
        }
        sc if sc.starts_with("recover") || sc.starts_with("viewer") || sc.starts_with("editor") => {
            let Some(mut r) = fixture_recover() else {
                eprintln!("the fixture vault did not open");
                std::process::exit(2);
            };
            let state = match sc {
                "recover-rh" => "rh".to_string(),
                "recover" | "recover-scroll1" | "recover-scroll2" => "form".into(),
                "recover-filled" => "filled".into(),
                "recover-pow" => "pow".into(),
                "recover-done" | "recover-done-bottom" => "done".into(),
                "recover-toast" => "toast".into(),
                "viewer-image" => "viewer:cabin-sunset.png".into(),
                "viewer-pdf" => "viewer:letter-of-instruction.pdf".into(),
                "viewer-text" => "viewer:accounts.txt".into(),
                "viewer-seed" => "viewer:btc_cold_storage.txt".into(),
                "editor-bottom" => "editor".into(),
                other => other.to_string(),
            };
            // the two capture runs shuffled the multiple-choice options differently
            let second_run = sc.starts_with("editor") || sc == "viewer-seed" || sc == "recover-toast";
            let order: &[&str] = if second_run { &["Volvo", "Honda", "Ford", "Subaru", "Toyota", "Jeep"] } else { &["Honda", "Ford", "Subaru", "Toyota", "Volvo", "Jeep"] };
            r.snap(&state, &ANSWERS, TRAP, order);
            Page::Recover(r)
        }
        other => {
            eprintln!("unknown scenario {other}");
            std::process::exit(2);
        }
    }
}

/// Called at the end of every frame in snapshot mode.
pub fn tick(sh: &mut Shared, ctx: &egui::Context) {
    let time = sh.time;
    let Some(snap) = &mut sh.snap else { return };
    if snap.frame + 1 == WARM_FRAMES && time < snap.min_secs {
        ctx.request_repaint();
        return;
    }
    snap.frame += 1;
    ctx.request_repaint();
    if snap.frame == WARM_FRAMES + 1 {
        // the recorder ran during the last frame: write it, then grab that frame
        std::fs::create_dir_all(&snap.out).ok();
        let dump = json!({
            "url": snap.scenario, "vw": ctx.content_rect().width(), "vh": ctx.content_rect().height(),
            "dpr": ctx.pixels_per_point(), "texts": sh.rec.texts, "boxes": sh.rec.boxes,
        });
        std::fs::write(snap.out.join(format!("{}.json", snap.name())), serde_json::to_vec(&dump).unwrap()).ok();
        let path = snap.out.join(format!("{}.png", snap.name()));
        let saved = snap.saved.clone();
        let ctx2 = ctx.clone();
        ctx.request_screenshot(move |img| {
            let (w, h) = (img.size[0] as u32, img.size[1] as u32);
            let mut buf = Vec::with_capacity((w * h * 4) as usize);
            for p in &img.pixels {
                buf.extend_from_slice(&[p.r(), p.g(), p.b(), 255]);
            }
            if let Some(im) = image::RgbaImage::from_raw(w, h, buf) {
                im.save(&path).ok();
            }
            saved.store(true, Ordering::SeqCst);
            ctx2.request_repaint();
        });
    }
    if snap.saved.load(Ordering::SeqCst) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}
