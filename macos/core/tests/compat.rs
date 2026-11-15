//! Compatibility with the Electron MVault / Memoro Vault, in both directions.
//!
//! * the vault and question layer the Electron app built (reference/pgpm/_fixture-*) open here
//!   and match what went in, byte for byte;
//! * vaults sealed and re-sealed here open with the Electron app's own vault-core.js and WASM
//!   engine (run under Node), recovered the way recover.js does it;
//! * the strength estimate matches the zxcvbn.js the app ships.
//!
//! The Node half is skipped (with a note) when Node or the Electron checkout is missing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use base64::Engine;
use mvault_core::build::{self, Hint, UserData};
use mvault_core::jsstr::{normalize_answer, space_indexes};
use mvault_core::recover;
use mvault_core::vault::{self, Seed, Session, Upload};
use serde_json::{Value, json};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

fn reference() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../reference/pgpm")
}

/// The Electron MVault checkout: $MVAULT_REPO, the folder this project sits in (when it lives
/// inside the MVault repository), or a sibling `MVault` folder.
fn mvault_repo() -> Option<PathBuf> {
    let ws = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    [std::env::var("MVAULT_REPO").ok().map(PathBuf::from), Some(ws.join("..")), Some(ws.join("../MVault"))]
        .into_iter()
        .flatten()
        .find(|p| p.join("test/helpers/env.cjs").exists() && p.join("src/libs/zxcvbn.js").exists())
}

fn node() -> Option<PathBuf> {
    mvault_repo()?;
    let candidates = [
        std::env::var("MVAULT_NODE").ok().map(PathBuf::from),
        std::env::var("NVM_BIN").ok().map(|b| PathBuf::from(b).join("node")),
        Some(PathBuf::from("node")),
    ];
    candidates.into_iter().flatten().find(|p| Command::new(p).arg("--version").output().is_ok_and(|o| o.status.success()))
}

fn node_cmd(node: &Path, script: &str) -> Command {
    let mut c = Command::new(node);
    c.arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join(script));
    c.env("MVAULT_REPO", mvault_repo().unwrap_or_default());
    c
}

struct Fixture {
    user: UserData,
    uploads: Vec<Upload>,
    seeds: Vec<Seed>,
    raw_hints: Value,
}

fn fixture() -> Fixture {
    let v: Value = serde_json::from_slice(&std::fs::read(reference().join("_fixture-inputs.json")).expect("run reference/capture.cjs first")).unwrap();
    let ud = &v["userData"];
    let strs = |x: &Value| x.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect::<Vec<_>>();
    let answers = strs(&ud["answers"]);
    let mut hints = BTreeMap::new();
    for (k, h) in ud["hints"].as_object().unwrap() {
        hints.insert(k.parse().unwrap(), Hint {
            letters: h["letters"].as_array().map(|a| a.iter().map(|n| n.as_u64().unwrap() as u32).collect()).unwrap_or_default(),
            custom: h["custom"].as_str().unwrap_or("").into(),
            show_length: h["showLength"].as_bool(),
            mcq_enabled: h["mcqEnabled"].as_bool().unwrap_or(false),
            mcq_options: h.get("mcqOptions").map(strs).unwrap_or_default(),
        });
    }
    let user = UserData {
        space_indexes: answers.iter().map(|a| space_indexes(a)).collect(),
        answers,
        hints,
        questions: strs(&ud["questions"]),
        unlock_indexes: [ud["unlockIndexes"][0].as_u64().unwrap() as usize, ud["unlockIndexes"][1].as_u64().unwrap() as usize],
        final_message: ud["finalMessage"].as_str().unwrap().into(),
        trap_index: ud["trapIndex"].as_u64().map(|t| t as usize),
    };
    let uploads = v["uploads"].as_array().unwrap().iter().map(|u| Upload {
        name: u["name"].as_str().unwrap().into(),
        mime: u["type"].as_str().unwrap().into(),
        bytes: Arc::new(B64.decode(u["bytes"].as_str().unwrap()).unwrap()),
    }).collect();
    let seeds = v["namedSeedFiles"].as_array().unwrap().iter().map(|s| Seed { name: s["name"].as_str().unwrap().into(), content: s["content"].as_str().unwrap().into() }).collect();
    Fixture { user, uploads, seeds, raw_hints: ud["hints"].clone() }
}

fn normalized(user: &UserData) -> Vec<String> {
    user.answers.iter().enumerate().map(|(i, a)| if Some(i) == user.trap_index { String::new() } else { normalize_answer(a) }).collect()
}

/// mcqOptions are shuffled at build time: compare them as sets.
fn without_mcq_order(list: &[Value]) -> Vec<Value> {
    list.iter().map(|q| {
        let mut q = q.clone();
        if let Some(Value::Array(opts)) = q.get_mut("mcqOptions") {
            opts.sort_by_key(|o| o.to_string());
        }
        q
    }).collect()
}

#[test]
fn opens_vault_built_by_electron_app() {
    let f = fixture();
    let zip = std::fs::read(reference().join("_fixture-vault.zip")).unwrap();
    let vz = vault::read_vault_zip(&zip).unwrap();
    let a = normalized(&f.user);
    let unlock = format!("{}{}", a[f.user.unlock_indexes[0]], a[f.user.unlock_indexes[1]]);
    let l1 = vault::decrypt_layer1(&unlock, &vz.vault_json, &vz.vault_enc).unwrap();
    assert_eq!(l1.gates, vec![0, 3]);
    assert_eq!(l1.selected.len(), 6);
    assert!(!l1.selected.contains(&8), "the red herring is kept out of the hidden order");

    // the question layer JS wrote equals what build_question_list writes for the same inputs
    assert_eq!(without_mcq_order(&l1.question_list), without_mcq_order(&build::build_question_list(&f.user)));

    let mut attempts = 0;
    let un = recover::find_order(&vz.vault_json, vz.meta_bytes(), &l1, &|i| a[i].clone(),
        &mut |p| if let recover::Progress::Attempt { n, .. } = p { attempts = n }, &|| false).unwrap();
    assert_eq!(un.attempt, attempts);
    let files = recover::decrypt_files(&vz, &un.meta, &un.permuted_concat, &mut |_| {});
    assert_eq!(files.len(), un.meta.files.len());
    let c = vault::split_contents(&un.meta, files);
    assert_eq!(c.uploads.iter().map(|u| u.name.clone()).collect::<Vec<_>>(), f.uploads.iter().map(|u| u.name.clone()).collect::<Vec<_>>());
    for (got, want) in c.uploads.iter().zip(&f.uploads) {
        assert_eq!(got.bytes, want.bytes, "{}", want.name);
        assert_eq!(got.mime, want.mime);
    }
    assert_eq!(c.seeds, f.seeds);
    assert_eq!(c.final_message, f.user.final_message);
    eprintln!("electron vault opened on attempt {} of 720; hints {}", un.attempt, f.raw_hints);
}

fn run_js_open(node: &Path, zip: &[u8], expected: &Value, tag: &str) -> Value {
    let dir = std::env::temp_dir().join(format!("mvault-compat-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let zp = dir.join("vault.zip");
    let ep = dir.join("expected.json");
    std::fs::write(&zp, zip).unwrap();
    std::fs::write(&ep, serde_json::to_vec(expected).unwrap()).unwrap();
    let out = node_cmd(node, "js_open.cjs").arg(&zp).arg(&ep).output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "vault-core.js could not open the {tag} vault:\n{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn electron_app_opens_vaults_sealed_here() {
    let Some(node) = node() else {
        eprintln!("skipped: Node or the MVault (Electron) checkout not available; set MVAULT_REPO");
        return;
    };
    let f = fixture();
    let a = normalized(&f.user);

    // 1) a new vault, as build-vault does
    let plan = build::plan(&f.user);
    let full = build::seal_full(&plan, &f.uploads, &f.seeds, &f.user.final_message, None).unwrap();
    let zip = vault::package_zip(&full, true).unwrap();
    let expected = json!({
        "answers": a,
        "gates": [0, 3],
        "uploads": f.uploads.iter().map(|u| json!({"name": u.name, "b64": B64.encode(u.bytes.as_slice())})).collect::<Vec<_>>(),
        "seeds": f.seeds.iter().map(|s| json!({"name": s.name, "content": s.content})).collect::<Vec<_>>(),
        "finalMessage": f.user.final_message,
    });
    let r = run_js_open(&node, &zip, &expected, "built");
    eprintln!("JS opened the Rust vault on attempt {} ({} files)", r["attempt"], r["files"]);
    assert_eq!(r["vaultJsonKeys"], json!(["version", "created", "instructions", "fullSalt", "layer1Salt", "questionPrompts", "vaultMetaIv", "argonSettings"]));
    assert_eq!(without_mcq_order(r["questionList"].as_array().unwrap()), without_mcq_order(&plan.layer1.question_list));

    // 2) the same vault edited and re-sealed, as the vault editor does
    let vz = vault::read_vault_zip(&zip).unwrap();
    let l1 = vault::decrypt_layer1(&plan.unlock_concat, &vz.vault_json, &vz.vault_enc).unwrap();
    let un = recover::find_order(&vz.vault_json, vz.meta_bytes(), &l1, &|i| a[i].clone(), &mut |_| {}, &|| false).unwrap();
    let session = Session {
        vault_json: vz.vault_json.clone(),
        vault_enc: vz.vault_enc.clone(),
        unlock_concat: plan.unlock_concat.clone(),
        gate_answers: l1.gates.iter().map(|&i| a[i].clone()).collect(),
        remaining_answers: l1.remaining.iter().map(|&i| a[i].clone()).collect(),
        selected_answers: un.permutation.clone(),
    };
    let mut uploads = f.uploads.clone();
    uploads[0].name = "renamed.pdf".into();
    uploads.push(Upload { name: "accounts.txt".into(), mime: "text/plain".into(), bytes: Arc::new(b"a second accounts.txt".to_vec()) });
    let seeds = vec![Seed { name: "eth_wallet.txt".into(), content: "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong".into() }];
    let extras = json!({ "revision": 2, "modified": "2026-11-15T10:00:00.000Z" });
    let resealed = vault::reseal_vault(&session, &uploads, &seeds, "  edited message \n", extras.as_object().cloned(), None).unwrap();
    assert_eq!(resealed.upload_names.last().unwrap(), "accounts (1).txt");
    let expected = json!({
        "answers": a,
        "gates": [0, 3],
        "uploads": uploads.iter().zip(&resealed.upload_names).map(|(u, n)| json!({"name": n, "b64": B64.encode(u.bytes.as_slice())})).collect::<Vec<_>>(),
        "seeds": [{"name": "eth_wallet.txt", "content": seeds[0].content}],
        "finalMessage": "edited message",
        "metaExtras": extras,
    });
    let r = run_js_open(&node, &resealed.zip_bytes, &expected, "resealed");
    eprintln!("JS opened the re-sealed vault on attempt {}", r["attempt"]);
}

#[test]
fn strength_estimate_matches_zxcvbn_js() {
    let Some(node) = node() else {
        eprintln!("skipped: Node or the MVault (Electron) checkout not available; set MVAULT_REPO");
        return;
    };
    let samples = [
        "", "a", "password", "tomasz wiley", "juniper lane", "everett", "biscuit", "springfield", "mrs halvorsen",
        "subaru", "augustine", "hatchet", "falcons", "grandma ruth’s lake house", "ruth ann", "correct horse battery staple",
        "qwerty123", "1987", "19870615", "john2024", "aaaaaaa", "abcdefg", "zxcvbnm", "trustno1", "p@ssw0rd",
        "the blue cabin by the lake", "xk7#qpl", "mississippi", "s3cr3t", "1234567890", "admin", "ilovemydog",
        "my first car was a red volvo", "ñandú", "日本語のこたえ", "z", "q1w2e3r4", "bob", "batman", "superman1",
        // l33t, keyboard walks, repeats, sequences, dates, years, capitals
        "p4$$w0rd", "l33tsp34k", "h3ll0 w0rld", "qazwsx", "1qaz2wsx", "asdfghjkl;", "!@#$%^&*", "QWERTY", "zaq12wsx",
        "147258369", "abcabcabc", "aabaabaab", "xyzxyzxyz", "hahahahaha", "12121212", "nbnbnbnbnb", "aaaaaaaaaaaaaaaaaaaa",
        "abcdef", "zyxwvu", "9753", "13579", "acegi", "2468", "11/11/1991", "1991-11-11", "11.11.91", "1191", "111504",
        "20151231", "31121999", "1/1/2001", "2050", "1999", "2019", "2020", "2024", "200", "2015_06_04",
        "Password", "PASSWORD", "passworD", "PassWord", "pAsSwOrD", "iLoveYou", "Ölberg", "straße", "ΣΊΣΥΦΟΣ",
        "constructor", "my dad was a constructor", "__proto__", "tostring", "valueof",
        "correcthorsebatterystaple", "trombone", "mypasswordismypassword", "monkey123monkey", "drowssap", "nimda",
        "four score and seven years ago our fathers", "😀😀😀", "a😀b😀", "tab\there", "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
        "9z8y7x", "the quick brown fox jumps over the lazy dog", "elizabeth", "smith", "jennifer2003", "1q2w3e4r5t",
    ];
    let mut child = node_cmd(&node, "js_zxcvbn.cjs")
        .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
    use std::io::Write;
    child.stdin.take().unwrap().write_all((samples.join("\n") + "\n").as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    let js: Vec<f64> = serde_json::from_slice::<Vec<Option<f64>>>(&out.stdout).unwrap().into_iter().map(|x| x.unwrap_or(f64::NAN)).collect();
    assert_eq!(js.len(), samples.len());
    let mut mismatches = Vec::new();
    for (s, j) in samples.iter().zip(&js) {
        let r = mvault_core::entropy::log2_guesses(s).unwrap_or(f64::NAN);
        let same = (r.is_nan() && j.is_nan()) || (r - j).abs() <= 1e-9 * j.abs().max(1.0);
        if !same {
            mismatches.push(format!("{s:?}: js {j:.6} rust {r:.6}"));
        }
    }
    assert!(mismatches.is_empty(), "zxcvbn differs:\n{}", mismatches.join("\n"));
}
