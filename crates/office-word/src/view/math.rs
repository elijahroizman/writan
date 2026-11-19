//! Office Math (`m:oMath`) as text the view can draw: a linear form close to the one Word shows
//! when an equation is switched to linear, with accents as combining marks, scripts as Unicode
//! superscripts and subscripts where they exist, and fractions and roots written out.

use office_core::xml::{ns, Element};

/// Whether `el` is an equation, inline (`m:oMath`) or a display block (`m:oMathPara`).
pub fn is_math(el: &Element) -> bool {
    el.is(ns::M, "oMath") || el.is(ns::M, "oMathPara")
}

/// The text an equation shows.
pub fn linear(el: &Element) -> String {
    let mut out = String::new();
    if el.is(ns::M, "oMathPara") {
        let parts: Vec<String> = el.children_named(ns::M, "oMath").map(linear).filter(|s| !s.is_empty()).collect();
        return parts.join("    ");
    }
    push_children(el, &mut out);
    out
}

fn push_children(el: &Element, out: &mut String) {
    for c in el.elements() {
        push(c, out);
    }
}

fn arg(el: &Element, name: &str) -> String {
    let mut s = String::new();
    if let Some(c) = el.child(ns::M, name) {
        push_children(c, &mut s);
    }
    s
}

/// A property's `m:val`: `None` when the property is absent.
fn prop<'a>(el: &'a Element, pr: &str, name: &str) -> Option<&'a str> {
    el.path(ns::M, &[pr, name]).map(|p| p.attr_ns(ns::M, "val").unwrap_or(""))
}

fn on(el: &Element, pr: &str, name: &str) -> bool {
    prop(el, pr, name).is_some_and(|v| !matches!(v, "0" | "false" | "off"))
}

fn push(el: &Element, out: &mut String) {
    if el.in_ns(ns::W) {
        match el.local.as_str() {
            "t" => out.push_str(&el.text()),
            "tab" => out.push(' '),
            "del" | "delText" | "rPr" | "instrText" => {}
            _ => push_children(el, out),
        }
        return;
    }
    if !el.in_ns(ns::M) {
        push_children(el, out);
        return;
    }
    match el.local.as_str() {
        "t" => out.push_str(&el.text()),
        "r" => {
            for c in el.elements() {
                if c.is(ns::M, "t") || c.in_ns(ns::W) {
                    push(c, out);
                }
            }
        }
        "acc" => {
            let base = arg(el, "e");
            let chr = prop(el, "accPr", "chr").and_then(|v| v.chars().next()).unwrap_or('\u{0302}');
            out.push_str(&base);
            out.push(combining(chr));
        }
        "bar" => {
            let mark = if prop(el, "barPr", "pos") == Some("top") { '\u{0305}' } else { '\u{0332}' };
            for c in arg(el, "e").chars() {
                out.push(c);
                if !c.is_whitespace() {
                    out.push(mark);
                }
            }
        }
        "f" => {
            let (num, den) = (arg(el, "num"), arg(el, "den"));
            if prop(el, "fPr", "type") == Some("noBar") {
                out.push_str(&format!("({num} ¦ {den})"));
            } else {
                out.push_str(&format!("{}/{}", group(&num), group(&den)));
            }
        }
        "sSup" => {
            out.push_str(&arg(el, "e"));
            out.push_str(&script(&arg(el, "sup"), true));
        }
        "sSub" => {
            out.push_str(&arg(el, "e"));
            out.push_str(&script(&arg(el, "sub"), false));
        }
        "sSubSup" => {
            out.push_str(&arg(el, "e"));
            out.push_str(&script(&arg(el, "sub"), false));
            out.push_str(&script(&arg(el, "sup"), true));
        }
        "sPre" => {
            out.push_str(&script(&arg(el, "sub"), false));
            out.push_str(&script(&arg(el, "sup"), true));
            out.push_str(&arg(el, "e"));
        }
        "rad" => {
            let deg = if on(el, "radPr", "degHide") { String::new() } else { arg(el, "deg") };
            match deg.trim() {
                "" | "2" => out.push('√'),
                "3" => out.push('∛'),
                "4" => out.push('∜'),
                d => {
                    out.push_str(&script(d, true));
                    out.push('√');
                }
            }
            out.push_str(&group(&arg(el, "e")));
        }
        "d" => {
            let beg = prop(el, "dPr", "begChr").unwrap_or("(");
            let end = prop(el, "dPr", "endChr").unwrap_or(")");
            let sep = prop(el, "dPr", "sepChr").unwrap_or("|");
            out.push_str(beg);
            let parts: Vec<String> = el
                .children_named(ns::M, "e")
                .map(|e| {
                    let mut s = String::new();
                    push_children(e, &mut s);
                    s
                })
                .collect();
            out.push_str(&parts.join(sep));
            out.push_str(end);
        }
        "nary" => {
            out.push_str(prop(el, "naryPr", "chr").filter(|c| !c.is_empty()).unwrap_or("∫"));
            if !on(el, "naryPr", "subHide") {
                out.push_str(&script(&arg(el, "sub"), false));
            }
            if !on(el, "naryPr", "supHide") {
                out.push_str(&script(&arg(el, "sup"), true));
            }
            let e = arg(el, "e");
            if !e.is_empty() {
                out.push('\u{2009}');
                out.push_str(&e);
            }
        }
        "func" => {
            out.push_str(&arg(el, "fName"));
            out.push('\u{2009}');
            out.push_str(&arg(el, "e"));
        }
        "limLow" => {
            out.push_str(&arg(el, "e"));
            out.push_str(&script(&arg(el, "lim"), false));
        }
        "limUpp" => {
            out.push_str(&arg(el, "e"));
            out.push_str(&script(&arg(el, "lim"), true));
        }
        "phant" => {
            if !prop(el, "phantPr", "show").is_some_and(|v| matches!(v, "0" | "false" | "off")) {
                out.push_str(&arg(el, "e"));
            }
        }
        "eqArr" => {
            let rows: Vec<String> = el
                .children_named(ns::M, "e")
                .map(|e| {
                    let mut s = String::new();
                    push_children(e, &mut s);
                    s
                })
                .collect();
            out.push_str(&rows.join("    "));
        }
        "m" => {
            let rows: Vec<String> = el
                .children_named(ns::M, "mr")
                .map(|r| {
                    r.children_named(ns::M, "e")
                        .map(|e| {
                            let mut s = String::new();
                            push_children(e, &mut s);
                            s
                        })
                        .collect::<Vec<_>>()
                        .join("  ")
                })
                .collect();
            out.push_str(&rows.join("; "));
        }
        "oMath" => push_children(el, out),
        // Properties: how a part looks, not what it shows.
        local if local.ends_with("Pr") => {}
        // Groups, boxes and the arguments of everything else show their content.
        _ => push_children(el, out),
    }
}

/// An operand, in parentheses unless it is a single number or name.
fn group(s: &str) -> String {
    if s.chars().count() <= 1 || s.chars().all(|c| c.is_alphanumeric() || c == '.') {
        s.to_string()
    } else {
        format!("({s})")
    }
}

/// A superscript or subscript: Unicode script characters when each one has one, else `^`/`_`.
fn script(s: &str, sup: bool) -> String {
    if s.is_empty() {
        return String::new();
    }
    let mapped: Option<String> = s.chars().map(|c| if sup { superscript(c) } else { subscript(c) }).collect();
    match mapped {
        Some(m) => m,
        None if s.chars().count() == 1 => format!("{}{s}", if sup { '^' } else { '_' }),
        None => format!("{}({s})", if sup { '^' } else { '_' }),
    }
}

fn superscript(c: char) -> Option<char> {
    Some(match c {
        '0' => '⁰',
        '1' => '¹',
        '2' => '²',
        '3' => '³',
        '4'..='9' => char::from_u32(0x2074 + (c as u32 - '4' as u32))?,
        '+' => '⁺',
        '-' | '−' => '⁻',
        '=' => '⁼',
        '(' => '⁽',
        ')' => '⁾',
        'n' => 'ⁿ',
        'i' => 'ⁱ',
        '′' | '″' | '‴' | '*' | '∗' | '∘' | '°' => c,
        _ => return None,
    })
}

fn subscript(c: char) -> Option<char> {
    Some(match c {
        '0'..='9' => char::from_u32(0x2080 + (c as u32 - '0' as u32))?,
        '+' => '₊',
        '-' | '−' => '₋',
        '=' => '₌',
        '(' => '₍',
        ')' => '₎',
        'a' => 'ₐ',
        'e' => 'ₑ',
        'o' => 'ₒ',
        'x' => 'ₓ',
        'h' => 'ₕ',
        'k' => 'ₖ',
        'l' => 'ₗ',
        'm' => 'ₘ',
        'n' => 'ₙ',
        'p' => 'ₚ',
        's' => 'ₛ',
        't' => 'ₜ',
        'i' => 'ᵢ',
        'j' => 'ⱼ',
        'r' => 'ᵣ',
        'u' => 'ᵤ',
        'v' => 'ᵥ',
        _ => return None,
    })
}

/// The combining mark for an accent character: Word files name accents either way.
fn combining(c: char) -> char {
    match c {
        '`' => '\u{0300}',
        '´' => '\u{0301}',
        '^' | 'ˆ' => '\u{0302}',
        '~' | '˜' => '\u{0303}',
        '¯' | 'ˉ' => '\u{0304}',
        '‾' => '\u{0305}',
        '˘' => '\u{0306}',
        '˙' => '\u{0307}',
        '¨' => '\u{0308}',
        '˚' => '\u{030A}',
        'ˇ' => '\u{030C}',
        '→' => '\u{20D7}',
        '←' => '\u{20D6}',
        '↔' => '\u{20E1}',
        c => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn math(body: &str) -> String {
        let xml = format!(r#"<m:oMath xmlns:m="{}" xmlns:w="{}">{body}</m:oMath>"#, ns::M, ns::W);
        linear(&office_core::xml::parse_element(&xml).unwrap())
    }

    fn r(t: &str) -> String {
        format!("<m:r><m:t>{t}</m:t></m:r>")
    }

    #[test]
    fn accents_and_bars_become_combining_marks() {
        let acc = |chr: &str| format!(r#"<m:acc><m:accPr><m:chr m:val="{chr}"/></m:accPr><m:e>{}</m:e></m:acc>"#, r("a"));
        assert_eq!(math(&acc("\u{0301}")), "a\u{0301}");
        assert_eq!(math(&acc("˙")), "a\u{0307}");
        assert_eq!(math(&format!("<m:acc><m:e>{}</m:e></m:acc>", r("x"))), "x\u{0302}");
        assert_eq!(math(&format!("<m:bar><m:e>{}</m:e></m:bar>", r("ab"))), "a\u{0332}b\u{0332}");
        assert_eq!(math(&format!(r#"<m:bar><m:barPr><m:pos m:val="top"/></m:barPr><m:e>{}</m:e></m:bar>"#, r("a"))), "a\u{0305}");
    }

    #[test]
    fn fractions_scripts_roots_and_delimiters() {
        let frac = format!("<m:f><m:num>{}</m:num><m:den>{}</m:den></m:f>", r("a+b"), r("2"));
        assert_eq!(math(&frac), "(a+b)/2");
        let sup = format!("<m:sSup><m:e>{}</m:e><m:sup>{}</m:sup></m:sSup>", r("x"), r("2"));
        assert_eq!(math(&sup), "x²");
        let sub = format!("<m:sSub><m:e>{}</m:e><m:sub>{}</m:sub></m:sSub>", r("x"), r("i+1"));
        assert_eq!(math(&sub), "xᵢ₊₁");
        let odd = format!("<m:sSup><m:e>{}</m:e><m:sup>{}</m:sup></m:sSup>", r("e"), r("πx"));
        assert_eq!(math(&odd), "e^(πx)");
        let rad = format!(r#"<m:rad><m:radPr><m:degHide m:val="1"/></m:radPr><m:deg/><m:e>{}</m:e></m:rad>"#, r("b²-4ac"));
        assert_eq!(math(&rad), "√(b²-4ac)");
        let cube = format!("<m:rad><m:deg>{}</m:deg><m:e>{}</m:e></m:rad>", r("3"), r("x"));
        assert_eq!(math(&cube), "∛x");
        let d =
            format!(r#"<m:d><m:dPr><m:begChr m:val="["/><m:endChr m:val="]"/></m:dPr><m:e>{}</m:e><m:e>{}</m:e></m:d>"#, r("a"), r("b"));
        assert_eq!(math(&d), "[a|b]");
        let sum = format!(
            r#"<m:nary><m:naryPr><m:chr m:val="∑"/></m:naryPr><m:sub>{}</m:sub><m:sup>{}</m:sup><m:e>{}</m:e></m:nary>"#,
            r("i=1"),
            r("n"),
            r("i")
        );
        assert_eq!(math(&sum), "∑ᵢ₌₁ⁿ\u{2009}i");
        let func = format!("<m:func><m:fName>{}</m:fName><m:e>{}</m:e></m:func>", r("sin"), r("x"));
        assert_eq!(math(&func), "sin\u{2009}x");
    }

    #[test]
    fn properties_and_deleted_text_show_nothing() {
        let body = r#"<m:r><w:rPr><w:rFonts w:ascii="Cambria Math"/></w:rPr><m:t>x</m:t></m:r><w:del><m:r><m:t>y</m:t></m:r></w:del><m:ctrlPr><w:rPr/></m:ctrlPr>"#;
        assert_eq!(math(body), "x");
    }
}
