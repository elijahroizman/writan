//! Number formats: how Excel shows a cell's value, from format codes like `#,##0.00`,
//! `0%`, `d-mmm-yy` and `[Red](#,##0);0`. Formatting follows Excel's US English rules: a dot
//! for decimals and a comma between thousands.

use office_core::model::Value;

use crate::dates::{from_serial, DateSystem};

/// The format codes behind Excel's built-in format numbers (US English). Numbers not listed
/// are East Asian formats, shown as General here.
pub fn builtin(id: u32) -> Option<&'static str> {
    Some(match id {
        0 => "General",
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        5 => "\"$\"#,##0_);\\(\"$\"#,##0\\)",
        6 => "\"$\"#,##0_);[Red]\\(\"$\"#,##0\\)",
        7 => "\"$\"#,##0.00_);\\(\"$\"#,##0.00\\)",
        8 => "\"$\"#,##0.00_);[Red]\\(\"$\"#,##0.00\\)",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 => "m/d/yyyy",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "m/d/yyyy h:mm",
        37 => "#,##0 ;(#,##0)",
        38 => "#,##0 ;[Red](#,##0)",
        39 => "#,##0.00;(#,##0.00)",
        40 => "#,##0.00;[Red](#,##0.00)",
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mmss.0",
        48 => "##0.0E+0",
        49 => "@",
        _ => return None,
    })
}

/// A value as Excel shows it with a format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Formatted {
    pub text: String,
    /// A color the format asks for (`[Red]`), lowercase.
    pub color: Option<String>,
}

/// Shows `value` the way format `code` says.
pub fn format(value: &Value, code: &str, system: DateSystem) -> Formatted {
    let plain = |text: String| Formatted { text, color: None };
    let sections = split_sections(code);
    match value {
        Value::Empty => plain(String::new()),
        Value::Bool(b) => plain(if *b { "TRUE" } else { "FALSE" }.into()),
        Value::Error(e) => plain(e.clone()),
        Value::Text(_) | Value::Rich(_) => {
            let text = value.display();
            // The fourth section is for text, or a single section that has a place for it.
            let section = match sections.len() {
                4.. => Some(&sections[3]),
                1 if sections[0].contains('@') => Some(&sections[0]),
                _ => None,
            };
            match section {
                Some(s) => render_text(&tokenize(s), &text),
                None => plain(text),
            }
        }
        Value::Number(n) => format_number_with(*n, &sections, system),
    }
}

/// Whether a format shows numbers as dates or times.
pub fn is_date_format(code: &str) -> bool {
    split_sections(code).first().is_some_and(|s| tokenize(s).iter().any(|t| matches!(t, Token::Date(_) | Token::Elapsed(..))))
}

pub(crate) fn has_calendar_date(code: &str, n: f64) -> bool {
    let sections: Vec<_> = split_sections(code).iter().map(|s| tokenize(s)).collect();
    if sections.is_empty() {
        return false;
    }
    let (tokens, _) = number_section(&sections, n);
    !tokens.iter().any(|t| matches!(t, Token::General))
        && tokens.iter().enumerate().any(|(i, t)| match date_field(t) {
            Some('d' | 'y') => true,
            Some('m') => !is_minute(tokens, i),
            _ => false,
        })
}

fn format_number_with(n: f64, sections: &[String], system: DateSystem) -> Formatted {
    if !n.is_finite() {
        return Formatted { text: "#NUM!".into(), color: None };
    }
    if sections.is_empty() {
        return Formatted { text: general(n), color: None };
    }
    let parsed: Vec<Vec<Token>> = sections.iter().map(|s| tokenize(s)).collect();
    let (tokens, value) = number_section(&parsed, n);
    let mut out = if tokens.iter().any(|t| matches!(t, Token::General)) {
        Formatted { text: general(value), color: None }
    } else if tokens.iter().any(|t| matches!(t, Token::Text)) && !tokens.iter().any(|t| matches!(t, Token::Digit(_))) {
        // A text-only section (`@`) on a number shows the number itself.
        render_text(tokens, &general(value))
    } else if tokens.iter().any(|t| matches!(t, Token::Date(_) | Token::Elapsed(..))) {
        render_date(tokens, value, system)
    } else {
        render_number(tokens, value)
    };
    if let Some(Token::Color(c)) = tokens.iter().find(|t| matches!(t, Token::Color(_))) {
        out.color = Some(c.clone());
    }
    out
}

fn number_section(parsed: &[Vec<Token>], n: f64) -> (&[Token], f64) {
    let conditions: Vec<Option<Condition>> =
        parsed.iter().map(|t| t.iter().find_map(|t| if let Token::Condition(c) = t { Some(*c) } else { None })).collect();
    // Which section, and whether it shows the sign itself.
    let (index, abs) = if conditions.iter().any(Option::is_some) {
        let i = conditions.iter().take(parsed.len().min(3)).position(|c| c.is_some_and(|c| c.holds(n))).unwrap_or_else(|| {
            // Without a match, the first section without a condition, else the last one.
            conditions.iter().position(Option::is_none).unwrap_or(parsed.len() - 1)
        });
        (i, i > 0 && n < 0.0)
    } else {
        match (parsed.len(), n) {
            (1, _) => (0, false),
            (_, n) if n > 0.0 => (0, false),
            (_, n) if n < 0.0 => (1, true),
            (2, _) => (0, false),
            _ => (2, false),
        }
    };
    (&parsed[index.min(parsed.len() - 1)], if abs { -n } else { n })
}

/// General: whole numbers as they are, others to ten significant digits, and an exponent
/// when the number won't fit in eleven characters, as Excel's General does.
pub fn general(n: f64) -> String {
    if !n.is_finite() {
        return "#NUM!".into();
    }
    if n == 0.0 {
        return "0".into();
    }
    let abs = n.abs();
    if (1e-9..1e11).contains(&abs) {
        let digits_before = (abs.log10().floor() as i32 + 1).max(1);
        let decimals = (10 - digits_before).clamp(0, 10) as usize;
        let s = format!("{:.*}", decimals, n);
        let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
        if s != "0" && s != "-0" {
            return s;
        }
    }
    let s = format!("{:.5E}", n);
    // Rust writes 1.23457E11; Excel writes 1.23457E+11, without trailing zeros.
    let Some((mantissa, exp)) = s.split_once('E') else {
        return s;
    };
    let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
    let Ok(exp) = exp.parse::<i32>() else {
        return s;
    };
    format!("{mantissa}E{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs())
}

// ---- format codes ----

#[derive(Copy, Clone, Debug, PartialEq)]
struct Condition {
    op: &'static str,
    value: f64,
}

impl Condition {
    fn holds(&self, n: f64) -> bool {
        match self.op {
            ">=" => n >= self.value,
            "<=" => n <= self.value,
            "<>" => n != self.value,
            ">" => n > self.value,
            "<" => n < self.value,
            _ => n == self.value,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    General,
    Literal(String),
    /// `0`, `#` or `?`.
    Digit(char),
    Point,
    Comma,
    Percent,
    /// `E+` or `E-` (or lowercase), then the exponent's digits as following `Digit`s.
    Exponent(bool),
    Slash,
    Text,
    Color(String),
    Condition(Condition),
    /// y, m, d, h, s, AM/PM, A/P parts, as written (`yyyy`, `mmm`, `AM/PM`…).
    Date(String),
    /// `[h]`, `[mm]`, `[ss]`: hours, minutes or seconds in all, not of the day.
    Elapsed(char, usize),
    /// `.0`, `.00` after seconds.
    SubSecond(usize),
}

/// Splits a code into its sections at `;`, outside quotes, brackets and escapes.
fn split_sections(code: &str) -> Vec<String> {
    let mut sections = vec![String::new()];
    let mut chars = code.chars();
    let mut quoted = false;
    let mut bracket = false;
    while let Some(c) = chars.next() {
        let cur = sections.last_mut().unwrap();
        match c {
            '"' => quoted = !quoted,
            '[' if !quoted => bracket = true,
            ']' if !quoted => bracket = false,
            '\\' if !quoted => {
                cur.push(c);
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
                continue;
            }
            ';' if !quoted && !bracket => {
                sections.push(String::new());
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    sections
}

const COLORS: [&str; 8] = ["black", "blue", "cyan", "green", "magenta", "red", "white", "yellow"];

fn tokenize(section: &str) -> Vec<Token> {
    let chars: Vec<char> = section.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let lit = |out: &mut Vec<Token>, s: &str| match out.last_mut() {
        Some(Token::Literal(l)) => l.push_str(s),
        _ => out.push(Token::Literal(s.to_string())),
    };
    let mut seen_exponent = false;
    while i < chars.len() {
        let c = chars[i];
        let rest: String = chars[i..].iter().collect();
        let lower = rest.to_ascii_lowercase();
        match c {
            _ if lower.starts_with("general") => {
                out.push(Token::General);
                i += 7;
            }
            '"' => {
                let end = chars[i + 1..].iter().position(|&c| c == '"').map(|p| i + 1 + p).unwrap_or(chars.len());
                lit(&mut out, &chars[i + 1..end].iter().collect::<String>());
                i = end + 1;
            }
            '\\' => {
                if let Some(&n) = chars.get(i + 1) {
                    lit(&mut out, &n.to_string());
                }
                i += 2;
            }
            // `_x` leaves room as wide as x; a space is near enough. `*x` fills the cell; nothing here.
            '_' => {
                lit(&mut out, " ");
                i += 2;
            }
            '*' => i += 2,
            '[' => {
                let end = chars[i..].iter().position(|&c| c == ']').map(|p| i + p).unwrap_or(chars.len());
                let inside: String = chars[i + 1..end.min(chars.len())].iter().collect();
                let low = inside.to_ascii_lowercase();
                if COLORS.contains(&low.as_str()) || low.starts_with("color") {
                    out.push(Token::Color(low));
                } else if let Some(cond) = parse_condition(&inside) {
                    out.push(Token::Condition(cond));
                } else if !low.is_empty() && low.chars().all(|c| c == 'h') {
                    out.push(Token::Elapsed('h', low.len()));
                } else if !low.is_empty() && low.chars().all(|c| c == 'm') {
                    out.push(Token::Elapsed('m', low.len()));
                } else if !low.is_empty() && low.chars().all(|c| c == 's') {
                    out.push(Token::Elapsed('s', low.len()));
                } else if let Some(currency) = inside.strip_prefix('$') {
                    // [$€-407]: a currency symbol and a locale; the symbol shows.
                    lit(&mut out, currency.split('-').next().unwrap_or(""));
                }
                i = end + 1;
            }
            '0' | '#' | '?' => {
                out.push(Token::Digit(c));
                i += 1;
            }
            '.' => {
                // Right after seconds, a dot starts fractions of a second.
                let zeros = chars[i + 1..].iter().take_while(|&&c| c == '0').count();
                if zeros > 0
                    && matches!(out.iter().rev().find(|t| !matches!(t, Token::Literal(_))), Some(Token::Date(d)) if d.starts_with('s'))
                {
                    out.push(Token::SubSecond(zeros));
                    i += 1 + zeros;
                } else {
                    out.push(Token::Point);
                    i += 1;
                }
            }
            ',' => {
                out.push(Token::Comma);
                i += 1;
            }
            '%' => {
                out.push(Token::Percent);
                i += 1;
            }
            '@' => {
                out.push(Token::Text);
                i += 1;
            }
            '/' if out.iter().any(|t| matches!(t, Token::Digit(_))) && !out.iter().any(|t| matches!(t, Token::Date(_))) => {
                out.push(Token::Slash);
                i += 1;
            }
            'E' | 'e'
                if !seen_exponent && matches!(chars.get(i + 1), Some('+' | '-')) && out.iter().any(|t| matches!(t, Token::Digit(_))) =>
            {
                out.push(Token::Exponent(chars[i + 1] == '+'));
                seen_exponent = true;
                i += 2;
            }
            _ if lower.starts_with("am/pm") => {
                out.push(Token::Date("AM/PM".into()));
                i += 5;
            }
            _ if lower.starts_with("a/p") => {
                out.push(Token::Date(if c == 'a' { "a/p" } else { "A/P" }.into()));
                i += 3;
            }
            'y' | 'Y' | 'm' | 'M' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' => {
                let n = chars[i..].iter().take_while(|&&x| x.eq_ignore_ascii_case(&c)).count();
                out.push(Token::Date(c.to_ascii_lowercase().to_string().repeat(n)));
                i += n;
            }
            // Characters Excel shows as they are without quotes.
            c => {
                lit(&mut out, &c.to_string());
                i += 1;
            }
        }
    }
    out
}

fn parse_condition(s: &str) -> Option<Condition> {
    let s = s.trim();
    let op = [">=", "<=", "<>", ">", "<", "="].into_iter().find(|op| s.starts_with(op))?;
    let value = s[op.len()..].trim().parse().ok()?;
    Some(Condition { op, value })
}

// ---- rendering ----

fn render_text(tokens: &[Token], text: &str) -> Formatted {
    let mut out = String::new();
    let mut color = None;
    for t in tokens {
        match t {
            Token::Text => out.push_str(text),
            Token::Literal(l) => out.push_str(l),
            Token::Color(c) => color = Some(c.clone()),
            _ => {}
        }
    }
    Formatted { text: out, color }
}

fn render_number(tokens: &[Token], n: f64) -> Formatted {
    let mut value = n.abs();
    for t in tokens {
        if matches!(t, Token::Percent) {
            value *= 100.0;
        }
    }
    let split = tokens.iter().position(|t| matches!(t, Token::Point | Token::Exponent(_) | Token::Slash)).unwrap_or(tokens.len());
    if tokens.iter().any(|t| matches!(t, Token::Slash)) {
        return Formatted { text: render_fraction(tokens, value, n < 0.0), color: None };
    }
    let int_tokens = &tokens[..split];
    let is_digit = |t: Option<&Token>| matches!(t, Some(Token::Digit(_)));
    // Commas right after the last digit (of the integer or the decimals) divide by a thousand
    // each; a comma between integer digits groups thousands.
    let before_exponent = tokens.iter().position(|t| matches!(t, Token::Exponent(_))).unwrap_or(tokens.len());
    if let Some(last) = tokens[..before_exponent].iter().rposition(|t| matches!(t, Token::Digit(_))) {
        let mut j = last + 1;
        while matches!(tokens.get(j), Some(Token::Comma)) {
            value /= 1000.0;
            j += 1;
        }
    }
    let last_digit = int_tokens.iter().rposition(|t| matches!(t, Token::Digit(_)));
    let grouping = last_digit.is_some_and(|last| {
        (1..last).any(|i| matches!(int_tokens[i], Token::Comma) && is_digit(int_tokens.get(i - 1)) && is_digit(int_tokens.get(i + 1)))
    });
    let exponent = tokens.iter().position(|t| matches!(t, Token::Exponent(_)));
    let decimal_tokens: &[Token] = match tokens.get(split) {
        Some(Token::Point) => {
            let end = exponent.unwrap_or(tokens.len());
            &tokens[split + 1..end]
        }
        _ => &[],
    };
    let decimals = decimal_tokens.iter().filter(|t| matches!(t, Token::Digit(_))).count();
    let int_places = int_tokens.iter().filter(|t| matches!(t, Token::Digit(_))).count();

    // Scientific: the exponent is a multiple of the integer digits (##0.0E+0 is engineering).
    let mut exp = 0i32;
    if exponent.is_some() && value != 0.0 {
        let step = int_places.max(1) as i32;
        exp = (value.log10().floor() as i32).div_euclid(step) * step;
        value /= 10f64.powi(exp);
        // Rounding can carry the mantissa up a place: 9.996 as 0.00E+00 is 1.00E+01.
        if format!("{:.*}", decimals, round_decimal(value, decimals)).parse::<f64>().unwrap_or(value) >= 10f64.powi(step) {
            exp += step;
            value /= 10f64.powi(step);
        }
    }
    let rounded = format!("{:.*}", decimals, round_decimal(value, decimals));
    let (int_part, dec_part) = rounded.split_once('.').unwrap_or((&rounded, ""));
    // A value that rounds to zero shows no sign.
    let negative = n < 0.0 && rounded.bytes().any(|b| (b'1'..=b'9').contains(&b));

    // Integer digits fill the placeholders from the right; extra ones go on the first.
    let digits: Vec<char> = if int_part == "0" { Vec::new() } else { int_part.chars().collect() };
    let offset = digits.len() as isize - int_places as isize;
    let mut int_out = String::new();
    let mut place = 0isize;
    for t in int_tokens {
        match t {
            Token::Digit(p) => {
                let at = place + offset;
                if place == 0 && offset > 0 {
                    int_out.extend(&digits[..=offset as usize]);
                } else if at >= 0 {
                    int_out.push(digits[at as usize]);
                } else {
                    match p {
                        '0' => int_out.push('0'),
                        '?' => int_out.push(' '),
                        _ => {}
                    }
                }
                place += 1;
            }
            Token::Literal(l) => int_out.push_str(l),
            Token::Percent => int_out.push('%'),
            _ => {}
        }
    }
    // A format with digits only after the point still shows the number's whole part; one with
    // no digits at all shows only its text.
    if int_places == 0 && decimals > 0 && !digits.is_empty() {
        int_out.insert_str(0, int_part);
    }
    if grouping {
        int_out = group_thousands(&int_out);
    }
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    out.push_str(&int_out);

    if matches!(tokens.get(split), Some(Token::Point)) {
        out.push('.');
        let dec: Vec<char> = dec_part.chars().collect();
        // Trailing zeros under # are dropped and under ? become spaces, back to a 0 or a digit.
        let places: Vec<char> = decimal_tokens.iter().filter_map(|t| if let Token::Digit(p) = t { Some(*p) } else { None }).collect();
        let mut keep = places.len();
        while keep > 0 && places[keep - 1] != '0' && dec[keep - 1] == '0' {
            keep -= 1;
        }
        let mut d = 0;
        for t in decimal_tokens {
            match t {
                Token::Digit(p) => {
                    if d < keep {
                        out.push(dec[d]);
                    } else if *p == '?' {
                        out.push(' ');
                    }
                    d += 1;
                }
                Token::Literal(l) => out.push_str(l),
                Token::Percent => out.push('%'),
                _ => {}
            }
        }
    }
    if let Some(e) = exponent {
        let Token::Exponent(plus) = tokens[e] else { unreachable!() };
        let width = tokens[e + 1..].iter().filter(|t| matches!(t, Token::Digit(_))).count().max(1);
        out.push('E');
        if exp < 0 {
            out.push('-');
        } else if plus {
            out.push('+');
        }
        out.push_str(&format!("{:0width$}", exp.abs()));
        for t in &tokens[e + 1..] {
            match t {
                Token::Literal(l) => out.push_str(l),
                Token::Percent => out.push('%'),
                _ => {}
            }
        }
    }
    Formatted { text: out, color: None }
}

/// Round decimal ties away from zero, tolerating the representation error in values such
/// as 1.005. The correction is at most one floating-point rounding unit.
fn round_decimal(value: f64, decimals: usize) -> f64 {
    let factor = 10f64.powi(decimals.min(309) as i32);
    let scaled = value * factor;
    if !scaled.is_finite() || factor == 0.0 || scaled.abs() >= (1u64 << 52) as f64 {
        return value;
    }
    let rounded = (scaled.abs() + scaled.abs() * f64::EPSILON).round().copysign(value);
    rounded / factor
}

fn group_thousands(s: &str) -> String {
    // Group the run of digits, leaving literals around it alone.
    let start = s.find(|c: char| c.is_ascii_digit());
    let Some(start) = start else { return s.to_string() };
    let end = s[start..].find(|c: char| !c.is_ascii_digit()).map(|e| start + e).unwrap_or(s.len());
    let digits = &s[start..end];
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{}{}{}", &s[..start], grouped, &s[end..])
}

/// `# ?/?`, `# ??/??`, `?/8`: a whole part (if the code has one) and the nearest fraction with
/// that many denominator digits, or that denominator.
fn render_fraction(tokens: &[Token], value: f64, negative: bool) -> String {
    let slash = tokens.iter().position(|t| matches!(t, Token::Slash)).unwrap();
    let before: Vec<&Token> = tokens[..slash].iter().collect();
    // A whole part is a digit group, then a literal (the space), then the numerator's digits.
    let groups: Vec<usize> = {
        let mut g = Vec::new();
        let mut run = 0;
        for t in &before {
            match t {
                Token::Digit(_) => run += 1,
                _ if run > 0 => {
                    g.push(run);
                    run = 0;
                }
                _ => {}
            }
        }
        if run > 0 {
            g.push(run);
        }
        g
    };
    let whole_part = groups.len() >= 2;
    let denominator: String = tokens[slash + 1..]
        .iter()
        .take_while(|t| matches!(t, Token::Digit(_) | Token::Literal(_)))
        .map(|t| match t {
            Token::Digit(c) => c.to_string(),
            Token::Literal(l) => l.clone(),
            _ => unreachable!(),
        })
        .collect();
    let fixed = if denominator.chars().all(|c| c.is_ascii_digit()) { denominator.parse::<u64>().ok().filter(|d| *d > 0) } else { None };
    let places = denominator.chars().filter(|c| matches!(c, '0' | '?' | '#')).count();
    let max_den = 10u64.checked_pow(places.max(1).try_into().unwrap_or(u32::MAX)).unwrap_or(u64::MAX).saturating_sub(1);
    let (whole, frac) = if whole_part { (value.trunc(), value.fract()) } else { (0.0, value) };
    let (num, den) = match fixed {
        Some(d) => ((frac * d as f64).round() as u64, d),
        None => best_fraction(frac, max_den),
    };
    let (mut whole, mut num) = (whole as u64, num);
    if num == den && whole_part {
        whole = whole.saturating_add(1);
        num = 0;
    }
    let sign = if negative { "-" } else { "" };
    if whole_part {
        if num == 0 {
            return format!("{sign}{whole}");
        }
        if whole == 0 {
            return format!("{sign}{num}/{den}");
        }
        format!("{sign}{whole} {num}/{den}")
    } else {
        format!("{sign}{num}/{den}")
    }
}

/// The fraction nearest `x` (0–1) with a denominator no bigger than `max`, by continued fractions.
fn best_fraction(x: f64, max: u64) -> (u64, u64) {
    let max = max.max(1);
    let (mut p0, mut q0, mut p1, mut q1) = (0u64, 1u64, 1u64, 0u64);
    let mut remaining = x;
    for _ in 0..128 {
        let a = remaining.floor();
        if !a.is_finite() || a >= u64::MAX as f64 {
            break;
        }
        let a = a as u64;
        let Some(q2) = a.checked_mul(q1).and_then(|q| q.checked_add(q0)).filter(|q| *q <= max) else {
            break;
        };
        let Some(p2) = a.checked_mul(p1).and_then(|p| p.checked_add(p0)) else {
            break;
        };
        (p0, q0, p1, q1) = (p1, q1, p2, q2);
        if (x - p1 as f64 / q1 as f64).abs() <= f64::EPSILON * x.abs().max(1.0) {
            return (p1, q1);
        }
        let tail = remaining - a as f64;
        if tail == 0.0 {
            return (p1, q1);
        }
        remaining = 1.0 / tail;
    }
    if q1 == 0 {
        return (x.round() as u64, 1);
    }
    let k = (max - q0) / q1;
    let bound = k.checked_mul(p1).and_then(|p| p.checked_add(p0)).zip(k.checked_mul(q1).and_then(|q| q.checked_add(q0)));
    match bound {
        Some((p, q)) if q > 0 && (x - p as f64 / q as f64).abs() < (x - p1 as f64 / q1 as f64).abs() => (p, q),
        _ => (p1, q1),
    }
}

const MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

fn date_field(token: &Token) -> Option<char> {
    if let Token::Date(field) = token {
        field.chars().next()
    } else {
        None
    }
}

// An `m` means minutes right after hours or right before seconds.
fn is_minute(tokens: &[Token], i: usize) -> bool {
    let prev = tokens[..i].iter().rev().find_map(date_field);
    let next = tokens[i + 1..].iter().find_map(date_field);
    prev == Some('h') || next == Some('s') || prev.is_none() && tokens[..i].iter().any(|t| matches!(t, Token::Elapsed('h', _)))
}

fn render_date(tokens: &[Token], n: f64, system: DateSystem) -> Formatted {
    let Some(dt) = from_serial(n, system) else {
        // Excel fills the cell with #### for a date it can't show.
        return Formatted { text: "#".repeat(8), color: None };
    };
    let twelve = tokens.iter().any(|t| matches!(t, Token::Date(d) if d == "AM/PM" || d.eq_ignore_ascii_case("a/p")));
    let sub_digits = tokens.iter().find_map(|t| if let Token::SubSecond(n) = t { Some(*n) } else { None });
    // With fractions of a second shown, seconds aren't rounded up into the next one.
    let mut dt = dt;
    if sub_digits.is_none() && dt.milli >= 500 {
        if let Some(next) = from_serial(n + 0.5 / 86_400.0, system) {
            dt = next;
        }
    }
    let mut out = String::new();
    for (i, t) in tokens.iter().enumerate() {
        match t {
            Token::Literal(l) => out.push_str(l),
            Token::Date(d) => match d.as_str() {
                "AM/PM" => out.push_str(if dt.hour < 12 { "AM" } else { "PM" }),
                "a/p" => out.push(if dt.hour < 12 { 'a' } else { 'p' }),
                "A/P" => out.push(if dt.hour < 12 { 'A' } else { 'P' }),
                d if d.starts_with('y') => {
                    if d.len() <= 2 {
                        out.push_str(&format!("{:02}", dt.year.rem_euclid(100)));
                    } else {
                        out.push_str(&format!("{:04}", dt.year));
                    }
                }
                d if d.starts_with('m') && is_minute(tokens, i) => out.push_str(&pad(dt.minute, d.len())),
                d if d.starts_with('m') => match d.len() {
                    1 | 2 => out.push_str(&pad(dt.month, d.len())),
                    3 => out.push_str(&MONTHS[dt.month as usize - 1][..3]),
                    5 => out.push_str(&MONTHS[dt.month as usize - 1][..1]),
                    _ => out.push_str(MONTHS[dt.month as usize - 1]),
                },
                d if d.starts_with('d') => match d.len() {
                    1 | 2 => out.push_str(&pad(dt.day, d.len())),
                    3 => out.push_str(&DAYS[dt.weekday() as usize - 1][..3]),
                    _ => out.push_str(DAYS[dt.weekday() as usize - 1]),
                },
                d if d.starts_with('h') => {
                    let h = if twelve { (dt.hour + 11) % 12 + 1 } else { dt.hour };
                    out.push_str(&pad(h, d.len()));
                }
                d if d.starts_with('s') => out.push_str(&pad(dt.second, d.len())),
                other => out.push_str(other),
            },
            Token::Elapsed(unit, width) => {
                let total_seconds = (n * 86_400.0).round() as i64;
                let v = match unit {
                    'h' => total_seconds.div_euclid(3600),
                    'm' => total_seconds.div_euclid(60),
                    _ => total_seconds,
                };
                out.push_str(&format!("{:0w$}", v, w = *width));
            }
            Token::SubSecond(digits) => {
                let frac = dt.milli as f64 / 1000.0;
                let s = format!("{:.*}", digits, frac);
                out.push_str(&s[1..]);
            }
            Token::Digit(c) => out.push(*c),
            Token::Point => out.push('.'),
            Token::Comma => out.push(','),
            Token::Percent => out.push('%'),
            _ => {}
        }
    }
    Formatted { text: out, color: None }
}

fn pad(v: u32, width: usize) -> String {
    if width >= 2 {
        format!("{v:02}")
    } else {
        v.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(n: f64, code: &str) -> String {
        format(&Value::Number(n), code, DateSystem::From1900).text
    }

    #[test]
    fn bounded_fractions_match_exhaustive_small_denominators() {
        for max in [1, 9, 99] {
            for i in 0..1000 {
                let x = i as f64 / 997.0;
                let (p, q) = best_fraction(x, max);
                assert!((1..=max).contains(&q));
                let error = (x - p as f64 / q as f64).abs();
                let expected = (1..=max).map(|d| (x - (x * d as f64).round() / d as f64).abs()).fold(f64::INFINITY, f64::min);
                assert!(error <= expected + 1e-14, "{x}: {p}/{q} bounded by {max}");
            }
        }
    }

    #[test]
    fn numbers() {
        assert_eq!(f(1234.567, "0"), "1235");
        assert_eq!(f(1234.567, "0.00"), "1234.57");
        assert_eq!(f(1234.567, "#,##0"), "1,235");
        assert_eq!(f(1234567.891, "#,##0.00"), "1,234,567.89");
        assert_eq!(f(0.5, "0%"), "50%");
        assert_eq!(f(0.1234, "0.00%"), "12.34%");
        assert_eq!(f(-1234.5, "#,##0.00"), "-1,234.50");
        assert_eq!(f(0.5, "#.##"), ".5");
        assert_eq!(f(12.0, "#.##"), "12.");
        assert_eq!(f(1.5, "0.0#"), "1.5");
        assert_eq!(f(1.567, "0.0#"), "1.57");
        assert_eq!(f(5.0, "000"), "005");
        assert_eq!(f(1234567.0, "#,##0,"), "1,235");
        assert_eq!(f(1234567890.0, "0.0,,"), "1234.6");
        assert_eq!(f(2125551234.0, "(000) 000-0000"), "(212) 555-1234");
    }

    #[test]
    fn sections_colors_and_conditions() {
        assert_eq!(f(-5.0, "#,##0 ;(#,##0)"), "(5)");
        assert_eq!(f(5.0, "#,##0 ;(#,##0)"), "5 ");
        assert_eq!(f(0.0, "0;-0;\"zero\""), "zero");
        let red = format(&Value::Number(-3.0), "#,##0;[Red]-#,##0", DateSystem::From1900);
        assert_eq!(red, Formatted { text: "-3".into(), color: Some("red".into()) });
        assert_eq!(f(150.0, "[>100]\"big\";[<=100]\"small\""), "big");
        assert_eq!(f(50.0, "[>100]\"big\";[<=100]\"small\""), "small");
        assert_eq!(f(1234.5, "\"$\"#,##0.00_);\\(\"$\"#,##0.00\\)"), "$1,234.50 ");
        assert_eq!(f(-1234.5, builtin(7).unwrap()), "($1,234.50)");
        assert_eq!(f(9.99, "[$€-407] #,##0.00"), "€ 9.99");
        assert_eq!(f(-0.001, "0.00"), "0.00", "no minus on what rounds to zero");
    }

    #[test]
    fn scientific_and_fractions() {
        assert_eq!(f(12345.0, "0.00E+00"), "1.23E+04");
        assert_eq!(f(0.000123, "0.00E+00"), "1.23E-04");
        assert_eq!(f(12345.0, "##0.0E+0"), "12.3E+3");
        assert_eq!(f(1.5, "# ?/?"), "1 1/2");
        assert_eq!(f(0.75, "# ?/?"), "3/4");
        assert_eq!(f(2.35, "# ??/??"), "2 7/20");
        assert_eq!(f(0.3, "?/8"), "2/8");
        assert_eq!(f(2.0, "# ?/?"), "2");
    }

    #[test]
    fn general() {
        assert_eq!(f(0.1 + 0.2, "General"), "0.3");
        assert_eq!(f(1234567.0, "General"), "1234567");
        assert_eq!(f(1.0 / 3.0, "General"), "0.333333333");
        assert_eq!(f(123456789012.0, "General"), "1.23457E+11");
        assert_eq!(f(0.0000000001, "General"), "1E-10");
        assert_eq!(f(-42.5, "General"), "-42.5");
    }

    #[test]
    fn dates_and_times() {
        assert_eq!(f(45000.0, "m/d/yyyy"), "3/15/2023");
        assert_eq!(f(45000.0, "d-mmm-yy"), "15-Mar-23");
        assert_eq!(f(45000.0, "dddd, mmmm d, yyyy"), "Wednesday, March 15, 2023");
        assert_eq!(f(45000.0, "mmmmm"), "M");
        assert_eq!(f(0.5, "h:mm AM/PM"), "12:00 PM");
        assert_eq!(f(0.25, "h:mm:ss AM/PM"), "6:00:00 AM");
        assert_eq!(f(45000.75, "yyyy-mm-dd hh:mm"), "2023-03-15 18:00");
        assert_eq!(f(1.5, "[h]:mm:ss"), "36:00:00");
        assert_eq!(f(0.0001, "mm:ss.0"), "00:08.6");
        assert_eq!(f(0.5, "h:mm a/p"), "12:00 p");
        assert!(is_date_format("d-mmm-yy"));
        assert!(is_date_format("[h]:mm"));
        assert!(!is_date_format("0.00"));
        assert!(!is_date_format("\"day\" 0"));
    }

    #[test]
    fn text() {
        let t = |v: &str, code: &str| format(&Value::Text(v.into()), code, DateSystem::From1900).text;
        assert_eq!(t("abc", "@"), "abc");
        assert_eq!(t("abc", "\"Name: \"@"), "Name: abc");
        assert_eq!(t("abc", "0;0;0;\"[\"@\"]\""), "[abc]");
        assert_eq!(t("abc", "0.00"), "abc");
    }

    #[test]
    fn text_format_on_numbers() {
        assert_eq!(f(5.0, "@"), "5");
        assert_eq!(f(-12.5, "@"), "-12.5");
        assert_eq!(f(1234.567, "@"), "1234.567");
        assert_eq!(f(5.0, "\"total: \"@"), "total: 5");
        assert_eq!(f(-5.0, "0;\"neg:\"@"), "neg:5");
    }
}
