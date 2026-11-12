//! DrawingML text (`a:p`, `a:r`, `a:rPr`, `a:pPr`): the text in PowerPoint's shapes and table
//! cells, and in Word's and Excel's drawings.

use super::common::{origin_as, rel_attr, shell, PartCtx, PartRels};
use crate::drawing::Color;
use crate::model::{
    underline_from_drawingml, underline_to_drawingml, Align, Baseline, Inline, LineSpacing, Link, ParaProps, Paragraph, Run, RunProps,
    Strike, TextBody,
};
use crate::units::{parse_bool, Emu, FontSize, Percent};
use crate::xml::{ns, Element};

/// Reads `p:txBody`/`a:txBody`. The body's `a:bodyPr` and `a:lstStyle` stay in its origin.
pub fn read_text_body(el: &Element, rels: Option<&PartRels>) -> TextBody {
    TextBody {
        paragraphs: el.children_named(ns::A, "p").map(|p| read_paragraph(p, rels)).collect(),
        origin: shell(el, |c| c.is(ns::A, "p")),
    }
}

pub fn read_paragraph(p: &Element, rels: Option<&PartRels>) -> Paragraph {
    let mut content = Vec::new();
    for child in p.elements() {
        if !child.in_ns(ns::A) {
            content.push(Inline::Raw { xml: child.clone() });
            continue;
        }
        match child.local.as_str() {
            "pPr" | "endParaRPr" => {}
            "r" | "br" => {
                let rpr = child.child(ns::A, "rPr");
                let text =
                    if child.local == "br" { "\n".to_string() } else { child.child(ns::A, "t").map(Element::text).unwrap_or_default() };
                content.push(Inline::Text(Run {
                    text,
                    props: rpr.map(read_run_props).unwrap_or_default(),
                    link: rpr.and_then(|r| read_link(r, rels)),
                    // The whole run, so an untouched one can be written back as it was.
                    origin: crate::model::Origin::new(child.clone()),
                }));
            }
            // Fields (slide numbers, dates) and anything else, whole.
            _ => content.push(Inline::Raw { xml: child.clone() }),
        }
    }
    Paragraph {
        props: p.child(ns::A, "pPr").map(read_para_props).unwrap_or_default(),
        content,
        mark: p.child(ns::A, "endParaRPr").map(read_run_props),
        origin: shell(p, |c| !(c.is(ns::A, "pPr") || c.is(ns::A, "endParaRPr"))),
    }
}

/// `a:rPr`, `a:endParaRPr` or `a:defRPr`.
pub fn read_run_props(el: &Element) -> RunProps {
    let face = |local: &str| el.child(ns::A, local).and_then(|f| f.attr("typeface")).filter(|f| !f.is_empty()).map(str::to_string);
    let (mut caps, mut small_caps) = (None, None);
    match el.attr("cap") {
        Some("all") => caps = Some(true),
        Some("small") => small_caps = Some(true),
        Some("none") => {
            caps = Some(false);
            small_caps = Some(false);
        }
        _ => {}
    }
    RunProps {
        font: face("latin"),
        font_east_asian: face("ea"),
        font_complex: face("cs"),
        size: el.attr("sz").and_then(|s| s.parse().ok()).map(FontSize),
        bold: el.attr("b").and_then(parse_bool),
        italic: el.attr("i").and_then(parse_bool),
        underline: el.attr("u").map(|u| underline_from_drawingml(u).to_string()),
        strike: el.attr("strike").map(|s| match s {
            "sngStrike" => Strike::Single,
            "dblStrike" => Strike::Double,
            _ => Strike::None,
        }),
        caps,
        small_caps,
        color: el.child(ns::A, "solidFill").and_then(Color::in_drawingml),
        highlight: el.child(ns::A, "highlight").and_then(Color::in_drawingml),
        baseline: el.attr("baseline").and_then(Percent::parse).and_then(|b| match b.0.signum() {
            1 => Some(Baseline::Superscript),
            -1 => Some(Baseline::Subscript),
            _ => None,
        }),
        style: None,
        lang: el.attr("lang").map(str::to_string),
        ..Default::default()
    }
}

/// A run's `a:hlinkClick` to a URL. Clicks that jump to a slide or run an action stay in the
/// run's origin for the PowerPoint layer.
pub(crate) fn read_link(rpr: &Element, rels: Option<&PartRels>) -> Option<Link> {
    let click = rpr.child(ns::A, "hlinkClick")?;
    if click.attr("action").is_some_and(|a| !a.is_empty() && !a.starts_with("ppaction://hlinkfile")) {
        return None;
    }
    let url = rels.and_then(|r| r.url(rel_attr(click, "id")?))?;
    Some(Link { url: Some(url), anchor: None, tooltip: click.attr("tooltip").map(str::to_string) })
}

pub fn read_para_props(el: &Element) -> ParaProps {
    let emu = |k: &str| el.attr(k).and_then(|v| Emu::parse(v, 1));
    let spacing = |local: &str| {
        el.child(ns::A, local).and_then(|s| s.child(ns::A, "spcPts")).and_then(|p| p.attr("val")).and_then(|v| v.parse::<f64>().ok())
    };
    ParaProps {
        style: None,
        align: el.attr("algn").and_then(|a| {
            Some(match a {
                "l" => Align::Left,
                "ctr" => Align::Center,
                "r" => Align::Right,
                "just" | "justLow" => Align::Justify,
                "dist" | "thaiDist" => Align::Distribute,
                _ => return None,
            })
        }),
        level: el.attr("lvl").and_then(|l| l.parse().ok()),
        indent_start: emu("marL"),
        indent_end: emu("marR"),
        indent_first_line: emu("indent"),
        space_before: spacing("spcBef").map(|p| Emu::from_points(p / 100.0)),
        space_after: spacing("spcAft").map(|p| Emu::from_points(p / 100.0)),
        line_spacing: el.child(ns::A, "lnSpc").and_then(|l| {
            if let Some(pct) = l.child(ns::A, "spcPct").and_then(|p| p.attr("val")).and_then(Percent::parse) {
                Some(LineSpacing::Multiple(pct))
            } else {
                let pts = l.child(ns::A, "spcPts")?.attr("val")?.parse::<f64>().ok()?;
                Some(LineSpacing::Exactly(Emu::from_points(pts / 100.0)))
            }
        }),
        rtl: el.attr("rtl").and_then(parse_bool),
    }
}

// ---- writing ----

/// The children of `a:rPr`, in schema order.
const RPR: &[&[&str]] = &[
    &["ln"],
    &["noFill", "solidFill", "gradFill", "blipFill", "pattFill", "grpFill"],
    &["effectLst", "effectDag"],
    &["highlight"],
    &["uLnTx", "uLn"],
    &["uFillTx", "uFill"],
    &["latin"],
    &["ea"],
    &["cs"],
    &["sym"],
    &["hlinkClick"],
    &["hlinkMouseOver"],
    &["rtl"],
    &["extLst"],
];

/// The children of `a:pPr`, in schema order.
const PPR: &[&[&str]] = &[
    &["lnSpc"],
    &["spcBef"],
    &["spcAft"],
    &["buClrTx", "buClr"],
    &["buSzTx", "buSzPct", "buSzPts"],
    &["buFontTx", "buFont"],
    &["buNone", "buAutoNum", "buChar", "buBlip"],
    &["tabLst"],
    &["defRPr"],
    &["extLst"],
];

/// Sets an attribute, or removes it for `None`.
pub(crate) fn set_opt(el: &mut Element, local: &str, value: Option<String>) {
    match value {
        Some(v) => el.set_attr("", local, v),
        None => {
            el.remove_attr("", local);
        }
    }
}

/// Writes `text` (a `p:txBody` or `a:txBody`, by `ns`) from its origin or from scratch.
pub fn write_text_body(body: &TextBody, ns_: &str, ctx: &mut PartCtx) -> Element {
    let mut el = origin_as(&body.origin, ns_, "txBody").unwrap_or_else(|| {
        Element::new(ns_, "txBody").with_child(Element::new(ns::A, "bodyPr")).with_child(Element::new(ns::A, "lstStyle"))
    });
    for p in &body.paragraphs {
        el.push(write_paragraph(p, ctx));
    }
    // A text body must have a paragraph.
    if body.paragraphs.is_empty() {
        el.push(Element::new(ns::A, "p"));
    }
    el
}

pub fn write_paragraph(p: &Paragraph, ctx: &mut PartCtx) -> Element {
    let mut el = origin_as(&p.origin, ns::A, "p").unwrap_or_else(|| Element::new(ns::A, "p"));
    let mut ppr = el.child(ns::A, "pPr").cloned();
    let mut end = el.child(ns::A, "endParaRPr").cloned();
    el.children.clear();

    if !p.props.is_empty() || ppr.is_some() {
        let mut e = ppr.take().unwrap_or_else(|| Element::new(ns::A, "pPr"));
        patch_para_props(&p.props, &mut e);
        el.push(e);
    }
    for inline in &p.content {
        match inline {
            Inline::Text(run) => write_run(run, ctx, &mut el),
            Inline::Break { .. } => {
                el.push(Element::new(ns::A, "br"));
            }
            Inline::Raw { xml } if xml.in_ns(ns::A) || p.origin.get().is_some_and(|o| o.is(ns::A, "p")) => {
                el.push(xml.clone());
            }
            // Pictures can't sit in DrawingML text, and other formats' markup means nothing here.
            Inline::Image(_) | Inline::Raw { .. } => {}
        }
    }
    if let Some(mark) = &p.mark {
        let mut e = end.take().unwrap_or_else(|| Element::new(ns::A, "endParaRPr"));
        patch_run_props(mark, &mut e);
        el.push(e);
    }
    el
}

fn write_run(run: &Run, ctx: &mut PartCtx, para: &mut Element) {
    let origin = run.origin.get().filter(|e| e.in_ns(ns::A) && (e.local == "r" || e.local == "br"));
    if let Some(o) = origin {
        let text = if o.local == "br" { "\n".to_string() } else { o.child(ns::A, "t").map(Element::text).unwrap_or_default() };
        let rpr = o.child(ns::A, "rPr");
        let same_link = run.link == rpr.and_then(|r| current_link(r, ctx));
        if text == run.text && rpr.map(read_run_props).unwrap_or_default() == run.props && same_link {
            para.push(o.clone());
            return;
        }
    }
    let had_rpr = origin.and_then(|o| o.child(ns::A, "rPr")).is_some();
    let mut rpr = origin.and_then(|o| o.child(ns::A, "rPr")).cloned().unwrap_or_else(|| Element::new(ns::A, "rPr"));
    patch_run_props(&run.props, &mut rpr);
    patch_link(run.link.as_ref(), &mut rpr, ctx);
    let keep_rpr = had_rpr || !rpr.attrs.is_empty() || rpr.elements().next().is_some();
    let attrs = origin.filter(|o| o.local == "r").map(|o| o.attrs.clone()).unwrap_or_default();

    for (i, piece) in run.text.split('\n').enumerate() {
        if i > 0 {
            let mut br = Element::new(ns::A, "br");
            if keep_rpr {
                br.push(rpr.clone());
            }
            para.push(br);
        }
        // A line break alone is just the a:br.
        if piece.is_empty() && (i > 0 || run.text.starts_with('\n')) {
            continue;
        }
        let mut r = Element::new(ns::A, "r");
        r.attrs = attrs.clone();
        if keep_rpr {
            r.push(rpr.clone());
        }
        r.push(Element::new(ns::A, "t").with_text(piece));
        para.push(r);
    }
}

/// A run's URL hyperlink, changed only if it differs from what `rpr` has.
fn current_link(rpr: &Element, ctx: &PartCtx) -> Option<Link> {
    let c = rpr.child(ns::A, "hlinkClick")?;
    if c.attr("action").is_some_and(|a| !a.is_empty() && !a.starts_with("ppaction://hlinkfile")) {
        return None;
    }
    let url = ctx.url(rel_attr(c, "id")?)?.to_string();
    Some(Link { url: Some(url), anchor: None, tooltip: c.attr("tooltip").map(str::to_string) })
}

pub(crate) fn patch_link(link: Option<&Link>, rpr: &mut Element, ctx: &mut PartCtx) {
    let click = rpr.child(ns::A, "hlinkClick");
    let current = current_link(rpr, ctx);
    // Links to slides and actions aren't the model's; leave them be.
    if current.is_none() && click.is_some() && link.is_none() {
        return;
    }
    let wanted = link.filter(|l| l.url.is_some()).cloned();
    if wanted == current {
        return;
    }
    rpr.clear(ns::A, &["hlinkClick"]);
    if let Some(l) = wanted {
        let id = ctx.link(l.url.as_deref().unwrap_or_default());
        let mut c = Element::new(ns::A, "hlinkClick").with_attr(ns::R, "id", id);
        if let Some(t) = &l.tooltip {
            c.set_attr("", "tooltip", t);
        }
        rpr.place(c, RPR);
    }
}

/// Updates `a:rPr` (or `a:endParaRPr`, `a:defRPr`) to say `props`, touching only what changed.
pub fn patch_run_props(props: &RunProps, el: &mut Element) {
    let old = read_run_props(el);
    let face = |el: &mut Element, local: &str, value: &Option<String>| match value {
        Some(f) => el.place(Element::new(ns::A, local).with_attr("", "typeface", f), RPR),
        None => el.clear(ns::A, &[local]),
    };
    if props.font != old.font {
        face(el, "latin", &props.font);
    }
    if props.font_east_asian != old.font_east_asian {
        face(el, "ea", &props.font_east_asian);
    }
    if props.font_complex != old.font_complex {
        face(el, "cs", &props.font_complex);
    }
    if props.size != old.size {
        set_opt(el, "sz", props.size.map(|s| s.0.to_string()));
    }
    let flag = |b: Option<bool>| b.map(|b| if b { "1" } else { "0" }.to_string());
    if props.bold != old.bold {
        set_opt(el, "b", flag(props.bold));
    }
    if props.italic != old.italic {
        set_opt(el, "i", flag(props.italic));
    }
    if props.underline != old.underline {
        set_opt(el, "u", props.underline.as_deref().map(|u| underline_to_drawingml(u).to_string()));
    }
    if props.strike != old.strike {
        let s = props.strike.map(|s| match s {
            Strike::None => "noStrike",
            Strike::Single => "sngStrike",
            Strike::Double => "dblStrike",
        });
        set_opt(el, "strike", s.map(str::to_string));
    }
    if (props.caps, props.small_caps) != (old.caps, old.small_caps) {
        let cap = match (props.caps, props.small_caps) {
            (Some(true), _) => Some("all"),
            (_, Some(true)) => Some("small"),
            (Some(false), _) | (_, Some(false)) => Some("none"),
            _ => None,
        };
        set_opt(el, "cap", cap.map(str::to_string));
    }
    if props.color != old.color {
        match &props.color {
            Some(c) => el.place(Element::new(ns::A, "solidFill").with_child(c.to_drawingml()), RPR),
            None => el.clear(ns::A, RPR[1]),
        }
    }
    if props.highlight != old.highlight {
        match &props.highlight {
            Some(c) => el.place(Element::new(ns::A, "highlight").with_child(c.to_drawingml()), RPR),
            None => el.clear(ns::A, &["highlight"]),
        }
    }
    if props.baseline != old.baseline {
        let b = props.baseline.map(|b| match b {
            Baseline::Superscript => "30000",
            Baseline::Subscript => "-25000",
        });
        set_opt(el, "baseline", b.map(str::to_string));
    }
    if props.lang != old.lang {
        set_opt(el, "lang", props.lang.clone());
    }
}

/// Updates `a:pPr` to say `props`, touching only what changed.
pub fn patch_para_props(props: &ParaProps, el: &mut Element) {
    let old = read_para_props(el);
    if props.align != old.align {
        let a = props.align.map(|a| match a {
            Align::Left => "l",
            Align::Center => "ctr",
            Align::Right => "r",
            Align::Justify => "just",
            Align::Distribute => "dist",
        });
        set_opt(el, "algn", a.map(str::to_string));
    }
    if props.level != old.level {
        set_opt(el, "lvl", props.level.map(|l| l.to_string()));
    }
    if props.indent_start != old.indent_start {
        set_opt(el, "marL", props.indent_start.map(|e| e.0.to_string()));
    }
    if props.indent_end != old.indent_end {
        set_opt(el, "marR", props.indent_end.map(|e| e.0.to_string()));
    }
    if props.indent_first_line != old.indent_first_line {
        set_opt(el, "indent", props.indent_first_line.map(|e| e.0.to_string()));
    }
    if props.rtl != old.rtl {
        set_opt(el, "rtl", props.rtl.map(|b| if b { "1" } else { "0" }.to_string()));
    }
    // Points in hundredths, as DrawingML writes them.
    let points = |e: Emu| Element::new(ns::A, "spcPts").with_attr("", "val", ((e.points() * 100.0).round() as i64).to_string());
    for (local, new, before) in [("spcBef", props.space_before, old.space_before), ("spcAft", props.space_after, old.space_after)] {
        if new != before {
            match new {
                Some(e) => el.place(Element::new(ns::A, local).with_child(points(e)), PPR),
                None => el.clear(ns::A, &[local]),
            }
        }
    }
    if props.line_spacing != old.line_spacing {
        match props.line_spacing {
            Some(LineSpacing::Multiple(p)) => {
                el.place(Element::new(ns::A, "lnSpc").with_child(Element::new(ns::A, "spcPct").with_attr("", "val", p.0.to_string())), PPR)
            }
            // DrawingML has no "at least"; exact is the nearest it can say.
            Some(LineSpacing::Exactly(e) | LineSpacing::AtLeast(e)) => el.place(Element::new(ns::A, "lnSpc").with_child(points(e)), PPR),
            None => el.clear(ns::A, &["lnSpc"]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::Rgb;
    use crate::xml::parse_element;

    fn p(inner: &str) -> Paragraph {
        read_paragraph(&parse_element(&format!(r#"<a:p xmlns:a="{}">{inner}</a:p>"#, ns::A)).unwrap(), None)
    }

    #[test]
    fn reads_runs_and_breaks() {
        let para = p(
            r#"<a:pPr algn="ctr" lvl="1" marL="457200" indent="-228600"><a:lnSpc><a:spcPct val="150000"/></a:lnSpc><a:spcBef><a:spcPts val="600"/></a:spcBef><a:buChar char="•"/></a:pPr><a:r><a:rPr lang="en-GB" sz="2400" b="1" u="sng" strike="dblStrike" cap="small" baseline="30000"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill><a:latin typeface="Georgia"/></a:rPr><a:t>Big</a:t></a:r><a:br><a:rPr sz="2400"/></a:br><a:r><a:t>next</a:t></a:r><a:fld id="{X}" type="slidenum"><a:t>3</a:t></a:fld><a:endParaRPr lang="en-GB" sz="1800"/>"#,
        );
        assert_eq!(para.props.align, Some(Align::Center));
        assert_eq!(para.props.level, Some(1));
        assert_eq!(para.props.indent_first_line, Some(Emu(-228_600)));
        assert_eq!(para.props.line_spacing, Some(LineSpacing::Multiple(Percent(150_000))));
        assert_eq!(para.props.space_before, Some(Emu::from_points(6.0)));
        let runs: Vec<&Run> = para.runs().collect();
        assert_eq!(runs.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["Big", "\n", "next"]);
        let big = &runs[0].props;
        assert_eq!(big.size, Some(FontSize(2400)));
        assert_eq!(big.bold, Some(true));
        assert_eq!(big.underline.as_deref(), Some("single"));
        assert_eq!(big.strike, Some(Strike::Double));
        assert_eq!(big.small_caps, Some(true));
        assert_eq!(big.baseline, Some(Baseline::Superscript));
        assert_eq!(big.font.as_deref(), Some("Georgia"));
        assert_eq!(big.color, Some(Color::rgb(Rgb(255, 0, 0))));
        assert_eq!(big.lang.as_deref(), Some("en-GB"));
        assert!(matches!(para.content[3], Inline::Raw { .. }), "the field is kept whole");
        assert_eq!(para.mark.as_ref().unwrap().size, Some(FontSize(1800)));
        assert_eq!(para.plain_text(), "Big\nnext");
        // The bullet isn't modeled; it stays in the origin's pPr.
        assert!(para.origin.get().unwrap().descendant(ns::A, "buChar").is_some());
        assert!(para.origin.get().unwrap().child(ns::A, "r").is_none());
    }
}
