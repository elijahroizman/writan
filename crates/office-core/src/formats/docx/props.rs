//! Word's run and paragraph properties (`w:rPr`, `w:pPr`) and colors.

use crate::drawing::{Color, Rgb};
use crate::model::{Align, Baseline, LineSpacing, ParaProps, RunProps, Strike};
use crate::units::{parse_bool, Emu, FontSize, Percent};
use crate::xml::{ns, Element};

/// Word's `w:val`.
pub fn val(el: &Element) -> Option<&str> {
    el.attr_ns(ns::W, "val")
}

/// An on/off property: `<w:b/>` is on, `<w:b w:val="0"/>` off.
pub fn toggle(parent: &Element, local: &str) -> Option<bool> {
    parent.child(ns::W, local).map(|e| val(e).map(|v| parse_bool(v).unwrap_or(true)).unwrap_or(true))
}

/// Word's theme font names, and DrawingML's references for the same fonts (the model's).
pub const THEME_FONTS: &[(&str, &str)] = &[
    ("minorHAnsi", "+mn-lt"),
    ("minorAscii", "+mn-lt"),
    ("majorHAnsi", "+mj-lt"),
    ("majorAscii", "+mj-lt"),
    ("minorEastAsia", "+mn-ea"),
    ("majorEastAsia", "+mj-ea"),
    ("minorBidi", "+mn-cs"),
    ("majorBidi", "+mj-cs"),
];

fn theme_font(word: &str) -> Option<&'static str> {
    THEME_FONTS.iter().find(|(w, _)| *w == word).map(|(_, d)| *d)
}

/// Word's theme color names, and the scheme slots they are.
pub const THEME_COLORS: &[(&str, &str)] = &[
    ("dark1", "dk1"),
    ("light1", "lt1"),
    ("dark2", "dk2"),
    ("light2", "lt2"),
    ("accent1", "accent1"),
    ("accent2", "accent2"),
    ("accent3", "accent3"),
    ("accent4", "accent4"),
    ("accent5", "accent5"),
    ("accent6", "accent6"),
    ("hyperlink", "hlink"),
    ("followedHyperlink", "folHlink"),
    ("text1", "tx1"),
    ("background1", "bg1"),
    ("text2", "tx2"),
    ("background2", "bg2"),
];

/// `w:color`: a theme color with Word's tint or shade (bytes, 255 is none) becomes the
/// DrawingML transforms Word itself uses for them; `auto` is no color.
pub fn read_color(el: &Element) -> Option<Color> {
    if let Some(slot) = el.attr_ns(ns::W, "themeColor").and_then(|t| THEME_COLORS.iter().find(|(w, _)| *w == t)).map(|(_, s)| *s) {
        let byte = |k: &str| {
            el.attr_ns(ns::W, k).and_then(|v| u8::from_str_radix(v, 16).ok()).map(|b| (b as i64 * Percent::WHOLE as i64 + 127) / 255)
        };
        let mut c = Color::scheme(slot);
        if let Some(t) = byte("themeTint") {
            c = c.with("lumMod", t as i32).with("lumOff", (Percent::WHOLE as i64 - t) as i32);
        } else if let Some(s) = byte("themeShade") {
            c = c.with("lumMod", s as i32);
        }
        return Some(c);
    }
    val(el).and_then(Rgb::parse).map(Color::rgb)
}

/// Word's highlight colors, which are names, not values.
pub const HIGHLIGHTS: &[(&str, Rgb)] = &[
    ("black", Rgb(0, 0, 0)),
    ("blue", Rgb(0, 0, 255)),
    ("cyan", Rgb(0, 255, 255)),
    ("green", Rgb(0, 255, 0)),
    ("magenta", Rgb(255, 0, 255)),
    ("red", Rgb(255, 0, 0)),
    ("yellow", Rgb(255, 255, 0)),
    ("white", Rgb(255, 255, 255)),
    ("darkBlue", Rgb(0, 0, 128)),
    ("darkCyan", Rgb(0, 128, 128)),
    ("darkGreen", Rgb(0, 128, 0)),
    ("darkMagenta", Rgb(128, 0, 128)),
    ("darkRed", Rgb(128, 0, 0)),
    ("darkYellow", Rgb(128, 128, 0)),
    ("darkGray", Rgb(128, 128, 128)),
    ("lightGray", Rgb(192, 192, 192)),
];

pub fn read_run_props(rpr: &Element) -> RunProps {
    let fonts = rpr.child(ns::W, "rFonts");
    let font = |direct: &str, theme: &str| {
        let f = fonts?;
        f.attr_ns(ns::W, theme).and_then(theme_font).map(str::to_string).or_else(|| f.attr_ns(ns::W, direct).map(str::to_string))
    };
    let strike = match (toggle(rpr, "dstrike"), toggle(rpr, "strike")) {
        (Some(true), _) => Some(Strike::Double),
        (_, Some(true)) => Some(Strike::Single),
        (Some(false), _) | (_, Some(false)) => Some(Strike::None),
        _ => None,
    };
    let size = rpr.child(ns::W, "sz").and_then(val).and_then(|v| v.parse().ok()).map(FontSize::from_half_points);
    let (bold, italic) = (toggle(rpr, "b"), toggle(rpr, "i"));
    RunProps {
        font: font("ascii", "asciiTheme"),
        font_east_asian: font("eastAsia", "eastAsiaTheme"),
        font_complex: font("cs", "cstheme"),
        size,
        bold,
        italic,
        // Only where they differ from the others: Word sets both alike.
        size_complex: rpr
            .child(ns::W, "szCs")
            .and_then(val)
            .and_then(|v| v.parse().ok())
            .map(FontSize::from_half_points)
            .filter(|s| Some(*s) != size),
        bold_complex: toggle(rpr, "bCs").filter(|b| Some(*b) != bold),
        italic_complex: toggle(rpr, "iCs").filter(|i| Some(*i) != italic),
        rtl: toggle(rpr, "rtl"),
        underline: rpr.child(ns::W, "u").map(|u| val(u).unwrap_or("single").to_string()),
        strike,
        caps: toggle(rpr, "caps"),
        small_caps: toggle(rpr, "smallCaps"),
        color: rpr.child(ns::W, "color").and_then(read_color),
        highlight: rpr
            .child(ns::W, "highlight")
            .and_then(val)
            .and_then(|h| HIGHLIGHTS.iter().find(|(n, _)| *n == h))
            .map(|(_, rgb)| Color::rgb(*rgb)),
        baseline: rpr.child(ns::W, "vertAlign").and_then(val).and_then(|v| match v {
            "superscript" => Some(Baseline::Superscript),
            "subscript" => Some(Baseline::Subscript),
            _ => None,
        }),
        style: rpr.child(ns::W, "rStyle").and_then(val).map(str::to_string),
        lang: rpr.child(ns::W, "lang").and_then(val).map(str::to_string),
    }
}

pub fn read_para_props(ppr: &Element) -> ParaProps {
    let twips = |el: Option<&Element>, k: &str| el.and_then(|e| e.attr_ns(ns::W, k)).and_then(|v| Emu::parse(v, Emu::PER_TWIP));
    let ind = ppr.child(ns::W, "ind");
    let spacing = ppr.child(ns::W, "spacing");
    let auto = |k: &str| spacing.and_then(|s| s.attr_ns(ns::W, k)).and_then(parse_bool).unwrap_or(false);
    let first_line = twips(ind, "firstLine").or_else(|| twips(ind, "hanging").map(|h| Emu(-h.0)));
    ParaProps {
        style: ppr.child(ns::W, "pStyle").and_then(val).map(str::to_string),
        align: ppr.child(ns::W, "jc").and_then(val).and_then(|j| {
            Some(match j {
                "left" | "start" => Align::Left,
                "center" => Align::Center,
                "right" | "end" => Align::Right,
                "both" | "lowKashida" | "mediumKashida" | "highKashida" => Align::Justify,
                "distribute" | "thaiDistribute" => Align::Distribute,
                _ => return None,
            })
        }),
        level: ppr.path(ns::W, &["numPr", "ilvl"]).and_then(val).and_then(|v| v.parse().ok()),
        indent_start: twips(ind, "start").or_else(|| twips(ind, "left")),
        indent_end: twips(ind, "end").or_else(|| twips(ind, "right")),
        indent_first_line: first_line,
        space_before: if auto("beforeAutospacing") { None } else { twips(spacing, "before") },
        space_after: if auto("afterAutospacing") { None } else { twips(spacing, "after") },
        line_spacing: spacing.and_then(|s| {
            let line: i64 = s.attr_ns(ns::W, "line")?.parse().ok()?;
            Some(match s.attr_ns(ns::W, "lineRule").unwrap_or("auto") {
                "exact" => LineSpacing::Exactly(Emu::from_twips(line)),
                "atLeast" => LineSpacing::AtLeast(Emu::from_twips(line)),
                // 240ths of a line.
                _ => LineSpacing::Multiple(Percent((line * Percent::WHOLE as i64 / 240) as i32)),
            })
        }),
        rtl: toggle(ppr, "bidi"),
    }
}

// ---- writing ----

/// The children of `w:rPr`, in schema order.
const RPR: &[&[&str]] = &[
    &["rStyle"],
    &["rFonts"],
    &["b"],
    &["bCs"],
    &["i"],
    &["iCs"],
    &["caps"],
    &["smallCaps"],
    &["strike"],
    &["dstrike"],
    &["outline"],
    &["shadow"],
    &["emboss"],
    &["imprint"],
    &["noProof"],
    &["snapToGrid"],
    &["vanish"],
    &["webHidden"],
    &["color"],
    &["spacing"],
    &["w"],
    &["kern"],
    &["position"],
    &["sz"],
    &["szCs"],
    &["highlight"],
    &["u"],
    &["effect"],
    &["bdr"],
    &["shd"],
    &["fitText"],
    &["vertAlign"],
    &["rtl"],
    &["cs"],
    &["em"],
    &["lang"],
    &["eastAsianLayout"],
    &["specVanish"],
    &["oMath"],
];

/// The children of `w:pPr`, in schema order.
pub const PPR: &[&[&str]] = &[
    &["pStyle"],
    &["keepNext"],
    &["keepLines"],
    &["pageBreakBefore"],
    &["framePr"],
    &["widowControl"],
    &["numPr"],
    &["suppressLineNumbers"],
    &["pBdr"],
    &["shd"],
    &["tabs"],
    &["suppressAutoHyphens"],
    &["kinsoku"],
    &["wordWrap"],
    &["overflowPunct"],
    &["topLinePunct"],
    &["autoSpaceDE"],
    &["autoSpaceDN"],
    &["bidi"],
    &["adjustRightInd"],
    &["snapToGrid"],
    &["spacing"],
    &["ind"],
    &["contextualSpacing"],
    &["mirrorIndents"],
    &["suppressOverlap"],
    &["jc"],
    &["textDirection"],
    &["textAlignment"],
    &["textboxTightWrap"],
    &["outlineLvl"],
    &["divId"],
    &["cnfStyle"],
    &["rPr"],
    &["sectPr"],
    &["pPrChange"],
];

fn w_val(local: &str, v: impl Into<String>) -> Element {
    Element::new(ns::W, local).with_attr(ns::W, "val", v)
}

/// Sets a `w:` attribute, or removes it for `None`.
fn set_w(el: &mut Element, local: &str, v: Option<String>) {
    match v {
        Some(v) => el.set_attr(ns::W, local, v),
        None => {
            el.remove_attr(ns::W, local);
        }
    }
}

fn set_toggle(rpr: &mut Element, local: &str, v: Option<bool>) {
    match v {
        Some(true) => rpr.place(Element::new(ns::W, local), RPR),
        Some(false) => rpr.place(w_val(local, "0"), RPR),
        None => rpr.clear(ns::W, &[local]),
    }
}

/// `w:color` for a color: theme colors keep their slot (and Word's tint or shade when the
/// transforms are ones Word can say), with the resolved value alongside, as Word writes it.
pub fn write_color(c: &Color, theme: &crate::drawing::Theme) -> Element {
    use crate::drawing::ColorBase;
    let rgb = c.resolve(&|n| theme.scheme_color(n)).rgb;
    let mut el = w_val("color", rgb.hex());
    if let ColorBase::Scheme { name } = &c.base {
        if let Some((word, _)) = THEME_COLORS.iter().find(|(_, slot)| slot == name) {
            el.set_attr(ns::W, "themeColor", *word);
            let byte = |v: i32| format!("{:02X}", ((v as f64 / Percent::WHOLE as f64) * 255.0).round().clamp(0.0, 255.0) as u8);
            match c.transforms.as_slice() {
                [m, o] if m.kind == "lumMod" && o.kind == "lumOff" => el.set_attr(ns::W, "themeTint", byte(m.value)),
                [m] if m.kind == "lumMod" => el.set_attr(ns::W, "themeShade", byte(m.value)),
                _ => {}
            }
        }
    }
    el
}

/// Updates `w:rPr` to say `props`, touching only what changed.
pub fn patch_run_props(props: &RunProps, rpr: &mut Element, theme: &crate::drawing::Theme) {
    let old = read_run_props(rpr);
    if props.style != old.style {
        match &props.style {
            Some(s) => rpr.place(w_val("rStyle", s), RPR),
            None => rpr.clear(ns::W, &["rStyle"]),
        }
    }
    if (&props.font, &props.font_east_asian, &props.font_complex) != (&old.font, &old.font_east_asian, &old.font_complex) {
        let mut fonts = rpr.child(ns::W, "rFonts").cloned().unwrap_or_else(|| Element::new(ns::W, "rFonts"));
        let mut set = |value: &Option<String>, before: &Option<String>, direct: &[&str], themed: &[&str]| {
            if value == before {
                return;
            }
            for k in direct.iter().chain(themed) {
                fonts.remove_attr(ns::W, k);
            }
            let Some(f) = value else { return };
            let theme_name = THEME_FONTS.iter().filter(|(_, d)| d == f).map(|(w, _)| *w);
            match theme_name.clone().next() {
                // Latin text has two attributes, ASCII and the rest; Word sets both.
                Some(_) => {
                    let names: Vec<&str> = theme_name.collect();
                    for (k, name) in themed.iter().zip(names.iter().chain(names.iter())) {
                        fonts.set_attr(ns::W, k, *name);
                    }
                }
                None => {
                    for k in direct {
                        fonts.set_attr(ns::W, k, f);
                    }
                }
            }
        };
        set(&props.font, &old.font, &["ascii", "hAnsi"], &["asciiTheme", "hAnsiTheme"]);
        set(&props.font_east_asian, &old.font_east_asian, &["eastAsia"], &["eastAsiaTheme"]);
        set(&props.font_complex, &old.font_complex, &["cs"], &["cstheme"]);
        if fonts.attrs.is_empty() {
            rpr.clear(ns::W, &["rFonts"]);
        } else {
            rpr.place(fonts, RPR);
        }
    }
    // Complex-script bold, italic and size are the others' unless set apart, and written
    // alongside them, as Word does.
    if props.bold != old.bold {
        set_toggle(rpr, "b", props.bold);
    }
    if props.bold_complex.or(props.bold) != old.bold_complex.or(old.bold) {
        set_toggle(rpr, "bCs", props.bold_complex.or(props.bold));
    }
    if props.italic != old.italic {
        set_toggle(rpr, "i", props.italic);
    }
    if props.italic_complex.or(props.italic) != old.italic_complex.or(old.italic) {
        set_toggle(rpr, "iCs", props.italic_complex.or(props.italic));
    }
    if props.rtl != old.rtl {
        set_toggle(rpr, "rtl", props.rtl);
    }
    if props.caps != old.caps {
        set_toggle(rpr, "caps", props.caps);
    }
    if props.small_caps != old.small_caps {
        set_toggle(rpr, "smallCaps", props.small_caps);
    }
    if props.strike != old.strike {
        let (single, double) = match props.strike {
            Some(Strike::Single) => (Some(true), None),
            Some(Strike::Double) => (None, Some(true)),
            Some(Strike::None) => (Some(false), None),
            None => (None, None),
        };
        set_toggle(rpr, "strike", single);
        set_toggle(rpr, "dstrike", double);
    }
    if props.color != old.color {
        match &props.color {
            Some(c) => rpr.place(write_color(c, theme), RPR),
            None => rpr.clear(ns::W, &["color"]),
        }
    }
    if props.size != old.size {
        match props.size {
            Some(s) => rpr.place(w_val("sz", s.half_points().to_string()), RPR),
            None => rpr.clear(ns::W, &["sz"]),
        }
    }
    if props.size_complex.or(props.size) != old.size_complex.or(old.size) {
        match props.size_complex.or(props.size) {
            Some(s) => rpr.place(w_val("szCs", s.half_points().to_string()), RPR),
            None => rpr.clear(ns::W, &["szCs"]),
        }
    }
    if props.highlight != old.highlight {
        rpr.clear(ns::W, &["highlight"]);
        if let Some(c) = &props.highlight {
            // Word's highlights are a fixed set of names; other colors are shading.
            let rgb = c.resolve(&|n| theme.scheme_color(n)).rgb;
            match HIGHLIGHTS.iter().find(|(_, h)| *h == rgb) {
                Some((name, _)) => rpr.place(w_val("highlight", *name), RPR),
                None => rpr.place(
                    Element::new(ns::W, "shd").with_attr(ns::W, "val", "clear").with_attr(ns::W, "color", "auto").with_attr(
                        ns::W,
                        "fill",
                        rgb.hex(),
                    ),
                    RPR,
                ),
            }
        }
    }
    if props.underline != old.underline {
        match &props.underline {
            Some(u) => rpr.place(w_val("u", u), RPR),
            None => rpr.clear(ns::W, &["u"]),
        }
    }
    if props.baseline != old.baseline {
        match props.baseline {
            Some(Baseline::Superscript) => rpr.place(w_val("vertAlign", "superscript"), RPR),
            Some(Baseline::Subscript) => rpr.place(w_val("vertAlign", "subscript"), RPR),
            None => rpr.clear(ns::W, &["vertAlign"]),
        }
    }
    if props.lang != old.lang {
        match &props.lang {
            Some(l) => {
                let mut lang = rpr.child(ns::W, "lang").cloned().unwrap_or_else(|| Element::new(ns::W, "lang"));
                lang.set_attr(ns::W, "val", l);
                rpr.place(lang, RPR);
            }
            None => rpr.clear(ns::W, &["lang"]),
        }
    }
}

/// Updates `w:pPr` to say `props`, touching only what changed. The paragraph mark's `w:rPr`
/// is the caller's.
pub fn patch_para_props(props: &ParaProps, ppr: &mut Element) {
    let old = read_para_props(ppr);
    let twips = |e: Option<Emu>| e.map(|e| e.twips().to_string());
    if props.style != old.style {
        match &props.style {
            Some(s) => ppr.place(w_val("pStyle", s), PPR),
            None => ppr.clear(ns::W, &["pStyle"]),
        }
    }
    if props.level != old.level {
        match props.level {
            Some(l) => {
                let mut num = ppr.child(ns::W, "numPr").cloned().unwrap_or_else(|| Element::new(ns::W, "numPr"));
                num.place(w_val("ilvl", l.to_string()), &[&["ilvl"], &["numId"]]);
                ppr.place(num, PPR);
            }
            None => {
                if let Some(num) = ppr.child_mut(ns::W, "numPr") {
                    num.remove_children(ns::W, "ilvl");
                }
            }
        }
    }
    if props.rtl != old.rtl {
        match props.rtl {
            Some(true) => ppr.place(Element::new(ns::W, "bidi"), PPR),
            Some(false) => ppr.place(w_val("bidi", "0"), PPR),
            None => ppr.clear(ns::W, &["bidi"]),
        }
    }
    if (props.space_before, props.space_after, props.line_spacing) != (old.space_before, old.space_after, old.line_spacing) {
        let mut sp = ppr.child(ns::W, "spacing").cloned().unwrap_or_else(|| Element::new(ns::W, "spacing"));
        if props.space_before != old.space_before {
            sp.remove_attr(ns::W, "beforeAutospacing");
            set_w(&mut sp, "before", twips(props.space_before));
        }
        if props.space_after != old.space_after {
            sp.remove_attr(ns::W, "afterAutospacing");
            set_w(&mut sp, "after", twips(props.space_after));
        }
        if props.line_spacing != old.line_spacing {
            let (line, rule) = match props.line_spacing {
                Some(LineSpacing::Multiple(p)) => (Some((p.0 as i64 * 240 / Percent::WHOLE as i64).to_string()), Some("auto")),
                Some(LineSpacing::Exactly(e)) => (Some(e.twips().to_string()), Some("exact")),
                Some(LineSpacing::AtLeast(e)) => (Some(e.twips().to_string()), Some("atLeast")),
                None => (None, None),
            };
            set_w(&mut sp, "line", line);
            set_w(&mut sp, "lineRule", rule.map(str::to_string));
        }
        if sp.attrs.is_empty() {
            ppr.clear(ns::W, &["spacing"]);
        } else {
            ppr.place(sp, PPR);
        }
    }
    if (props.indent_start, props.indent_end, props.indent_first_line) != (old.indent_start, old.indent_end, old.indent_first_line) {
        let mut ind = ppr.child(ns::W, "ind").cloned().unwrap_or_else(|| Element::new(ns::W, "ind"));
        if props.indent_start != old.indent_start {
            ind.remove_attr(ns::W, "start");
            set_w(&mut ind, "left", twips(props.indent_start));
        }
        if props.indent_end != old.indent_end {
            ind.remove_attr(ns::W, "end");
            set_w(&mut ind, "right", twips(props.indent_end));
        }
        if props.indent_first_line != old.indent_first_line {
            ind.remove_attr(ns::W, "firstLine");
            ind.remove_attr(ns::W, "hanging");
            match props.indent_first_line {
                Some(e) if e.0 < 0 => ind.set_attr(ns::W, "hanging", Emu(-e.0).twips().to_string()),
                Some(e) => ind.set_attr(ns::W, "firstLine", e.twips().to_string()),
                None => {}
            }
        }
        if ind.attrs.is_empty() {
            ppr.clear(ns::W, &["ind"]);
        } else {
            ppr.place(ind, PPR);
        }
    }
    if props.align != old.align {
        let jc = props.align.map(|a| match a {
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
            Align::Justify => "both",
            Align::Distribute => "distribute",
        });
        match jc {
            Some(j) => ppr.place(w_val("jc", j), PPR),
            None => ppr.clear(ns::W, &["jc"]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml::parse_element;

    fn el(inner: &str, local: &str) -> Element {
        parse_element(&format!(r#"<w:{local} xmlns:w="{}">{inner}</w:{local}>"#, ns::W)).unwrap()
    }

    #[test]
    fn run_properties() {
        let rpr = el(
            r#"<w:rStyle w:val="Strong"/><w:rFonts w:asciiTheme="minorHAnsi" w:eastAsia="MS Mincho"/><w:b/><w:i w:val="0"/><w:dstrike/><w:color w:val="2F5597" w:themeColor="accent1" w:themeShade="BF"/><w:sz w:val="23"/><w:szCs w:val="23"/><w:highlight w:val="yellow"/><w:u w:val="double"/><w:vertAlign w:val="subscript"/><w:lang w:val="en-GB"/>"#,
            "rPr",
        );
        let p = read_run_props(&rpr);
        assert_eq!(p.style.as_deref(), Some("Strong"));
        assert_eq!(p.font.as_deref(), Some("+mn-lt"));
        assert_eq!(p.font_east_asian.as_deref(), Some("MS Mincho"));
        assert_eq!((p.bold, p.italic, p.strike), (Some(true), Some(false), Some(Strike::Double)));
        assert_eq!(p.size, Some(FontSize(1150)));
        assert_eq!(p.underline.as_deref(), Some("double"));
        assert_eq!(p.baseline, Some(Baseline::Subscript));
        assert_eq!(p.highlight, Some(Color::rgb(Rgb(255, 255, 0))));
        // Shade BF (191/255, a hair under 75%) is Word's "Darker 25%": Word itself writes 2F5496 for it.
        let c = p.color.unwrap();
        assert_eq!(c.resolve(&|n| (n == "accent1").then_some(Rgb(0x44, 0x72, 0xC4))).rgb, Rgb(0x2F, 0x54, 0x96));
    }

    #[test]
    fn paragraph_properties() {
        let ppr = el(
            r#"<w:pStyle w:val="Heading1"/><w:numPr><w:ilvl w:val="2"/><w:numId w:val="5"/></w:numPr><w:spacing w:before="240" w:after="120" w:line="360" w:lineRule="auto"/><w:ind w:left="720" w:hanging="360"/><w:jc w:val="both"/>"#,
            "pPr",
        );
        let p = read_para_props(&ppr);
        assert_eq!(p.style.as_deref(), Some("Heading1"));
        assert_eq!(p.level, Some(2));
        assert_eq!(p.space_before, Some(Emu::from_twips(240)));
        assert_eq!(p.line_spacing, Some(LineSpacing::Multiple(Percent(150_000))));
        assert_eq!(p.indent_start, Some(Emu::from_twips(720)));
        assert_eq!(p.indent_first_line, Some(Emu::from_twips(-360)));
        assert_eq!(p.align, Some(Align::Justify));
    }
}

#[cfg(test)]
mod write_tests {
    use super::*;
    use crate::drawing::Theme;
    use crate::xml::parse_element;

    #[test]
    fn patches_round_trip() {
        let theme = Theme::office();
        let props = RunProps {
            font: Some("+mn-lt".into()),
            font_east_asian: Some("MS Mincho".into()),
            size: Some(FontSize(1150)),
            bold: Some(true),
            italic: Some(false),
            strike: Some(Strike::Double),
            underline: Some("wave".into()),
            color: Some(Color::scheme("accent1").with("lumMod", 74_902)),
            highlight: Some(Color::rgb(Rgb(255, 255, 0))),
            baseline: Some(Baseline::Superscript),
            style: Some("Strong".into()),
            lang: Some("en-GB".into()),
            ..Default::default()
        };
        let mut rpr = Element::new(ns::W, "rPr");
        patch_run_props(&props, &mut rpr, &theme);
        assert_eq!(read_run_props(&rpr), props);
        let names: Vec<&str> = rpr.elements().map(|e| e.local.as_str()).collect();
        assert_eq!(
            names,
            ["rStyle", "rFonts", "b", "bCs", "i", "iCs", "dstrike", "color", "sz", "szCs", "highlight", "u", "vertAlign", "lang"]
        );
        assert_eq!(rpr.child(ns::W, "color").unwrap().attr_ns(ns::W, "themeShade"), Some("BF"));

        // Set apart, they stay apart; a right-to-left run says so.
        let hebrew = RunProps {
            size: Some(FontSize::from_half_points(22)),
            size_complex: Some(FontSize::from_half_points(28)),
            bold: Some(false),
            bold_complex: Some(true),
            italic_complex: Some(true),
            font_complex: Some("David".into()),
            rtl: Some(true),
            ..Default::default()
        };
        let mut rpr = Element::new(ns::W, "rPr");
        patch_run_props(&hebrew, &mut rpr, &theme);
        assert_eq!(read_run_props(&rpr), hebrew);
        let names: Vec<&str> = rpr.elements().map(|e| e.local.as_str()).collect();
        assert_eq!(names, ["rFonts", "b", "bCs", "iCs", "sz", "szCs", "rtl"]);

        let para = ParaProps {
            style: Some("Heading1".into()),
            align: Some(Align::Center),
            level: Some(1),
            indent_start: Some(Emu::from_twips(720)),
            indent_first_line: Some(Emu::from_twips(-360)),
            space_after: Some(Emu::from_twips(120)),
            line_spacing: Some(LineSpacing::Multiple(Percent(150_000))),
            ..Default::default()
        };
        let mut ppr = Element::new(ns::W, "pPr");
        patch_para_props(&para, &mut ppr);
        assert_eq!(read_para_props(&ppr), para);
    }

    #[test]
    fn patching_keeps_what_it_doesnt_model() {
        let mut rpr =
            parse_element(&format!(r#"<w:rPr xmlns:w="{}"><w:noProof/><w:sz w:val="20"/><w:szCs w:val="18"/></w:rPr>"#, ns::W)).unwrap();
        let mut props = read_run_props(&rpr);
        props.bold = Some(true);
        patch_run_props(&props, &mut rpr, &crate::drawing::Theme::office());
        let names: Vec<&str> = rpr.elements().map(|e| e.local.as_str()).collect();
        assert_eq!(names, ["b", "bCs", "noProof", "sz", "szCs"]);
        assert_eq!(rpr.child(ns::W, "szCs").and_then(val), Some("18"), "size didn't change, so szCs is as it was");
    }
}
