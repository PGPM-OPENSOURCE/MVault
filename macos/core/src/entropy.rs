//! The answer-strength estimate shown while building a vault (answer-questions.js), including the
//! JavaScript number formatting it displays (toFixed / toExponential round half up).

use crate::build::Hint;
use crate::jsstr::utf16_len;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Weak,
    Warn,
    Mid,
    Good,
    Strong,
}

/// "Entropy: 12.3 bits" for one answer, plus its colour level.
pub struct AnswerEntropy {
    pub text: String,
    pub level: Level,
}

/// `Math.log2(zxcvbn(answer).guesses || 1)`; None where zxcvbn.js throws.
pub fn log2_guesses(answer: &str) -> Option<f64> {
    let g = crate::zxcvbn::guesses(answer).ok()?;
    Some(if g.is_nan() || g == 0.0 { 0.0 } else { g.log2() })
}

/// answer-questions.js `updateEntropy`. `answer` is the raw input value. None where the page's
/// handler would stop on a zxcvbn.js exception (the line then keeps its previous text).
pub fn answer_entropy(answer: &str, hint: Option<&Hint>, is_trap: bool) -> Option<AnswerEntropy> {
    if is_trap {
        return Some(AnswerEntropy { text: "Entropy: 0.0 bits (red herring, not part of the key)".into(), level: Level::Warn });
    }
    let answer = crate::jsstr::normalize_answer(answer);
    let level_of = |e: f64| if e >= 40.0 { Level::Strong } else if e >= 20.0 { Level::Mid } else { Level::Weak };
    if let Some(h) = hint.filter(|h| h.mcq_enabled) {
        let n = h.mcq_options.len().clamp(2, 12) as f64;
        let e = n.log2().max(0.0);
        return Some(AnswerEntropy { text: format!("Entropy: {} bits", to_fixed(e, 1)), level: level_of(e) });
    }
    const BINARY: [&str; 8] = ["t", "f", "y", "n", "true", "false", "yes", "no"];
    let base = if BINARY.contains(&answer.as_str()) { 1.0 } else { log2_guesses(&answer)? };
    let letters = hint.map(|h| h.letters.len()).unwrap_or(0) as f64;
    let letters_penalty = letters * 26f64.log2();
    let show_length = hint.and_then(|h| h.show_length).unwrap_or(true);
    let length_penalty = if show_length { (utf16_len(&answer).min(20).max(1) as f64).log2() } else { 0.0 };
    // Math.max(0, x): NaN stays NaN ("Entropy: NaN bits")
    let x = base - letters_penalty - length_penalty;
    let e = if x.is_nan() { f64::NAN } else { x.max(0.0) };
    Some(AnswerEntropy { text: format!("Entropy: {} bits", to_fixed(e, 1)), level: level_of(e) })
}

/// The "Estimated Vault Strength" block.
pub struct Total {
    pub bits_text: String,
    pub assessment: &'static str,
    pub assessment_level: Level,
    pub combinations: String,
    pub crack_time: String,
    pub tier: &'static str,
    pub tier_level: Level,
}

/// answer-questions.js `calculateTotalEntropy`: sums the numbers shown in each "Entropy: x bits" line.
pub fn total(lines: &[String]) -> Total {
    let mut bits = 0.0f64;
    for l in lines {
        if let Some(v) = first_bits_number(l) {
            bits += v;
        }
    }
    let (assessment, assessment_level) = if bits < 40.0 {
        ("Very Weak", Level::Weak)
    } else if bits < 80.0 {
        ("Weak", Level::Warn)
    } else if bits < 120.0 {
        ("Moderate", Level::Mid)
    } else if bits < 180.0 {
        ("Strong", Level::Good)
    } else {
        ("Very Strong", Level::Strong)
    };
    let combos = 2f64.powf(bits);
    let combinations = if combos <= 1.0 {
        "0".to_string()
    } else {
        let exp = combos.log10().floor();
        let base = combos / 10f64.powf(exp);
        format!("{} × 10^{}", to_fixed(base, 2), js_num(exp))
    };
    let s = combos / 1e12;
    let crack_time = if s < 1.0 {
        "Instantly crackable".to_string()
    } else if s < 60.0 {
        format!("~{} second{}", js_num(s.ceil()), if s > 1.0 { "s" } else { "" })
    } else if s < 3600.0 {
        format!("~{} minute{}", js_num((s / 60.0).ceil()), if s >= 120.0 { "s" } else { "" })
    } else if s < 86400.0 {
        format!("~{} hour{}", js_num((s / 3600.0).ceil()), if s >= 7200.0 { "s" } else { "" })
    } else if s < 31_536_000.0 {
        format!("~{} day{}", js_num((s / 86400.0).ceil()), if s >= 172_800.0 { "s" } else { "" })
    } else if s < 1e21 {
        let years = s / 31_536_000.0;
        format!("~{} year{}", to_fixed(years, 1), if years > 1.1 { "s" } else { "" })
    } else {
        format!("~{} years", to_exponential(s, 2))
    };
    let (tier, tier_level) = if bits < 40.0 {
        ("Crackable instantly — trivial for any attacker", Level::Weak)
    } else if bits < 80.0 {
        ("Vulnerable to distributed attacks (≤ 79 bits)", Level::Warn)
    } else if bits < 120.0 {
        ("Resistant to consumer + small-scale parallel attacks (≥ 80 bits)", Level::Mid)
    } else if bits < 180.0 {
        ("Strong even against enterprise GPU clusters (≥ 120 bits)", Level::Good)
    } else {
        ("Functionally immune to brute-force (≥ 180 bits)", Level::Strong)
    };
    Total { bits_text: to_fixed(bits, 1), assessment, assessment_level, combinations, crack_time, tier, tier_level }
}

/// `text.match(/([\d.]+) bits/)` then parseFloat.
fn first_bits_number(s: &str) -> Option<f64> {
    let idx = s.find(" bits")?;
    let head = &s[..idx];
    let start = head.rfind(|c: char| !(c.is_ascii_digit() || c == '.')).map(|i| i + 1).unwrap_or(0);
    let num = &head[start..];
    if num.is_empty() {
        return None;
    }
    parse_float_prefix(num)
}

/// JavaScript parseFloat on a string of digits and dots ("1.2.3" -> 1.2).
fn parse_float_prefix(s: &str) -> Option<f64> {
    let mut end = 0;
    let mut dot = false;
    for (i, c) in s.char_indices() {
        if c == '.' {
            if dot { break; }
            dot = true;
        }
        end = i + 1;
    }
    let t = &s[..end];
    let t = t.trim_end_matches('.');
    if t.is_empty() { None } else { t.parse().ok() }
}

/// `String(x)` for the integers and infinities shown here.
pub fn js_num(x: f64) -> String {
    if x.is_nan() {
        "NaN".into()
    } else if x.is_infinite() {
        if x > 0.0 { "Infinity".into() } else { "-Infinity".into() }
    } else if x.fract() == 0.0 && x.abs() < 1e21 {
        format!("{}", x as i128)
    } else {
        format!("{x}")
    }
}

/// Exact decimal expansion of a finite double: (digits, decimal point position).
fn exact_decimal(x: f64) -> (Vec<u8>, i32) {
    // 1100 fractional digits cover every finite double exactly
    let s = format!("{:.1100}", x.abs());
    let (int_part, frac) = s.split_once('.').unwrap_or((&s, ""));
    let mut digits: Vec<u8> = int_part.bytes().chain(frac.bytes()).map(|b| b - b'0').collect();
    let mut point = int_part.len() as i32;
    while digits.len() > 1 && digits[0] == 0 {
        digits.remove(0);
        point -= 1;
    }
    while digits.len() > 1 && *digits.last().unwrap() == 0 {
        digits.pop();
    }
    (digits, point)
}

/// Rounds the exact decimal half-up at `keep` digits; returns digits (len keep, may carry) and point.
fn round_half_up(digits: &[u8], point: i32, keep: i32) -> (Vec<u8>, i32) {
    if keep < 0 {
        return (vec![], point);
    }
    let keep = keep as usize;
    let mut out: Vec<u8> = digits.iter().copied().take(keep).collect();
    while out.len() < keep {
        out.push(0);
    }
    let round_up = digits.get(keep).copied().unwrap_or(0) >= 5;
    let mut point = point;
    if round_up {
        let mut i = keep;
        loop {
            if i == 0 {
                out.insert(0, 1);
                point += 1;
                out.pop();
                if out.len() < keep { out.push(0); }
                break;
            }
            i -= 1;
            if out[i] == 9 {
                out[i] = 0;
            } else {
                out[i] += 1;
                break;
            }
        }
    }
    (out, point)
}

/// Number.prototype.toFixed (for |x| < 1e21).
pub fn to_fixed(x: f64, f: usize) -> String {
    if !x.is_finite() {
        return js_num(x);
    }
    if x.abs() >= 1e21 {
        return js_num(x);
    }
    let neg = x < 0.0;
    let (digits, point) = exact_decimal(x);
    let keep = point + f as i32;
    let (rounded, point) = if keep < 0 { (vec![], point) } else { round_half_up(&digits, point, keep) };
    // render: integer part = first `point` digits
    let mut all: Vec<u8> = rounded;
    let mut point = point;
    if all.is_empty() || all.iter().all(|&d| d == 0) {
        all = vec![0; (point.max(0) + f as i32).max(1) as usize];
    }
    while point <= 0 {
        all.insert(0, 0);
        point += 1;
    }
    while (all.len() as i32) < point + f as i32 {
        all.push(0);
    }
    let int: String = all[..point as usize].iter().map(|d| (d + b'0') as char).collect();
    let frac: String = all[point as usize..point as usize + f].iter().map(|d| (d + b'0') as char).collect();
    let int = int.trim_start_matches('0');
    let int = if int.is_empty() { "0" } else { int };
    let body = if f > 0 { format!("{int}.{frac}") } else { int.to_string() };
    let is_zero = body.chars().all(|c| c == '0' || c == '.');
    if neg && !is_zero { format!("-{body}") } else { body }
}

/// Number.prototype.toExponential(f).
pub fn to_exponential(x: f64, f: usize) -> String {
    if !x.is_finite() {
        return js_num(x);
    }
    if x == 0.0 {
        return format!("{}e+0", to_fixed(0.0, f));
    }
    let neg = x < 0.0;
    let (digits, point) = exact_decimal(x);
    let (rounded, point) = round_half_up(&digits, point, f as i32 + 1);
    let e = point - 1;
    let mant: String = rounded.iter().map(|d| (d + b'0') as char).collect();
    let mant = if f > 0 { format!("{}.{}", &mant[..1], &mant[1..]) } else { mant };
    format!("{}{}e{}{}", if neg { "-" } else { "" }, mant, if e >= 0 { "+" } else { "-" }, e.abs())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn to_fixed_matches_js() {
        assert_eq!(to_fixed(0.25, 1), "0.3");
        assert_eq!(to_fixed(0.35, 1), "0.3"); // 0.35 is 0.34999... as a double
        assert_eq!(to_fixed(12.0, 1), "12.0");
        assert_eq!(to_fixed(0.0, 1), "0.0");
        assert_eq!(to_fixed(9.96, 1), "10.0");
        assert_eq!(to_fixed(1.005, 2), "1.00");
        assert_eq!(to_fixed(57.900000000000006, 1), "57.9");
        assert_eq!(to_fixed(0.04, 1), "0.0");
        assert_eq!(to_fixed(123.456, 2), "123.46");
    }
    #[test]
    fn to_exponential_matches_js() {
        assert_eq!(to_exponential(1.2345e25, 2), "1.23e+25");
        assert_eq!(to_exponential(9.999e30, 2), "1.00e+31");
        assert_eq!(to_exponential(f64::INFINITY, 2), "Infinity");
    }
    #[test]
    fn totals() {
        let t = total(&["Entropy: 0.0 bits".into()]);
        assert_eq!(t.combinations, "0");
        assert_eq!(t.crack_time, "Instantly crackable");
        let t = total(&vec!["Entropy: 100.0 bits".to_string(); 12]);
        assert_eq!(t.bits_text, "1200.0");
        assert_eq!(t.combinations, "NaN × 10^Infinity");
        assert_eq!(t.crack_time, "~Infinity years");
    }
}
