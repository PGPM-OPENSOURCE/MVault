//! zxcvbn 4.4.2 — the copy of the JavaScript library MVault ships — ported line by line so the
//! strength numbers on screen are the same: f64 arithmetic, UTF-16 code units, JavaScript's
//! Math.max/min (NaN wins), stable match ordering, and the plain-object dictionary quirk
//! ("constructor" and "__proto__" are "found" in every dictionary, with a rank that is NaN).
//! Dictionaries and keyboard graphs come from that same file (data/zxcvbn-data.json).

use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

type S16 = Vec<u16>;

struct Graph {
    name: String,
    adj: HashMap<u16, Vec<Option<S16>>>,
}

struct Data {
    dicts: Vec<HashMap<S16, f64>>, // the six frequency lists, then user_inputs (empty)
    graphs: Vec<Graph>,
    keyboard_avg_degree: f64,
    keypad_avg_degree: f64,
    keyboard_starting: f64,
    keypad_starting: f64,
}

fn data() -> &'static Data {
    static DATA: OnceLock<Data> = OnceLock::new();
    DATA.get_or_init(|| {
        let raw: serde_json::Value = serde_json::from_str(include_str!("../data/zxcvbn-data.json")).expect("zxcvbn data");
        let mut dicts = Vec::new();
        for pair in raw["frequency_lists"].as_array().unwrap() {
            let mut d = HashMap::new();
            for (i, w) in pair[1].as_array().unwrap().iter().enumerate() {
                let w = w.as_str().unwrap();
                if w == "__proto__" {
                    continue; // assigning it on a plain object sets the prototype, not a key
                }
                d.insert(w.encode_utf16().collect::<S16>(), (i + 1) as f64);
            }
            dicts.push(d);
        }
        dicts.push(HashMap::new()); // user_inputs
        let mut graphs = Vec::new();
        for pair in raw["adjacency_graphs"].as_array().unwrap() {
            let name = pair[0].as_str().unwrap().to_string();
            let mut adj = HashMap::new();
            for kv in pair[1].as_array().unwrap() {
                let key: S16 = kv[0].as_str().unwrap().encode_utf16().collect();
                let list = kv[1].as_array().unwrap().iter().map(|a| a.as_str().map(|s| s.encode_utf16().collect())).collect();
                adj.insert(key[0], list);
            }
            graphs.push(Graph { name, adj });
        }
        let avg = |g: &Graph| {
            let mut a = 0.0;
            for n in g.adj.values() {
                a += n.iter().filter(|x| x.is_some()).count() as f64;
            }
            a / g.adj.len() as f64
        };
        let qwerty = graphs.iter().find(|g| g.name == "qwerty").unwrap();
        let keypad = graphs.iter().find(|g| g.name == "keypad").unwrap();
        Data {
            keyboard_avg_degree: avg(qwerty),
            keypad_avg_degree: avg(keypad),
            keyboard_starting: qwerty.adj.len() as f64,
            keypad_starting: keypad.adj.len() as f64,
            dicts,
            graphs,
        }
    })
}

/// Loads the dictionaries ahead of time (the first estimate would otherwise pay for it).
pub fn warm_up() {
    data();
}

fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else if a >= b { a } else { b }
}

fn lower(s: &[u16]) -> S16 {
    String::from_utf16_lossy(s).to_lowercase().encode_utf16().collect()
}

/// `s[i..j]` in CoffeeScript (inclusive, clamped like String.prototype.slice).
fn slice(s: &[u16], i: usize, j_incl: usize) -> S16 {
    let end = (j_incl + 1).min(s.len());
    if i >= end { Vec::new() } else { s[i..end].to_vec() }
}

fn is_digit(c: u16) -> bool { (b'0' as u16..=b'9' as u16).contains(&c) }
fn is_upper(c: u16) -> bool { (b'A' as u16..=b'Z' as u16).contains(&c) }
fn is_lower(c: u16) -> bool { (b'a' as u16..=b'z' as u16).contains(&c) }
fn is_line_terminator(c: u16) -> bool { matches!(c, 0x0A | 0x0D | 0x2028 | 0x2029) }

#[derive(Clone)]
enum Pat {
    Dictionary { rank: f64, reversed: bool, l33t: bool, sub: Vec<(u16, u16)> },
    Spatial { keyboard: bool, turns: u32, shifted: u32 },
    Repeat { base_guesses: f64, repeat_count: f64 },
    Sequence { ascending: bool },
    Regex { year: f64 },
    Date { year: f64, separator: bool },
    Bruteforce,
}

#[derive(Clone)]
struct Match {
    i: usize,
    j: usize,
    token: S16,
    pat: Pat,
    guesses: Option<f64>,
}

fn sorted(mut m: Vec<Match>) -> Vec<Match> {
    m.sort_by(|a, b| a.i.cmp(&b.i).then(a.j.cmp(&b.j)));
    m
}

// ------------------------------------------------------------------------------------------
// matching
// ------------------------------------------------------------------------------------------
fn omnimatch(pw: &[u16]) -> Result<Vec<Match>, ()> {
    let mut m = dictionary_match(pw);
    m.extend(reverse_dictionary_match(pw));
    m.extend(l33t_match(pw));
    m.extend(spatial_match(pw));
    m.extend(repeat_match(pw)?);
    m.extend(sequence_match(pw));
    m.extend(regex_match(pw));
    m.extend(date_match(pw));
    Ok(sorted(m))
}

fn dict_rank(d: &HashMap<S16, f64>, word: &[u16]) -> Option<f64> {
    if let Some(r) = d.get(word) {
        return Some(*r);
    }
    // `word of ranked_dict` also finds Object.prototype's lowercase members; their value is not a number
    const CONSTRUCTOR: [u16; 11] = [99, 111, 110, 115, 116, 114, 117, 99, 116, 111, 114];
    const PROTO: [u16; 9] = [95, 95, 112, 114, 111, 116, 111, 95, 95];
    if word == CONSTRUCTOR || word == PROTO { Some(f64::NAN) } else { None }
}

fn dictionary_match(pw: &[u16]) -> Vec<Match> {
    let mut out = Vec::new();
    let len = pw.len();
    let pl = lower(pw);
    for d in &data().dicts {
        for i in 0..len {
            for j in i..len {
                let word = slice(&pl, i, j);
                if let Some(rank) = dict_rank(d, &word) {
                    out.push(Match { i, j, token: slice(pw, i, j), pat: Pat::Dictionary { rank, reversed: false, l33t: false, sub: vec![] }, guesses: None });
                }
            }
        }
    }
    sorted(out)
}

fn reverse_dictionary_match(pw: &[u16]) -> Vec<Match> {
    let rev: S16 = pw.iter().rev().copied().collect();
    let mut m = dictionary_match(&rev);
    let n = pw.len();
    for x in &mut m {
        x.token.reverse();
        if let Pat::Dictionary { reversed, .. } = &mut x.pat { *reversed = true; }
        let (i, j) = (n - 1 - x.j, n - 1 - x.i);
        x.i = i;
        x.j = j;
    }
    sorted(m)
}

const L33T_TABLE: [(char, &str); 12] = [
    ('a', "4@"), ('b', "8"), ('c', "({[<"), ('e', "3"), ('g', "69"), ('i', "1!|"),
    ('l', "1|7"), ('o', "0"), ('s', "$5"), ('t', "+7"), ('x', "%"), ('z', "2"),
];

fn enumerate_l33t_subs(table: &[(u16, Vec<u16>)]) -> Vec<Vec<(u16, u16)>> {
    let mut subs: Vec<Vec<(u16, u16)>> = vec![vec![]];
    for (first_key, chrs) in table {
        let mut next: Vec<Vec<(u16, u16)>> = Vec::new();
        for &l33t_chr in chrs {
            for sub in &subs {
                match sub.iter().position(|p| p.0 == l33t_chr) {
                    None => {
                        let mut e = sub.clone();
                        e.push((l33t_chr, *first_key));
                        next.push(e);
                    }
                    Some(dup) => {
                        let mut alt = sub.clone();
                        alt.remove(dup);
                        alt.push((l33t_chr, *first_key));
                        next.push(sub.clone());
                        next.push(alt);
                    }
                }
            }
        }
        // dedup by the same label zxcvbn.js builds (it carries list positions, so only exact repeats go)
        let mut seen = std::collections::HashSet::new();
        subs = next.into_iter().filter(|sub| {
            let mut assoc: Vec<String> = sub.iter().enumerate()
                .map(|(m, (c, l))| format!("{},{},{}", String::from_utf16_lossy(&[*c]), String::from_utf16_lossy(&[*l]), m))
                .collect();
            assoc.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            let label = assoc.iter().enumerate().map(|(p, a)| format!("{a},{p}")).collect::<Vec<_>>().join("-");
            seen.insert(label)
        }).collect();
    }
    // assoc lists -> dicts (insertion order kept)
    subs.into_iter().map(|sub| {
        let mut d: Vec<(u16, u16)> = Vec::new();
        for (k, v) in sub {
            if let Some(e) = d.iter_mut().find(|e| e.0 == k) { e.1 = v; } else { d.push((k, v)); }
        }
        d
    }).collect()
}

fn l33t_match(pw: &[u16]) -> Vec<Match> {
    let chars: std::collections::HashSet<u16> = pw.iter().copied().collect();
    let mut table: Vec<(u16, Vec<u16>)> = Vec::new();
    for (letter, subs) in L33T_TABLE {
        let rel: Vec<u16> = subs.encode_utf16().filter(|c| chars.contains(c)).collect();
        if !rel.is_empty() {
            table.push((letter as u16, rel));
        }
    }
    let mut out = Vec::new();
    for sub in enumerate_l33t_subs(&table) {
        if sub.is_empty() {
            break;
        }
        let subbed: S16 = pw.iter().map(|c| sub.iter().find(|p| p.0 == *c).map(|p| p.1).unwrap_or(*c)).collect();
        for mut m in dictionary_match(&subbed) {
            let token = slice(pw, m.i, m.j);
            let word = slice(&lower(&subbed), m.i, m.j);
            if lower(&token) == word {
                continue;
            }
            let match_sub: Vec<(u16, u16)> = sub.iter().filter(|(s, _)| token.contains(s)).copied().collect();
            if let Pat::Dictionary { l33t, sub: msub, .. } = &mut m.pat {
                *l33t = true;
                *msub = match_sub;
            }
            m.token = token;
            out.push(m);
        }
    }
    sorted(out.into_iter().filter(|m| m.token.len() > 1).collect())
}

fn spatial_match(pw: &[u16]) -> Vec<Match> {
    let mut out = Vec::new();
    for g in &data().graphs {
        out.extend(spatial_helper(pw, g));
    }
    sorted(out)
}

fn is_shifted(c: u16) -> bool {
    "~!@#$%^&*()_+QWERTYUIOP{}|ASDFGHJKL:\"ZXCVBNM<>?".encode_utf16().any(|x| x == c)
}

fn spatial_helper(pw: &[u16], g: &Graph) -> Vec<Match> {
    let keyboard = g.name == "qwerty" || g.name == "dvorak";
    let mut out = Vec::new();
    let mut i = 0usize;
    while pw.len() >= 2 && i < pw.len() - 1 {
        let mut j = i + 1;
        let mut last_direction: Option<i32> = None;
        let mut turns = 0u32;
        let mut shifted = if keyboard && is_shifted(pw[i]) { 1u32 } else { 0 };
        loop {
            let prev = pw[j - 1];
            let mut found = false;
            let mut cur_direction = -1i32;
            let empty = Vec::new();
            let adjacents = g.adj.get(&prev).unwrap_or(&empty);
            if j < pw.len() {
                let cur = pw[j];
                for adj in adjacents {
                    cur_direction += 1;
                    if let Some(a) = adj {
                        if let Some(pos) = a.iter().position(|x| *x == cur) {
                            found = true;
                            if pos == 1 {
                                shifted += 1;
                            }
                            if last_direction != Some(cur_direction) {
                                turns += 1;
                                last_direction = Some(cur_direction);
                            }
                            break;
                        }
                    }
                }
            }
            if found {
                j += 1;
            } else {
                if j - i > 2 {
                    out.push(Match { i, j: j - 1, token: pw[i..j].to_vec(), pat: Pat::Spatial { keyboard, turns, shifted }, guesses: None });
                }
                i = j;
                break;
            }
        }
    }
    out
}

/// One `/(.+)\1+/g` (greedy) or `/(.+?)\1+/g` (lazy) exec from `start`: (index, base length, length).
fn find_repeat(s: &[u16], start: usize, greedy: bool) -> Option<(usize, usize, usize)> {
    for p in start..s.len() {
        let run = s[p..].iter().take_while(|c| !is_line_terminator(**c)).count();
        let try_len = |l: usize| -> Option<usize> {
            if l == 0 || p + 2 * l > s.len() {
                return None;
            }
            let base = &s[p..p + l];
            let mut reps = 0;
            while p + (reps + 2) * l <= s.len() && &s[p + (reps + 1) * l..p + (reps + 2) * l] == base {
                reps += 1;
            }
            if reps >= 1 { Some((reps + 1) * l) } else { None }
        };
        if greedy {
            for l in (1..=run).rev() {
                if let Some(total) = try_len(l) {
                    return Some((p, l, total));
                }
            }
        } else {
            for l in 1..=run {
                if let Some(total) = try_len(l) {
                    return Some((p, l, total));
                }
            }
        }
    }
    None
}

fn repeat_match(pw: &[u16]) -> Result<Vec<Match>, ()> {
    let mut out = Vec::new();
    let mut last = 0usize;
    while last < pw.len() {
        let Some(g) = find_repeat(pw, last, true) else { break };
        let l = find_repeat(pw, last, false).expect("lazy matches where greedy does");
        let (index, total, base) = if g.2 > l.2 {
            let whole = &pw[g.0..g.0 + g.2];
            // /^(.+?)\1+$/ on the greedy match: its shortest repeating unit
            let base_len = (1..=whole.len() / 2).find(|&b| whole.len() % b == 0 && whole.chunks(b).all(|c| c == &whole[..b])).unwrap_or(whole.len());
            (g.0, g.2, whole[..base_len].to_vec())
        } else {
            (l.0, l.2, pw[l.0..l.0 + l.1].to_vec())
        };
        let (i, j) = (index, index + total - 1);
        let base_guesses = most_guessable(&base, omnimatch(&base)?, false)?;
        out.push(Match { i, j, token: pw[i..=j].to_vec(), pat: Pat::Repeat { base_guesses, repeat_count: total as f64 / base.len() as f64 }, guesses: None });
        last = j + 1;
    }
    Ok(out)
}

fn sequence_match(pw: &[u16]) -> Vec<Match> {
    if pw.len() == 1 || pw.is_empty() {
        return vec![];
    }
    let mut result = Vec::new();
    let mut update = |i: usize, j: usize, delta: i32| {
        if (j - i > 1 || delta.abs() == 1) && 0 < delta.abs() && delta.abs() <= 5 {
            let token = slice(pw, i, j);
            let ascending = delta > 0;
            result.push(Match { i, j, token, pat: Pat::Sequence { ascending }, guesses: None });
        }
    };
    let mut i = 0usize;
    let mut last_delta: Option<i32> = None;
    for k in 1..pw.len() {
        let delta = pw[k] as i32 - pw[k - 1] as i32;
        if last_delta.is_none() {
            last_delta = Some(delta);
        }
        if Some(delta) == last_delta {
            continue;
        }
        let j = k - 1;
        update(i, j, last_delta.unwrap());
        i = j;
        last_delta = Some(delta);
    }
    update(i, pw.len() - 1, last_delta.unwrap_or(0));
    result
}

fn regex_match(pw: &[u16]) -> Vec<Match> {
    // recent_year: /19\d\d|200\d|201\d/g
    let mut out = Vec::new();
    let mut p = 0usize;
    let d = |k: usize| pw.get(k).is_some_and(|c| is_digit(*c));
    let c = |k: usize, ch: u8| pw.get(k) == Some(&(ch as u16));
    while p + 4 <= pw.len() {
        let hit = (c(p, b'1') && c(p + 1, b'9') && d(p + 2) && d(p + 3))
            || (c(p, b'2') && c(p + 1, b'0') && c(p + 2, b'0') && d(p + 3))
            || (c(p, b'2') && c(p + 1, b'0') && c(p + 2, b'1') && d(p + 3));
        if hit {
            let token = pw[p..p + 4].to_vec();
            let year: f64 = String::from_utf16_lossy(&token).parse().unwrap();
            out.push(Match { i: p, j: p + 3, token, pat: Pat::Regex { year }, guesses: None });
            p += 4;
        } else {
            p += 1;
        }
    }
    sorted(out)
}

const DATE_MAX_YEAR: i64 = 2050;
const DATE_MIN_YEAR: i64 = 1000;

fn date_splits(len: usize) -> &'static [(usize, usize)] {
    match len {
        4 => &[(1, 2), (2, 3)],
        5 => &[(1, 3), (2, 3)],
        6 => &[(1, 2), (2, 4), (4, 5)],
        7 => &[(1, 3), (2, 3), (4, 5), (4, 6)],
        8 => &[(2, 4), (4, 6)],
        _ => &[],
    }
}

fn digits_value(s: &[u16]) -> i64 {
    s.iter().fold(0i64, |a, c| a * 10 + (*c - b'0' as u16) as i64)
}

fn map_ints_to_dm(a: i64, b: i64) -> Option<(i64, i64)> {
    for (d, m) in [(a, b), (b, a)] {
        if (1..=31).contains(&d) && (1..=12).contains(&m) {
            return Some((d, m));
        }
    }
    None
}

fn two_to_four(y: i64) -> i64 {
    if y > 99 { y } else if y > 50 { y + 1900 } else { y + 2000 }
}

/// Returns the year of the (year, month, day) mapping, if any.
fn map_ints_to_dmy(ints: [i64; 3]) -> Option<i64> {
    if ints[1] > 31 || ints[1] <= 0 {
        return None;
    }
    let (mut over_12, mut over_31, mut under_1) = (0, 0, 0);
    for &n in &ints {
        if (99 < n && n < DATE_MIN_YEAR) || n > DATE_MAX_YEAR {
            return None;
        }
        if n > 31 { over_31 += 1; }
        if n > 12 { over_12 += 1; }
        if n <= 0 { under_1 += 1; }
    }
    if over_31 >= 2 || over_12 == 3 || under_1 >= 2 {
        return None;
    }
    let splits = [(ints[2], (ints[0], ints[1])), (ints[0], (ints[1], ints[2]))];
    for (y, (a, b)) in splits {
        if (DATE_MIN_YEAR..=DATE_MAX_YEAR).contains(&y) {
            return map_ints_to_dm(a, b).map(|_| y);
        }
    }
    for (y, (a, b)) in splits {
        if map_ints_to_dm(a, b).is_some() {
            return Some(two_to_four(y));
        }
    }
    None
}

/// `new Date().getFullYear()`, taken once like the JS module does when it loads.
fn reference_year() -> f64 {
    static YEAR: OnceLock<f64> = OnceLock::new();
    *YEAR.get_or_init(|| {
        use chrono::Datelike;
        chrono::Local::now().year() as f64
    })
}

fn date_match(pw: &[u16]) -> Vec<Match> {
    let mut matches: Vec<(Match, i64)> = Vec::new();
    let n = pw.len() as i64;
    let ry = reference_year() as i64;
    for i in 0..=(n - 4).max(-1) {
        if n < 4 { break; }
        for j in (i + 3)..=(i + 7) {
            if j >= n { break; }
            let token = &pw[i as usize..=j as usize];
            if !token.iter().all(|c| is_digit(*c)) {
                continue;
            }
            let mut best: Option<i64> = None;
            let mut min_d = i64::MAX;
            for &(k, l) in date_splits(token.len()) {
                let ints = [digits_value(&token[..k]), digits_value(&token[k..l]), digits_value(&token[l..])];
                if let Some(y) = map_ints_to_dmy(ints) {
                    let d = (y - ry).abs();
                    if best.is_none() || d < min_d {
                        best = Some(y);
                        min_d = d;
                    }
                }
            }
            if let Some(y) = best {
                matches.push((Match { i: i as usize, j: j as usize, token: token.to_vec(), pat: Pat::Date { year: y as f64, separator: false }, guesses: None }, 0));
            }
        }
    }
    for i in 0..=(n - 6).max(-1) {
        if n < 6 { break; }
        for j in (i + 5)..=(i + 9) {
            if j >= n { break; }
            let t = &pw[i as usize..=j as usize];
            // ^(\d{1,4})([\s/\\_.-])(\d{1,2})\2(\d{1,4})$
            let d1 = t.iter().take_while(|c| is_digit(**c)).count();
            if !(1..=4).contains(&d1) || d1 >= t.len() { continue; }
            let sep = t[d1];
            let sep_ok = char::from_u32(sep as u32).is_some_and(crate::jsstr::is_js_space) || matches!(sep, 0x2F | 0x5C | 0x5F | 0x2E | 0x2D);
            if !sep_ok { continue; }
            let rest = &t[d1 + 1..];
            let d2 = rest.iter().take_while(|c| is_digit(**c)).count();
            if !(1..=2).contains(&d2) || d2 >= rest.len() || rest[d2] != sep { continue; }
            let last = &rest[d2 + 1..];
            if !(1..=4).contains(&last.len()) || !last.iter().all(|c| is_digit(*c)) { continue; }
            let ints = [digits_value(&t[..d1]), digits_value(&rest[..d2]), digits_value(last)];
            if let Some(y) = map_ints_to_dmy(ints) {
                matches.push((Match { i: i as usize, j: j as usize, token: t.to_vec(), pat: Pat::Date { year: y as f64, separator: true }, guesses: None }, 1));
            }
        }
    }
    let spans: Vec<(usize, usize)> = matches.iter().map(|(m, _)| (m.i, m.j)).collect();
    let kept: Vec<Match> = matches.iter().enumerate().filter(|(a, (m, _))| {
        !spans.iter().enumerate().any(|(b, (oi, oj))| b != *a && *oi <= m.i && *oj >= m.j)
    }).map(|(_, (m, _))| m.clone()).collect();
    sorted(kept)
}

// ------------------------------------------------------------------------------------------
// scoring
// ------------------------------------------------------------------------------------------
const MIN_GUESSES_BEFORE_GROWING_SEQUENCE: f64 = 10000.0;
const MIN_SUBMATCH_GUESSES_SINGLE_CHAR: f64 = 10.0;
const MIN_SUBMATCH_GUESSES_MULTI_CHAR: f64 = 50.0;
const MIN_YEAR_SPACE: f64 = 20.0;

fn n_ck(mut n: f64, k: f64) -> f64 {
    if k > n { return 0.0; }
    if k == 0.0 { return 1.0; }
    let mut r = 1.0;
    let mut d = 1.0;
    while d <= k {
        r *= n;
        r /= d;
        n -= 1.0;
        d += 1.0;
    }
    r
}

fn factorial(n: usize) -> f64 {
    if n < 2 { return 1.0; }
    (2..=n).fold(1.0, |f, i| f * i as f64)
}

fn uppercase_variations(word: &[u16]) -> f64 {
    let all_lower = !word.is_empty() && word.iter().all(|c| !is_upper(*c));
    if all_lower || lower(word) == word {
        return 1.0;
    }
    let n = word.len();
    let start_upper = n >= 2 && is_upper(word[0]) && word[1..].iter().all(|c| !is_upper(*c));
    let end_upper = n >= 2 && is_upper(word[n - 1]) && word[..n - 1].iter().all(|c| !is_upper(*c));
    let all_upper = !word.is_empty() && word.iter().all(|c| !is_lower(*c));
    if start_upper || end_upper || all_upper {
        return 2.0;
    }
    let u = word.iter().filter(|c| is_upper(**c)).count() as f64;
    let l = word.iter().filter(|c| is_lower(**c)).count() as f64;
    let mut v = 0.0;
    let mut i = 1.0;
    while i <= u.min(l) {
        v += n_ck(u + l, i);
        i += 1.0;
    }
    v
}

fn l33t_variations(token: &[u16], l33t: bool, sub: &[(u16, u16)]) -> f64 {
    if !l33t { return 1.0; }
    let mut variations = 1.0;
    let chrs = lower(token);
    for (subbed, unsubbed) in sub {
        let s = chrs.iter().filter(|c| *c == subbed).count() as f64;
        let u = chrs.iter().filter(|c| *c == unsubbed).count() as f64;
        if s == 0.0 || u == 0.0 {
            variations *= 2.0;
        } else {
            let p = u.min(s);
            let mut possibilities = 0.0;
            let mut i = 1.0;
            while i <= p {
                possibilities += n_ck(u + s, i);
                i += 1.0;
            }
            variations *= possibilities;
        }
    }
    variations
}

fn estimate_guesses(m: &mut Match, pw_len: usize) -> f64 {
    if let Some(g) = m.guesses {
        return g;
    }
    let mut min_guesses = 1.0;
    if m.token.len() < pw_len {
        min_guesses = if m.token.len() == 1 { MIN_SUBMATCH_GUESSES_SINGLE_CHAR } else { MIN_SUBMATCH_GUESSES_MULTI_CHAR };
    }
    let d = data();
    let ry = reference_year();
    let guesses = match &m.pat {
        Pat::Bruteforce => {
            let mut g = 10f64.powf(m.token.len() as f64);
            if g == f64::INFINITY { g = f64::MAX; }
            let min = if m.token.len() == 1 { MIN_SUBMATCH_GUESSES_SINGLE_CHAR + 1.0 } else { MIN_SUBMATCH_GUESSES_MULTI_CHAR + 1.0 };
            js_max(g, min)
        }
        Pat::Dictionary { rank, reversed, l33t, sub } => {
            *rank * uppercase_variations(&m.token) * l33t_variations(&m.token, *l33t, sub) * if *reversed { 2.0 } else { 1.0 }
        }
        Pat::Spatial { keyboard, turns, shifted } => {
            let (s, deg) = if *keyboard { (d.keyboard_starting, d.keyboard_avg_degree) } else { (d.keypad_starting, d.keypad_avg_degree) };
            let mut g = 0.0;
            let l = m.token.len();
            let t = *turns as usize;
            for i in 2..=l {
                let possible = t.min(i - 1);
                for j in 1..=possible {
                    g += n_ck((i - 1) as f64, (j - 1) as f64) * s * deg.powf(j as f64);
                }
            }
            if *shifted > 0 {
                let sh = *shifted as f64;
                let un = m.token.len() as f64 - sh;
                if sh == 0.0 || un == 0.0 {
                    g *= 2.0;
                } else {
                    let mut v = 0.0;
                    let mut i = 1.0;
                    while i <= sh.min(un) {
                        v += n_ck(sh + un, i);
                        i += 1.0;
                    }
                    g *= v;
                }
            }
            g
        }
        Pat::Repeat { base_guesses, repeat_count } => base_guesses * repeat_count,
        Pat::Sequence { ascending } => {
            let first = m.token[0];
            let mut base = if [b'a', b'A', b'z', b'Z', b'0', b'1', b'9'].iter().any(|c| *c as u16 == first) {
                4.0
            } else if is_digit(first) {
                10.0
            } else {
                26.0
            };
            if !ascending { base *= 2.0; }
            base * m.token.len() as f64
        }
        Pat::Regex { year } => js_max((year - ry).abs(), MIN_YEAR_SPACE),
        Pat::Date { year, separator } => {
            let ys = js_max((year - ry).abs(), MIN_YEAR_SPACE);
            let mut g = ys * 365.0;
            if *separator { g *= 4.0; }
            g
        }
    };
    let g = js_max(guesses, min_guesses);
    m.guesses = Some(g);
    g
}

/// scoring.most_guessable_match_sequence -> guesses (Err where zxcvbn.js would throw).
fn most_guessable(pw: &[u16], matches: Vec<Match>, exclude_additive: bool) -> Result<f64, ()> {
    let n = pw.len();
    if n == 0 {
        return Ok(1.0);
    }
    let mut all: Vec<Match> = matches;
    let mut by_j: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (idx, m) in all.iter().enumerate() {
        by_j[m.j].push(idx);
    }
    for lst in &mut by_j {
        lst.sort_by_key(|&x| all[x].i);
    }
    let mut om: Vec<BTreeMap<usize, usize>> = vec![BTreeMap::new(); n];
    let mut opi: Vec<BTreeMap<usize, f64>> = vec![BTreeMap::new(); n];
    let mut og: Vec<BTreeMap<usize, f64>> = vec![BTreeMap::new(); n];

    let update = |all: &mut Vec<Match>, mi: usize, l: usize, om: &mut Vec<BTreeMap<usize, usize>>, opi: &mut Vec<BTreeMap<usize, f64>>, og: &mut Vec<BTreeMap<usize, f64>>| {
        let k = all[mi].j;
        let mut pi = estimate_guesses(&mut all[mi], n);
        if l > 1 {
            let prev = opi[all[mi].i - 1].get(&(l - 1)).copied().unwrap_or(f64::NAN);
            pi *= prev;
        }
        let mut g = factorial(l) * pi;
        if !exclude_additive {
            g += MIN_GUESSES_BEFORE_GROWING_SEQUENCE.powf((l - 1) as f64);
        }
        for (&cl, &cg) in &og[k] {
            if cl > l { continue; }
            if cg <= g { return; }
        }
        og[k].insert(l, g);
        om[k].insert(l, mi);
        opi[k].insert(l, pi);
    };

    for k in 0..n {
        for idx in by_j[k].clone() {
            if all[idx].i > 0 {
                let ls: Vec<usize> = om[all[idx].i - 1].keys().copied().collect();
                for l in ls {
                    update(&mut all, idx, l + 1, &mut om, &mut opi, &mut og);
                }
            } else {
                update(&mut all, idx, 1, &mut om, &mut opi, &mut og);
            }
        }
        // bruteforce_update(k)
        all.push(Match { i: 0, j: k, token: pw[..=k].to_vec(), pat: Pat::Bruteforce, guesses: None });
        let bi = all.len() - 1;
        update(&mut all, bi, 1, &mut om, &mut opi, &mut og);
        for i in 1..=k {
            all.push(Match { i, j: k, token: pw[i..=k].to_vec(), pat: Pat::Bruteforce, guesses: None });
            let bi = all.len() - 1;
            let entries: Vec<(usize, usize)> = om[i - 1].iter().map(|(l, m)| (*l, *m)).collect();
            for (l, last_m) in entries {
                if matches!(all[last_m].pat, Pat::Bruteforce) { continue; }
                update(&mut all, bi, l + 1, &mut om, &mut opi, &mut og);
            }
        }
    }
    // unwind
    let mut k = n as isize - 1;
    let mut l: Option<usize> = None;
    let mut g = f64::INFINITY;
    for (&cl, &cg) in &og[n - 1] {
        if cg < g {
            l = Some(cl);
            g = cg;
        }
    }
    let mut seq_len = 0usize;
    let mut cur_l = l;
    while k >= 0 {
        let Some(mi) = cur_l.and_then(|cl| om[k as usize].get(&cl).copied()) else { return Err(()) };
        seq_len += 1;
        k = all[mi].i as isize - 1;
        cur_l = cur_l.and_then(|cl| cl.checked_sub(1));
    }
    Ok(og[n - 1].get(&seq_len).copied().unwrap_or(f64::NAN))
}

/// `zxcvbn(password).guesses`; Err where zxcvbn.js throws.
pub fn guesses(password: &str) -> Result<f64, ()> {
    let pw: S16 = password.encode_utf16().collect();
    most_guessable(&pw, omnimatch(&pw)?, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn basics() {
        assert_eq!(guesses("").unwrap(), 1.0);
        assert!(guesses("password").unwrap() < 10.0);
        assert!(guesses("correct horse battery staple").unwrap() > 1e20);
        assert!(guesses("constructor").is_ok());
    }
}

/// Loads the dictionaries on a background thread at start-up.
pub fn warm_up_in_background() {
    std::thread::spawn(warm_up);
}
