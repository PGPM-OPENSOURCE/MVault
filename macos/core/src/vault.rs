//! The Memoro Vault format (vault-core.js): seal, package, read, verify, split and re-seal.

use std::collections::HashMap;
use std::io::{Cursor, Read, Write};

use rayon::prelude::*;
use serde_json::{Map, Value, json};
use zeroize::{Zeroize, Zeroizing};

use crate::crypto::{self, ARGON, ArgonSettings, FILE_ID_BYTES, GCM_TAG_BYTES, IV_BYTES, SALT_BYTES};
use crate::jsstr::{is_js_space, js_trim, utf16_slice};
use crate::{Error, Result};

pub const APP_VERSION: &str = "1.1.0";
pub const FINAL_MESSAGE_NAME: &str = "final-message.txt";
pub const LITE_CAP_BYTES: u64 = 153_600;
pub const MAX_FILE_BYTES: u64 = 1_610_612_736; // 1.5 GiB
/// `contentBytes > 1.9 * 1024 ** 3` in JS is the same test as `> 2_040_109_465` on integers.
pub const MAX_VAULT_BYTES: u64 = 2_040_109_465;
pub const INSTRUCTIONS: &str = "Use recover.html to unlock this archive using your answers.";

pub const README_TEXT: &str = "Memoro Vault
===========

What you have found is an encrypted archive created with Memoro Vault.
It was likely prepared by someone who wished their most important digital
information to be preserved securely and passed down.

Inside this archive are files and messages that can only be unlocked
with the correct answers to memory-based questions. Without those answers,
decryption is mathematically impossible. There are no backdoors.

To open this vault, you will need:
1. The Memoro Vault program (provided as a desktop installer in the same folder).
2. The exact answers to the questions chosen by the person who built it.
3. The original vault file itself (this ZIP).

If you are a family member who has recently discovered these files, this vault
was designed to protect sensitive material such as letters, records,
cryptocurrency keys, or final instructions. Guard it carefully.

Redundant storage is critical: make multiple copies to M-DISC, offline drives,
and printed QR backups if available. A single corrupt copy can render it
unrecoverable.

Libereco ne estas donaco. Ĝi estas devo.
— Kaŝmaristo Delvakto";

pub const BUILDER_TEXT: &str = "Vault Builder Instructions
=========================

This archive was created with Memoro Vault. If you are reading this, it
means the vault was intentionally packaged for long-term preservation.

To keep it safe:
1. Place this ZIP file together with the Memoro Vault installer
   (e.g. memoro-win.exe, memoro-linux.AppImage, or memoro-mac.dmg)
   inside a clearly labeled folder (for example, \"Family_Vault\").
2. Copy that folder to multiple storage media:
   • One archival M-DISC
   • One offline USB/SSD in a fireproof safe
   • One cloud service of your choice
3. If a paper QR backup PDF was produced, print and store it with equal care.
   Those QR tiles can fully reconstruct this vault if digital copies are lost.

Important: Do not rename, modify, or edit any files inside this archive.
Doing so will make the vault impossible to recover.

To unlock, run the Memoro Vault recovery program and follow the on-screen
instructions. You will be prompted to enter answers in the exact order
chosen by the original builder.

Libereco ne estas donaco. Ĝi estas devo.
— Kaŝmaristo Delvakto";

// ---------------------------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------------------------
#[derive(Clone)]
pub struct Upload {
    pub name: String,
    pub mime: String,
    pub bytes: std::sync::Arc<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seed {
    pub name: String,
    pub content: String,
}

/// The layer-1 plaintext. `question_list` stays raw JSON so a re-seal writes back exactly what the
/// vault held (older releases stored slightly different question objects).
#[derive(Clone, Debug)]
pub struct Layer1 {
    pub question_list: Vec<Value>,
    pub selected: Vec<usize>,
    pub remaining: Vec<usize>,
    pub gates: Vec<usize>,
}

impl Layer1 {
    pub fn to_json(&self) -> Value {
        json!({
            "questionList": self.question_list,
            "selectedL2Indices": self.selected,
            "remainingL2Indices": self.remaining,
            "gateIndices": self.gates,
        })
    }

    pub fn from_json(v: &Value) -> Result<Layer1> {
        let idx = |k: &str| -> Vec<usize> {
            v.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(as_index).collect()).unwrap_or_default()
        };
        let question_list = v.get("questionList").and_then(Value::as_array).cloned()
            .ok_or_else(|| Error::Format("Verification: question layer is malformed.".into()))?;
        Ok(Layer1 { question_list, selected: idx("selectedL2Indices"), remaining: idx("remainingL2Indices"), gates: idx("gateIndices") })
    }
}

fn as_index(v: &Value) -> Option<usize> {
    if let Some(u) = v.as_u64() {
        return Some(u as usize);
    }
    v.as_f64().filter(|f| f.fract() == 0.0 && *f >= 0.0).map(|f| f as usize)
}

/// One question as the recovery screen reads it (lenient: every field may be missing).
#[derive(Clone, Debug, Default)]
pub struct QuestionView {
    pub question: String,
    pub expected_length: Option<f64>,
    pub hint_letters: Vec<f64>,
    pub hint_values: Vec<Option<String>>,
    pub space_indexes: Vec<f64>,
    pub custom: String,
    pub mcq_enabled: bool,
    pub mcq_options: Option<Vec<String>>,
}

impl QuestionView {
    pub fn from_json(v: &Value) -> QuestionView {
        let nums = |k: &str| v.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
        QuestionView {
            question: js_string(v.get("question")),
            expected_length: v.get("expectedLength").and_then(|x| x.as_f64().or_else(|| x.as_str().and_then(|s| s.trim().parse().ok()))),
            hint_letters: nums("hintLetters"),
            hint_values: v.get("hintValues").and_then(Value::as_array)
                .map(|a| a.iter().map(|x| if x.is_null() { None } else { Some(js_string(Some(x))) }).collect()).unwrap_or_default(),
            space_indexes: nums("spaceIndexes"),
            custom: v.get("custom").map(|c| if c.is_null() { String::new() } else { js_string(Some(c)) }).unwrap_or_default(),
            mcq_enabled: v.get("mcqEnabled").map(truthy).unwrap_or(false),
            mcq_options: v.get("mcqOptions").and_then(Value::as_array).map(|a| a.iter().map(|x| js_string(Some(x))).collect()),
        }
    }
}

/// `String(x)` for the JSON values a vault may carry.
pub fn js_string(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Array(a)) => a.iter().map(|x| if x.is_null() { String::new() } else { js_string(Some(x)) }).collect::<Vec<_>>().join(","),
        Some(Value::Object(_)) => "[object Object]".into(),
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0 && !f.is_nan()).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// vault.meta `files[]` entry.
#[derive(Clone, Debug)]
pub struct FileMeta {
    pub raw: Value,
    pub filename: String,
    pub original_name: Option<String>,
    pub mime_type: Option<String>,
    pub iv: Vec<u8>,
    pub sha256: Option<String>,
}

impl FileMeta {
    pub fn from_json(v: &Value) -> FileMeta {
        let s = |k: &str| v.get(k).filter(|x| !x.is_null()).map(|x| js_string(Some(x)));
        FileMeta {
            raw: v.clone(),
            filename: js_string(v.get("filename")),
            original_name: s("originalName"),
            mime_type: s("mimeType"),
            iv: v.get("iv").and_then(Value::as_array).map(|a| a.iter().map(|x| x.as_u64().unwrap_or(0) as u8).collect()).unwrap_or_default(),
            sha256: s("sha256").filter(|h| !h.is_empty()),
        }
    }
    pub fn display_name(&self) -> &str {
        self.original_name.as_deref().filter(|n| !n.is_empty()).unwrap_or(&self.filename)
    }
}

#[derive(Clone, Debug)]
pub struct VaultMeta {
    pub raw: Value,
    pub files: Vec<FileMeta>,
    pub seed_files: Vec<(String, Option<String>)>,
    pub vault_hash: Option<String>,
}

impl VaultMeta {
    pub fn from_json(v: Value) -> Result<VaultMeta> {
        let files = v.get("files").and_then(Value::as_array)
            .ok_or_else(|| Error::Format("vault.meta has no file list.".into()))?
            .iter().map(FileMeta::from_json).collect();
        let seed_files = v.get("seedFiles").and_then(Value::as_array).map(|a| {
            a.iter().filter_map(|sf| {
                let name = sf.get("name")?.as_str()?.to_string();
                Some((name, sf.get("content").and_then(Value::as_str).map(String::from)))
            }).collect()
        }).unwrap_or_default();
        let vault_hash = v.get("vaultHash").and_then(Value::as_str).map(String::from);
        Ok(VaultMeta { raw: v, files, seed_files, vault_hash })
    }
    pub fn revision(&self) -> Option<i64> {
        self.raw.get("revision").and_then(Value::as_i64)
    }
    pub fn modified(&self) -> Option<String> {
        self.raw.get("modified").and_then(Value::as_str).map(String::from)
    }
}

/// The plaintext vault.json fields recovery needs, plus the raw object.
#[derive(Clone, Debug)]
pub struct VaultJson {
    pub raw: Value,
}

impl VaultJson {
    pub fn str(&self, k: &str) -> Option<&str> {
        self.raw.get(k).and_then(Value::as_str)
    }
    pub fn full_salt(&self) -> Result<&str> {
        self.str("fullSalt").ok_or_else(|| Error::Format("Vault metadata is incomplete: fullSalt not found.".into()))
    }
    pub fn layer1_salt(&self) -> Result<&str> {
        self.str("layer1Salt").ok_or_else(|| Error::Format("Vault metadata is incomplete: layer1Salt not found.".into()))
    }
    pub fn meta_iv(&self) -> Vec<u8> {
        byte_array(self.raw.get("vaultMetaIv"))
    }
    pub fn question_prompts(&self) -> Vec<String> {
        self.raw.get("questionPrompts").and_then(Value::as_array)
            .map(|a| a.iter().map(|x| js_string(Some(x))).collect()).unwrap_or_default()
    }
    pub fn created(&self) -> Option<String> {
        self.str("created").map(String::from)
    }
    pub fn argon(&self) -> Result<ArgonSettings> {
        safe_argon_settings(self.raw.get("argonSettings"))
    }
}

pub fn byte_array(v: Option<&Value>) -> Vec<u8> {
    v.and_then(Value::as_array).map(|a| a.iter().map(|x| x.as_u64().unwrap_or(0) as u8).collect()).unwrap_or_default()
}

/// Argon2 settings from an untrusted vault.json (vault-core.js `safeArgonSettings`).
pub fn safe_argon_settings(s: Option<&Value>) -> Result<ArgonSettings> {
    let Some(s) = s.filter(|s| !s.is_null()) else { return Ok(ARGON) };
    let int = |k: &str| -> Option<f64> { s.get(k).and_then(Value::as_f64).filter(|f| f.fract() == 0.0) };
    let bad = || Error::Format("This vault asks for unsupported Argon2 settings.".into());
    let (t, m, p, len) = (int("time").ok_or_else(bad)?, int("mem").ok_or_else(bad)?, int("parallelism").ok_or_else(bad)?, int("hashLen").ok_or_else(bad)?);
    if !(1.0..=10.0).contains(&t) || !(8192.0..=262144.0).contains(&m) || !(1.0..=4.0).contains(&p) || len != 32.0 {
        return Err(bad());
    }
    Ok(ArgonSettings { time: t as u32, mem: m as u32, parallelism: p as u32, hash_len: 32 })
}

/// Untrusted file names (vault-core.js `cleanFileName`).
pub fn clean_file_name(input: &str, fallback: &str) -> String {
    let mut n = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '\\' | '/' | '\u{0000}'..='\u{001f}' | '\u{007f}' => n.push('_'),
            '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => {}
            _ => n.push(c),
        }
    }
    let n = utf16_slice(js_trim(&n), 200);
    if n.is_empty() || n == "." || n == ".." { fallback.to_string() } else { n }
}

// ---------------------------------------------------------------------------------------------
// Key derivation + per-file encryption (format identical to v1.0.8)
// ---------------------------------------------------------------------------------------------
pub fn derive_key(password: &str, salt_hex: &str, s: ArgonSettings) -> Result<crypto::Key32> {
    let salt = crypto::unhex(salt_hex)?;
    crypto::argon2_derive(password.as_bytes(), &salt, s)
}

/// Argon2id(permutedConcat + fileId, salt = SHA-256(permutedConcat + fileId)).
pub fn derive_file_key(permuted_concat: &str, file_id: &str) -> Result<crypto::Key32> {
    let mut pw = Zeroizing::new(String::with_capacity(permuted_concat.len() + file_id.len()));
    pw.push_str(permuted_concat);
    pw.push_str(file_id);
    let salt = crypto::sha256(pw.as_bytes());
    crypto::argon2_derive(pw.as_bytes(), &salt, ARGON)
}

pub struct EncryptedFile {
    pub meta: Value,
    pub filename: String,
    pub ciphertext: Vec<u8>,
}

pub fn encrypt_file(permuted_concat: &str, bytes: &[u8], name: &str, mime: &str) -> Result<EncryptedFile> {
    let file_id = crypto::random_hex(FILE_ID_BYTES);
    let key = derive_file_key(permuted_concat, &file_id)?;
    let iv = crypto::random_bytes(IV_BYTES);
    let ct = crypto::aes_gcm_encrypt(&key, &iv, bytes)?;
    let filename = format!("{file_id}.vaultdoc");
    let meta = json!({
        "id": file_id,
        "filename": filename,
        "originalName": name,
        "mimeType": mime,
        "iv": iv,
        "sha256": crypto::sha256_hex(bytes),
    });
    Ok(EncryptedFile { meta, filename, ciphertext: ct })
}

/// Decrypts one .vaultdoc and checks the SHA-256 recorded in vault.meta (when present).
pub fn decrypt_file(permuted_concat: &str, meta: &FileMeta, ciphertext: &[u8]) -> Result<Vec<u8>> {
    if ciphertext.len() < GCM_TAG_BYTES {
        return Err(Error::Format(format!("{} is too short to be valid.", meta.filename)));
    }
    let file_id = strip_suffix_ci(&meta.filename, ".vaultdoc");
    let key = derive_file_key(permuted_concat, file_id)?;
    let plain = crypto::aes_gcm_decrypt(&key, &meta.iv, ciphertext)?;
    if let Some(want) = &meta.sha256 {
        if crypto::sha256_hex(&plain) != want.to_lowercase() {
            return Err(Error::Format(format!("Integrity check failed for {} (SHA-256 mismatch).", meta.display_name())));
        }
    }
    Ok(plain)
}

fn strip_suffix_ci<'a>(s: &'a str, suffix: &str) -> &'a str {
    if s.len() >= suffix.len() && s.is_char_boundary(s.len() - suffix.len()) && s[s.len() - suffix.len()..].eq_ignore_ascii_case(suffix) {
        &s[..s.len() - suffix.len()]
    } else {
        s
    }
}

pub fn compose_concat(gates: &[String], remaining: &[String], permutation: &[String]) -> Zeroizing<String> {
    let mut s = Zeroizing::new(String::new());
    for a in gates.iter().chain(remaining).chain(permutation) {
        s.push_str(a);
    }
    s
}

fn same_multiset(a: &[String], b: &[String]) -> bool {
    let mut x = a.to_vec();
    let mut y = b.to_vec();
    x.sort();
    y.sort();
    x == y
}

// ---------------------------------------------------------------------------------------------
// Seal
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Debug)]
pub struct Fixed {
    pub layer1_salt: String,
    pub full_salt: String,
    pub permutation: Vec<String>,
}

pub struct SealArgs<'a> {
    pub unlock_concat: &'a str,
    pub layer1: &'a Layer1,
    pub gate_answers: &'a [String],
    pub remaining_answers: &'a [String],
    pub selected_answers: &'a [String],
    pub question_prompts: Vec<String>,
    pub uploads: &'a [Upload],
    pub seeds: &'a [Seed],
    pub final_message: &'a str,
    pub created: Option<String>,
    pub meta_extras: Option<Map<String, Value>>,
    pub fixed: Option<Fixed>,
    pub on_progress: Option<&'a (dyn Fn(usize, usize, &str) + Sync)>,
}

pub struct Sealed {
    pub vault_json: Value,
    pub vault_enc: Value,
    pub meta_bytes: Vec<u8>,
    pub files: Vec<(String, Vec<u8>)>,
    pub file_meta: Vec<Value>,
    pub layer1_salt: String,
    pub full_salt: String,
    pub permutation: Vec<String>,
    pub permuted_concat: Zeroizing<String>,
    pub vault_hash: String,
}

pub fn iso_now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

pub fn seal_vault(o: SealArgs<'_>) -> Result<Sealed> {
    let final_message = js_trim(o.final_message).to_string();
    let l = o.layer1;
    if o.unlock_concat.is_empty() {
        return Err(Error::Format("Unlock answers are missing.".into()));
    }
    if o.gate_answers.len() != l.gates.len() || o.remaining_answers.len() != l.remaining.len() || o.selected_answers.len() != l.selected.len() {
        return Err(Error::Format("Answer sets do not match the vault question layout.".into()));
    }
    if o.question_prompts.len() != 2 {
        return Err(Error::Format("Two unlock question prompts are required.".into()));
    }

    let mut content_bytes = final_message.len() as u64;
    for u in o.uploads {
        let n = u.bytes.len() as u64;
        if n > MAX_FILE_BYTES {
            return Err(Error::Format(format!("{} is larger than the 1.5 GB a single file in a vault can be.", u.name)));
        }
        content_bytes += n;
    }
    for s in o.seeds {
        content_bytes += s.content.len() as u64;
    }
    if content_bytes > MAX_VAULT_BYTES {
        return Err(Error::Format("The vault contents add up to more than 1.9 GB, which is more than Memoro Vault can seal and open.".into()));
    }

    let total = o.uploads.len() + o.seeds.len() + usize::from(!final_message.is_empty()) + 3;
    let done = std::sync::atomic::AtomicUsize::new(0);
    let tick = |label: &str| {
        let d = done.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        if let Some(cb) = o.on_progress {
            cb(d, total, label);
        }
    };

    // Layer 1: question list + index layout, keyed by the two gate answers
    let fixed = o.fixed.clone();
    let layer1_salt = fixed.as_ref().map(|f| f.layer1_salt.clone()).unwrap_or_else(|| crypto::random_hex(SALT_BYTES));
    let l1_key = derive_key(o.unlock_concat, &layer1_salt, ARGON)?;
    let iv1 = crypto::random_bytes(IV_BYTES);
    let l1_plain = Zeroizing::new(serde_json::to_vec(&l.to_json())?);
    let l1_cipher = crypto::aes_gcm_encrypt(&l1_key, &iv1, &l1_plain)?;
    drop(l1_key);
    tick("Encrypted question layer");

    // Full key: gates -> remaining L2 -> hidden order of the selected L2 answers
    let permutation = match &fixed {
        Some(f) => {
            if !same_multiset(&f.permutation, o.selected_answers) {
                return Err(Error::Format("Fixed permutation does not match the selected answers.".into()));
            }
            f.permutation.clone()
        }
        None => crypto::shuffle(o.selected_answers),
    };
    let permuted_concat = compose_concat(o.gate_answers, o.remaining_answers, &permutation);
    let full_salt = fixed.as_ref().map(|f| f.full_salt.clone()).unwrap_or_else(|| crypto::random_hex(SALT_BYTES));
    let full_key = derive_key(&permuted_concat, &full_salt, ARGON)?;
    let vault_hash = crypto::sha256_hex(permuted_concat.as_bytes());
    tick("Derived vault key");

    // Contents: uploads, then seed files, then the final message (v1.0.8 order)
    let mut items: Vec<(&[u8], &str, &str)> = Vec::new();
    for u in o.uploads {
        items.push((u.bytes.as_slice(), &u.name, &u.mime));
    }
    for s in o.seeds {
        items.push((s.content.as_bytes(), &s.name, "text/plain"));
    }
    if !final_message.is_empty() {
        items.push((final_message.as_bytes(), FINAL_MESSAGE_NAME, "text/plain"));
    }
    // Every file has its own Argon2id key: derive them on all cores (results keep vault order)
    let encrypted: Vec<EncryptedFile> = items
        .par_iter()
        .map(|(bytes, name, mime)| {
            let r = encrypt_file(&permuted_concat, bytes, name, mime);
            tick(&format!("Encrypted {name}"));
            r
        })
        .collect::<Result<Vec<_>>>()?;

    let mut meta_obj = Map::new();
    let file_meta: Vec<Value> = encrypted.iter().map(|e| e.meta.clone()).collect();
    meta_obj.insert("files".into(), Value::Array(file_meta.clone()));
    meta_obj.insert("seedFiles".into(), Value::Array(o.seeds.iter().map(|s| json!({"name": s.name, "content": s.content})).collect()));
    meta_obj.insert("vaultHash".into(), Value::String(vault_hash.clone()));
    if let Some(extras) = &o.meta_extras {
        for (k, v) in extras {
            meta_obj.insert(k.clone(), v.clone());
        }
    }
    let meta_iv = crypto::random_bytes(IV_BYTES);
    let meta_plain = Zeroizing::new(serde_json::to_vec(&Value::Object(meta_obj))?);
    let meta_bytes = crypto::aes_gcm_encrypt(&full_key, &meta_iv, &meta_plain)?;
    drop(full_key);
    tick("Encrypted vault index");

    let vault_json = json!({
        "version": APP_VERSION,
        "created": o.created.unwrap_or_else(iso_now),
        "instructions": INSTRUCTIONS,
        "fullSalt": full_salt,
        "layer1Salt": layer1_salt,
        "questionPrompts": o.question_prompts.iter().take(2).collect::<Vec<_>>(),
        "vaultMetaIv": meta_iv,
        "argonSettings": { "time": ARGON.time, "mem": ARGON.mem, "parallelism": ARGON.parallelism, "type": "Argon2id", "hashLen": ARGON.hash_len },
    });
    let vault_enc = json!({ "ciphertext": l1_cipher, "iv": iv1 });

    Ok(Sealed {
        vault_json,
        vault_enc,
        meta_bytes,
        files: encrypted.into_iter().map(|e| (e.filename, e.ciphertext)).collect(),
        file_meta,
        layer1_salt,
        full_salt,
        permutation,
        permuted_concat,
        vault_hash,
    })
}

// ---------------------------------------------------------------------------------------------
// ZIP
// ---------------------------------------------------------------------------------------------
fn pretty(v: &Value) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec_pretty(v)?)
}

/// Packs a sealed vault (full vaults carry the two helper text files). Encrypted entries are stored.
pub fn package_zip(sealed: &Sealed, include_docs: bool) -> Result<Vec<u8>> {
    use zip::write::SimpleFileOptions;
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let deflate = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored).large_file(false);
    w.start_file("vault.json", deflate)?;
    w.write_all(&pretty(&sealed.vault_json)?)?;
    w.start_file("vault.enc", deflate)?;
    w.write_all(&pretty(&sealed.vault_enc)?)?;
    w.start_file("vault.meta", stored)?;
    w.write_all(&sealed.meta_bytes)?;
    for (name, bytes) in &sealed.files {
        let opts = if bytes.len() as u64 >= u32::MAX as u64 { stored.large_file(true) } else { stored };
        w.start_file(name.as_str(), opts)?;
        w.write_all(bytes)?;
    }
    if include_docs {
        w.start_file("README.txt", deflate)?;
        w.write_all(README_TEXT.as_bytes())?;
        w.start_file("Vault_Builder_Instructions.txt", deflate)?;
        w.write_all(BUILDER_TEXT.as_bytes())?;
    }
    Ok(w.finish()?.into_inner())
}

/// A vault ZIP read into memory: entries by lowercase name (README files skipped).
pub struct VaultZip {
    pub entries: HashMap<String, Vec<u8>>,
    pub vault_json: VaultJson,
    pub vault_enc: Value,
}

impl VaultZip {
    pub fn get(&self, name: &str) -> Option<&Vec<u8>> {
        self.entries.get(&name.to_lowercase())
    }
    pub fn meta_bytes(&self) -> &[u8] {
        self.entries.get("vault.meta").map(Vec::as_slice).unwrap_or(&[])
    }
}

pub fn read_vault_zip(bytes: &[u8]) -> Result<VaultZip> {
    let mut ar = zip::ZipArchive::new(Cursor::new(bytes))?;
    const MAX_ENTRY: u64 = 2 * 1024 * 1024 * 1024;
    const MAX_TOTAL: u64 = 4 * 1024 * 1024 * 1024;
    let mut total = 0u64;
    let mut entries = HashMap::new();
    for i in 0..ar.len() {
        let mut f = ar.by_index(i)?;
        if f.is_dir() {
            continue;
        }
        let name = f.name()?.to_string();
        let lower = name.to_lowercase();
        if lower == "readme.txt" || lower == "vault_builder_instructions.txt" {
            continue;
        }
        let size = f.size();
        total += size;
        if size > MAX_ENTRY || total > MAX_TOTAL {
            return Err(Error::Format("This vault is larger than Memoro Vault can open safely.".into()));
        }
        let mut data = Vec::with_capacity(size as usize);
        (&mut f).take(size + 1).read_to_end(&mut data)?;
        if data.len() as u64 != size {
            return Err(Error::Format(format!("{name} in this vault is damaged.")));
        }
        entries.insert(lower, data);
    }
    let need = |n: &str| -> Result<&Vec<u8>> { entries.get(n).ok_or_else(|| Error::Format(format!("Vault ZIP is missing {n}."))) };
    let vault_json = VaultJson { raw: serde_json::from_slice(need("vault.json")?)? };
    let vault_enc: Value = serde_json::from_slice(need("vault.enc")?)?;
    need("vault.meta")?;
    Ok(VaultZip { entries, vault_json, vault_enc })
}

/// Just vault.json / vault.enc / vault.meta (the dashboard's import check), without unpacking files.
pub fn peek_vault_zip(bytes: &[u8]) -> Result<(VaultJson, Value, Vec<u8>)> {
    let mut ar = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut get = |n: &str| -> Result<Option<Vec<u8>>> {
        for i in 0..ar.len() {
            let mut f = ar.by_index(i)?;
            if f.name()?.to_lowercase() == n {
                let size = f.size();
                if size > 64 * 1024 * 1024 {
                    return Err(Error::Format(format!("{n} is too large.")));
                }
                let mut d = Vec::new();
                (&mut f).take(size + 1).read_to_end(&mut d)?;
                return Ok(Some(d));
            }
        }
        Ok(None)
    };
    let (j, e, m) = (get("vault.json")?, get("vault.enc")?, get("vault.meta")?);
    let (Some(j), Some(e), Some(m)) = (j, e, m) else {
        return Err(Error::Format("ZIP is missing one or more required files: vault.json, vault.enc, vault.meta".into()));
    };
    let vj = VaultJson { raw: serde_json::from_slice(&j)? };
    vj.argon()?;
    Ok((vj, serde_json::from_slice(&e)?, m))
}

// ---------------------------------------------------------------------------------------------
// Decrypt helpers
// ---------------------------------------------------------------------------------------------
pub fn decrypt_layer1(unlock_concat: &str, vault_json: &VaultJson, vault_enc: &Value) -> Result<Layer1> {
    let key = derive_key(unlock_concat, vault_json.layer1_salt()?, vault_json.argon()?)?;
    let iv = byte_array(vault_enc.get("iv"));
    let ct = byte_array(vault_enc.get("ciphertext"));
    let plain = Zeroizing::new(crypto::aes_gcm_decrypt(&key, &iv, &ct)?);
    let v: Value = serde_json::from_slice(&plain)?;
    Layer1::from_json(&v)
}

pub fn decrypt_meta_with_key(key: &[u8], vault_json: &VaultJson, meta_bytes: &[u8]) -> Result<VaultMeta> {
    let plain = Zeroizing::new(crypto::aes_gcm_decrypt(key, &vault_json.meta_iv(), meta_bytes)?);
    VaultMeta::from_json(serde_json::from_slice(&plain)?)
}

pub fn decrypt_meta(permuted_concat: &str, vault_json: &VaultJson, meta_bytes: &[u8]) -> Result<VaultMeta> {
    let key = derive_key(permuted_concat, vault_json.full_salt()?, vault_json.argon()?)?;
    decrypt_meta_with_key(&key, vault_json, meta_bytes)
}

pub struct Expected<'a> {
    pub name: &'a str,
    pub bytes: &'a [u8],
}

/// Re-opens a sealed ZIP from scratch and proves it is recoverable (vault-core.js `verifyVaultZip`).
pub fn verify_vault_zip(zip_bytes: &[u8], unlock_concat: &str, permuted_concat: &str, expected: Option<&[Expected<'_>]>) -> Result<(Layer1, VaultMeta)> {
    let vz = read_vault_zip(zip_bytes)?;
    let l1 = decrypt_layer1(unlock_concat, &vz.vault_json, &vz.vault_enc)?;
    let meta = decrypt_meta(permuted_concat, &vz.vault_json, vz.meta_bytes())?;
    if meta.vault_hash.as_deref() != Some(crypto::sha256_hex(permuted_concat.as_bytes()).as_str()) {
        return Err(Error::Format("Verification: vault hash mismatch.".into()));
    }
    if let Some(exp) = expected {
        if exp.len() != meta.files.len() {
            return Err(Error::Format("Verification: file count mismatch.".into()));
        }
    }
    let results: Vec<Result<()>> = meta.files.par_iter().enumerate().map(|(i, m)| {
        let ct = vz.get(&m.filename).ok_or_else(|| Error::Format(format!("Verification: {} is missing from the ZIP.", m.filename)))?;
        let mut plain = decrypt_file(permuted_concat, m, ct)?;
        if let Some(exp) = expected {
            let e = &exp[i];
            if m.original_name.as_deref() != Some(e.name) {
                return Err(Error::Format(format!("Verification: name mismatch for {}.", e.name)));
            }
            if plain != e.bytes {
                return Err(Error::Format(format!("Verification: contents differ for {}.", e.name)));
            }
        }
        plain.zeroize();
        Ok(())
    }).collect();
    for r in results {
        r?;
    }
    Ok((l1, meta))
}

// ---------------------------------------------------------------------------------------------
// Editing an unlocked vault
// ---------------------------------------------------------------------------------------------
/// UTF-8 text when the bytes are exactly valid UTF-8 (a BOM is kept as U+FEFF), otherwise None.
pub fn strict_utf8(bytes: &[u8]) -> Option<String> {
    std::str::from_utf8(bytes).ok().map(String::from)
}

#[derive(Clone)]
pub struct Contents {
    pub uploads: Vec<Upload>,
    pub seeds: Vec<Seed>,
    pub final_message: String,
}

/// vault-core.js `splitContents`: uploads, named seed files, final message (in vault order).
pub fn split_contents(meta: &VaultMeta, decrypted: Vec<(FileMeta, Vec<u8>)>) -> Contents {
    let mut claimed = std::collections::HashSet::new();
    let mut final_message = String::new();
    if let Some((m, bytes)) = decrypted.last() {
        if let Some(text) = strict_utf8(bytes) {
            let mime = m.mime_type.clone().filter(|x| !x.is_empty()).unwrap_or_else(|| "text/plain".into());
            if m.original_name.as_deref() == Some(FINAL_MESSAGE_NAME) && mime == "text/plain" && !text.is_empty() && text == js_trim(&text) {
                final_message = text;
                claimed.insert(decrypted.len() - 1);
            }
        }
    }
    let mut seeds: Vec<Seed> = Vec::new();
    for (name, content) in meta.seed_files.iter().rev() {
        let mut found = None;
        for i in (0..decrypted.len()).rev() {
            if claimed.contains(&i) || decrypted[i].0.original_name.as_deref() != Some(name.as_str()) {
                continue;
            }
            if let Some(text) = strict_utf8(&decrypted[i].1) {
                if content.as_ref().is_none_or(|c| *c == text) {
                    found = Some((i, text));
                    break;
                }
            }
        }
        if let Some((i, text)) = found {
            claimed.insert(i);
            seeds.insert(0, Seed { name: clean_file_name(name, "seed.txt"), content: text });
        } else if let Some(c) = content {
            seeds.insert(0, Seed { name: clean_file_name(name, "seed.txt"), content: c.clone() });
        }
    }
    let mut uploads = Vec::new();
    for (i, (m, bytes)) in decrypted.into_iter().enumerate() {
        if claimed.contains(&i) {
            continue;
        }
        let name = m.original_name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| m.filename.clone());
        uploads.push(Upload { name: clean_file_name(&name, "file.bin"), mime: m.mime_type.clone().unwrap_or_default(), bytes: std::sync::Arc::new(bytes) });
    }
    Contents { uploads, seeds, final_message }
}

/// Makes names unique and clear of the reserved ones: "name (1).ext" (vault-core.js `dedupeNames`).
pub fn dedupe_names(names: &[String], reserved: &[String]) -> Vec<String> {
    let mut seen: std::collections::HashSet<String> = reserved.iter().map(|n| n.to_lowercase()).collect();
    names.iter().map(|name| {
        let name = if name.is_empty() { "file.bin".to_string() } else { name.clone() };
        let dot = name.rfind('.').filter(|&d| d > 0);
        let (base, ext) = match dot { Some(d) => (&name[..d], &name[d..]), None => (name.as_str(), "") };
        let mut candidate = name.clone();
        let mut n = 1;
        while seen.contains(&candidate.to_lowercase()) {
            candidate = format!("{base} ({n}){ext}");
            n += 1;
        }
        seen.insert(candidate.to_lowercase());
        candidate
    }).collect()
}

pub struct Session {
    pub vault_json: VaultJson,
    pub vault_enc: Value,
    pub unlock_concat: Zeroizing<String>,
    pub gate_answers: Vec<String>,
    pub remaining_answers: Vec<String>,
    pub selected_answers: Vec<String>,
}

impl Drop for Session {
    fn drop(&mut self) {
        for a in self.gate_answers.iter_mut().chain(&mut self.remaining_answers).chain(&mut self.selected_answers) {
            a.zeroize();
        }
    }
}

pub struct Resealed {
    pub zip_bytes: Vec<u8>,
    pub sealed: Sealed,
    pub upload_names: Vec<String>,
    pub final_message: String,
}

/// Re-seals an unlocked vault with edited contents and verifies the result (vault-core.js `resealVault`).
pub fn reseal_vault(
    s: &Session,
    uploads: &[Upload],
    seeds: &[Seed],
    final_message: &str,
    meta_extras: Option<Map<String, Value>>,
    on_progress: Option<&(dyn Fn(usize, usize, &str) + Sync)>,
) -> Result<Resealed> {
    let l1 = decrypt_layer1(&s.unlock_concat, &s.vault_json, &s.vault_enc)?;
    let final_message = js_trim(final_message).to_string();
    let mut reserved = vec![FINAL_MESSAGE_NAME.to_string()];
    reserved.extend(seeds.iter().map(|x| x.name.clone()));
    let names = dedupe_names(&uploads.iter().map(|u| u.name.clone()).collect::<Vec<_>>(), &reserved);
    let uploads: Vec<Upload> = uploads.iter().zip(&names).map(|(u, n)| Upload { name: n.clone(), mime: u.mime.clone(), bytes: u.bytes.clone() }).collect();
    let prompts_json = s.vault_json.question_prompts();
    let prompts: Vec<String> = l1.gates.iter().enumerate().map(|(i, &g)| {
        l1.question_list.get(g).and_then(|q| q.get("question")).and_then(Value::as_str).filter(|q| !q.is_empty()).map(String::from)
            .or_else(|| prompts_json.get(i).cloned().filter(|p| !p.is_empty()))
            .unwrap_or_else(|| format!("Question {}", g + 1))
    }).collect();
    let sealed = seal_vault(SealArgs {
        unlock_concat: &s.unlock_concat,
        layer1: &l1,
        gate_answers: &s.gate_answers,
        remaining_answers: &s.remaining_answers,
        selected_answers: &s.selected_answers,
        question_prompts: prompts,
        uploads: &uploads,
        seeds,
        final_message: &final_message,
        created: s.vault_json.created(),
        meta_extras,
        fixed: None,
        on_progress,
    })?;
    let zip_bytes = package_zip(&sealed, true)?;
    let fm_bytes = final_message.as_bytes();
    let mut expected: Vec<Expected<'_>> = uploads.iter().map(|u| Expected { name: &u.name, bytes: &u.bytes }).collect();
    expected.extend(seeds.iter().map(|x| Expected { name: &x.name, bytes: x.content.as_bytes() }));
    if !final_message.is_empty() {
        expected.push(Expected { name: FINAL_MESSAGE_NAME, bytes: fm_bytes });
    }
    verify_vault_zip(&zip_bytes, &s.unlock_concat, &sealed.permuted_concat, Some(&expected))?;
    let mut sealed = sealed;
    sealed.files.clear(); // the ZIP holds them now
    Ok(Resealed { zip_bytes, sealed, upload_names: names, final_message })
}

/// `fileName.replace(/\.zip$/i, '')`
pub fn strip_zip_ext(name: &str) -> String {
    strip_suffix_ci(name, ".zip").to_string()
}

/// Seed file names: lowercase letters, digits and _ plus ".txt" (vault-editor.js `seedFileName`).
pub fn seed_file_name(input: &str) -> String {
    let t = js_trim(input);
    let t = strip_suffix_ci(t, ".txt");
    let base: String = t.to_lowercase().chars().map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() { c } else { '_' }).collect();
    if base.is_empty() { String::new() } else { format!("{base}.txt") }
}

/// answer-questions.js `saveNamedSeed`: `name.toLowerCase().replace(/[^a-z0-9]/gi, "_")`
pub fn named_seed_base(name: &str) -> String {
    name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

pub fn word_count(s: &str) -> usize {
    js_trim(s).split(is_js_space).filter(|w| !w.is_empty()).count()
}
