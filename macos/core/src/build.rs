//! Building a new vault from the answers page (build-vault.js): question list, layer-2 layout,
//! full vault and the "lite" companion for the paper backup.

use serde_json::{Value, json};

use crate::crypto;
use crate::jsstr::{js_trim, normalize_answer, utf16_at, utf16_len};
use crate::vault::{self, Fixed, Layer1, SealArgs, Sealed, Seed, Upload};
use crate::Result;

/// Hint options for one question (answer-questions.js `customHints[i]`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hint {
    pub letters: Vec<u32>,
    pub custom: String,
    pub show_length: Option<bool>,
    pub mcq_enabled: bool,
    pub mcq_options: Vec<String>,
}

/// What answer-questions.js stores for the builder (the `userData` record).
#[derive(Clone, Debug, Default)]
pub struct UserData {
    /// trimmed answers as typed (lowercase: the inputs force it)
    pub answers: Vec<String>,
    pub space_indexes: Vec<Vec<u32>>,
    pub hints: std::collections::BTreeMap<usize, Hint>,
    pub questions: Vec<String>,
    pub unlock_indexes: [usize; 2],
    pub final_message: String,
    /// -1 in JS when there is none
    pub trap_index: Option<usize>,
}

/// build-vault.js `buildQuestionList` (MCQ options get a CSPRNG shuffle per question).
pub fn build_question_list(u: &UserData) -> Vec<Value> {
    u.answers.iter().enumerate().map(|(i, ans)| {
        let trimmed = js_trim(ans).to_string();
        let space_indexes = u.space_indexes.get(i).cloned().unwrap_or_default();
        let hint = u.hints.get(&i).cloned().unwrap_or_default();
        let show_length = hint.show_length.unwrap_or(true);
        let mut hint_values: Vec<String> = hint.letters.iter()
            .map(|&pos| if pos >= 1 { utf16_at(&trimmed, pos as usize - 1).unwrap_or_else(|| "_".into()) } else { "_".into() })
            .collect();
        let mcq_enabled = hint.mcq_enabled;
        let mut mcq_options: Option<Vec<String>> = None;
        if mcq_enabled {
            let mut raw: Vec<String> = hint.mcq_options.iter().map(|s| js_trim(s).to_string()).filter(|s| !s.is_empty()).collect();
            if !raw.iter().any(|x| x.to_lowercase() == trimmed.to_lowercase()) {
                raw.insert(0, trimmed.clone());
            }
            let target = raw.len().clamp(6, 12);
            raw.truncate(target);
            mcq_options = Some(crypto::shuffle(&raw));
            hint_values.clear();
        }
        let question = u.questions.get(i).filter(|q| !q.is_empty()).cloned().unwrap_or_else(|| format!("Question {}", i + 1));
        json!({
            "question": question,
            "expectedLength": if mcq_enabled || !show_length { Value::Null } else { json!(utf16_len(&trimmed)) },
            "hintLetters": if mcq_enabled { vec![] } else { hint.letters.clone() },
            "hintValues": hint_values,
            "spaceIndexes": space_indexes,
            "custom": hint.custom,
            "showLength": show_length,
            "mcqEnabled": mcq_enabled,
            "mcqOptions": mcq_options,
        })
    }).collect()
}

/// Everything needed to seal the full vault and later the lite companion with the same answers.
pub struct Plan {
    pub unlock_concat: zeroize::Zeroizing<String>,
    pub layer1: Layer1,
    pub gate_answers: Vec<String>,
    pub remaining_answers: Vec<String>,
    pub selected_answers: Vec<String>,
    pub question_prompts: Vec<String>,
}

/// The layer-2 layout: a CSPRNG shuffle of the non-gate questions; the first six get the hidden
/// order, the red herring (an empty answer) stays out of them.
pub fn plan(u: &UserData) -> Plan {
    let unlock1 = normalize_answer(u.answers.get(u.unlock_indexes[0]).map(String::as_str).unwrap_or(""));
    let unlock2 = normalize_answer(u.answers.get(u.unlock_indexes[1]).map(String::as_str).unwrap_or(""));
    let normalized: Vec<String> = u.answers.iter().enumerate()
        .map(|(i, a)| if Some(i) == u.trap_index { String::new() } else { normalize_answer(a) })
        .collect();
    let n = normalized.len();
    let gates = vec![u.unlock_indexes[0], u.unlock_indexes[1]];
    let trap = u.trap_index;
    let pool: Vec<usize> = (0..n).filter(|i| !gates.contains(i) && Some(*i) != trap).collect();
    let layer2 = crypto::shuffle(&pool);
    let selected: Vec<usize> = layer2.iter().take(6).copied().collect();
    let mut rest: Vec<usize> = layer2.iter().skip(6).copied().collect();
    if let Some(t) = trap {
        if !gates.contains(&t) {
            rest.push(t);
        }
    }
    let remaining = crypto::shuffle(&rest);
    let at = |i: &usize| normalized[*i].clone();
    let mut unlock_concat = zeroize::Zeroizing::new(unlock1);
    unlock_concat.push_str(&unlock2);
    Plan {
        unlock_concat,
        gate_answers: gates.iter().map(at).collect(),
        remaining_answers: remaining.iter().map(at).collect(),
        selected_answers: selected.iter().map(at).collect(),
        question_prompts: gates.iter().map(|&i| u.questions.get(i).filter(|q| !q.is_empty()).cloned().unwrap_or_else(|| format!("Question {}", i + 1))).collect(),
        layer1: Layer1 { question_list: build_question_list(u), selected, remaining, gates },
    }
}

pub fn seal_full(p: &Plan, uploads: &[Upload], seeds: &[Seed], final_message: &str, on_progress: Option<&(dyn Fn(usize, usize, &str) + Sync)>) -> Result<Sealed> {
    vault::seal_vault(SealArgs {
        unlock_concat: &p.unlock_concat,
        layer1: &p.layer1,
        gate_answers: &p.gate_answers,
        remaining_answers: &p.remaining_answers,
        selected_answers: &p.selected_answers,
        question_prompts: p.question_prompts.clone(),
        uploads,
        seeds,
        final_message,
        created: None,
        meta_extras: None,
        fixed: None,
        on_progress,
    })
}

/// The lite companion: same answers, salts and hidden order as `full`, so the same answers open both.
pub fn seal_lite(p: &Plan, full: &Sealed, uploads: &[Upload], seeds: &[Seed], final_message: &str) -> Result<Sealed> {
    vault::seal_vault(SealArgs {
        unlock_concat: &p.unlock_concat,
        layer1: &p.layer1,
        gate_answers: &p.gate_answers,
        remaining_answers: &p.remaining_answers,
        selected_answers: &p.selected_answers,
        question_prompts: p.question_prompts.clone(),
        uploads,
        seeds,
        final_message,
        created: None,
        meta_extras: None,
        fixed: Some(Fixed { layer1_salt: full.layer1_salt.clone(), full_salt: full.full_salt.clone(), permutation: full.permutation.clone() }),
        on_progress: None,
    })
}
