//! Recovery (recover.js): find the hidden order of the six layer-2 answers, then decrypt the files.

use rayon::prelude::*;
use zeroize::Zeroizing;

use crate::vault::{self, FileMeta, Layer1, VaultJson, VaultMeta, VaultZip};
use crate::{Error, Result};

/// recover.js `generatePermutations`: position-major order (index 1 is the input order).
pub fn permutations<T: Clone>(items: &[T]) -> Vec<Vec<T>> {
    if items.len() <= 1 {
        return vec![items.to_vec()];
    }
    let mut out = Vec::new();
    for i in 0..items.len() {
        let mut rest = items.to_vec();
        let cur = rest.remove(i);
        for mut p in permutations(&rest) {
            p.insert(0, cur.clone());
            out.push(p);
        }
    }
    out
}

pub enum Progress<'a> {
    /// About to report attempt `n` of `total`
    Attempt { n: usize, total: usize },
    /// The vault opened on attempt `n`
    Unlocked { n: usize },
    FileOk(&'a str),
    FileSkipped(&'a str),
    FileFailed(&'a str, String),
}

pub struct Unlocked {
    pub meta: VaultMeta,
    pub permuted_concat: Zeroizing<String>,
    pub permutation: Vec<String>,
    pub attempt: usize,
}

/// Tries every order of the selected answers against vault.meta. Attempts run a batch at a time on
/// every core, but are reported and accepted strictly in order, so the result (and the attempt
/// number shown) is the same as recover.js trying them one by one.
pub fn find_order(
    vault_json: &VaultJson,
    meta_bytes: &[u8],
    l1: &Layer1,
    answer_at: &dyn Fn(usize) -> String,
    progress: &mut dyn FnMut(Progress<'_>),
    cancelled: &dyn Fn() -> bool,
) -> Result<Unlocked> {
    let full_salt = vault_json.full_salt()?.to_string();
    let argon = vault_json.argon()?;
    let gates: Vec<String> = l1.gates.iter().map(|&i| answer_at(i)).collect();
    let remaining: Vec<String> = l1.remaining.iter().map(|&i| answer_at(i)).collect();
    let brute: Vec<String> = l1.selected.iter().map(|&i| answer_at(i)).collect();
    let perms = permutations(&brute);
    let total = perms.len();
    let batch = rayon::current_num_threads().max(1);
    let mut start = 0;
    while start < total {
        if cancelled() {
            return Err(Error::Format("Cancelled.".into()));
        }
        let end = (start + batch).min(total);
        let results: Vec<Option<(VaultMeta, Zeroizing<String>)>> = perms[start..end].par_iter().map(|p| {
            let concat = vault::compose_concat(&gates, &remaining, p);
            let key = vault::derive_key(&concat, &full_salt, argon).ok()?;
            let meta = vault::decrypt_meta_with_key(&key, vault_json, meta_bytes).ok()?;
            Some((meta, concat))
        }).collect();
        for (k, r) in results.into_iter().enumerate() {
            let n = start + k + 1;
            progress(Progress::Attempt { n, total });
            if let Some((meta, concat)) = r {
                progress(Progress::Unlocked { n });
                return Ok(Unlocked { meta, permuted_concat: concat, permutation: perms[n - 1].clone(), attempt: n });
            }
        }
        start = end;
    }
    Err(Error::Format("All permutations exhausted without success. Please review your answers and try again.".into()))
}

/// Decrypts every .vaultdoc (each checked against its SHA-256); failures are reported and skipped.
pub fn decrypt_files(vz: &VaultZip, meta: &VaultMeta, permuted_concat: &str, progress: &mut dyn FnMut(Progress<'_>)) -> Vec<(FileMeta, Vec<u8>)> {
    let results: Vec<(usize, std::result::Result<Vec<u8>, Option<String>>)> = meta.files.par_iter().enumerate().map(|(i, m)| {
        match vz.get(&m.filename) {
            None => (i, Err(None)),
            Some(ct) => (i, vault::decrypt_file(permuted_concat, m, ct).map_err(|e| Some(e.to_string()))),
        }
    }).collect();
    let mut out = Vec::new();
    for (i, r) in results {
        let m = &meta.files[i];
        match r {
            Ok(bytes) => {
                progress(Progress::FileOk(&m.filename));
                out.push((m.clone(), bytes));
            }
            Err(None) => progress(Progress::FileSkipped(&m.filename)),
            Err(Some(e)) => progress(Progress::FileFailed(&m.filename, e)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn permutation_order_matches_recover_js() {
        let p = permutations(&[1, 2, 3]);
        assert_eq!(p, vec![vec![1, 2, 3], vec![1, 3, 2], vec![2, 1, 3], vec![2, 3, 1], vec![3, 1, 2], vec![3, 2, 1]]);
        assert_eq!(permutations(&[0; 6]).len(), 720);
    }
}
