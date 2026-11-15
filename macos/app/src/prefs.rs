//! The app's own folder (~/Library/Application Support/Memoro Vault, the Electron app's userData,
//! so a licence accepted there counts here too) and the first-run licence record kept in it.

use std::path::PathBuf;

const EULA_FILE: &str = "eula_accepted_v1.json";

pub fn user_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join("Library").join("Application Support").join("Memoro Vault")
}

/// main.js `isLicenseAccepted`
pub fn license_accepted() -> bool {
    std::fs::read(user_dir().join(EULA_FILE)).ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .is_some_and(|v| v.get("accepted") == Some(&serde_json::Value::Bool(true)))
}

/// main.js `markLicenseAccepted`
pub fn mark_license_accepted() {
    let dir = user_dir();
    let _ = std::fs::create_dir_all(&dir);
    let body = serde_json::json!({ "accepted": true, "ts": chrono::Utc::now().timestamp_millis() }).to_string();
    if let Err(e) = std::fs::write(dir.join(EULA_FILE), body) {
        eprintln!("Could not persist EULA acceptance: {e}");
    }
}

/// Releases up to 1.0.8 kept answers, seed phrases and uploaded files in the Electron profile's
/// web storage, next to these files. Overwrite (best effort) and delete whatever is left there,
/// as main.js `scrubLegacyWebStorage` does on every start.
const LEGACY_WEB_STORAGE: [&str; 8] = ["IndexedDB", "Local Storage", "Session Storage", "blob_storage", "WebStorage", "databases", "File System", "Service Worker"];

fn scrub_file(path: &std::path::Path) {
    use std::io::{Seek, SeekFrom, Write};
    use std::os::unix::fs::OpenOptionsExt;
    // O_NOFOLLOW: a file swapped for a symlink is refused instead of overwriting its target
    let Ok(mut f) = std::fs::OpenOptions::new().write(true).custom_flags(libc_o_nofollow()).open(path) else { return };
    let Ok(size) = f.metadata().map(|m| m.len()) else { return };
    let zeros = vec![0u8; size.min(1 << 20) as usize];
    let mut off = 0u64;
    while off < size {
        let n = (size - off).min(zeros.len() as u64) as usize;
        if f.seek(SeekFrom::Start(off)).is_err() || f.write_all(&zeros[..n]).is_err() {
            return;
        }
        off += n as u64;
    }
    let _ = f.sync_all();
}

const fn libc_o_nofollow() -> i32 {
    0x0100 // O_NOFOLLOW on macOS
}

fn scrub_dir(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let Ok(t) = e.file_type() else { continue };
        if t.is_symlink() {
            continue; // removed with the folder, its target left alone
        } else if t.is_dir() {
            scrub_dir(&e.path());
        } else if t.is_file() {
            scrub_file(&e.path());
        }
    }
}

pub fn scrub_legacy_web_storage() {
    let base = user_dir();
    for name in LEGACY_WEB_STORAGE {
        let p = base.join(name);
        let Ok(meta) = std::fs::symlink_metadata(&p) else { continue };
        if meta.file_type().is_symlink() {
            let _ = std::fs::remove_file(&p);
            continue;
        }
        if meta.is_dir() {
            scrub_dir(&p);
            let _ = std::fs::remove_dir_all(&p);
        } else if meta.is_file() {
            scrub_file(&p);
            let _ = std::fs::remove_file(&p);
        }
    }
}
