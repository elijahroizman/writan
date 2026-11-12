//! Text in shapes and table cells: paragraphs with what they inherit (placeholder, layout and
//! master list styles), bullets and automatic numbers, the body's insets and anchoring, and
//! PowerPoint's shrink-on-overflow scaling, turned into what the engine lays out.

use std::sync::Arc;

use office_core::drawing::{Color, Rgba};
use office_core::model::{Align, Document, Frame, FrameContent, LineSpacing, ParaProps, Paragraph, RunProps, Slide};
use office_core::units::Emu;
use office_core::xml::{ns, Element};
use office_engine::flow::{BlockBox, FlowLayout, Keep, ParaBox};
use office_engine::para::{self, Ctx, Item, ItemKind, Label, ParaSpec, Spacing, StyleId};
use office_engine::pos::Path;
use office_engine::style::TextStyle;
use office_engine::text;

use super::art::ArtSet;

pub fn emu_px(e: Emu) -> f64 {
    e.0 as f64 / 9525.0
}

/// A text body's own settings (`a:bodyPr`), with what it inherits.
#[derive(Clone, Debug, PartialEq)]
pub struct Body {
    /// Left, top, right, bottom, in pixels.
    pub insets: [f64; 4],
    /// 0 top, 1 middle, 2 bottom.
    pub anchor: u8,
    pub wrap: bool,
    /// `normAutofit`'s font scale and line spacing reduction, as fractions.
    pub font_scale: f64,
    pub spacing_cut: f64,
    /// Rotated text: 90 (`vert`), 270 (`vert270`), or 0.
    pub vertical: i32,
}

impl Default for Body {
    fn default() -> Body {
        Body { insets: [9.6, 4.8, 9.6, 4.8], anchor: 0, wrap: true, font_scale: 1.0, spacing_cut: 0.0, vertical: 0 }
    }
}

impl Body {
    fn apply(&mut self, pr: &Element) {
        let ins = |k: &str| pr.attr(k).and_then(|v| v.parse::<f64>().ok()).map(|v| v / 9525.0);
        for (i, k) in ["lIns", "tIns", "rIns", "bIns"].iter().enumerate() {
            if let Some(v) = ins(k) {
                self.insets[i] = v;
            }
        }
        match pr.attr("anchor") {
            Some("ctr") => self.anchor = 1,
            Some("b") => self.anchor = 2,
            Some("t") => self.anchor = 0,
            _ => {}
        }
        match pr.attr("wrap") {
            Some("none") => self.wrap = false,
            Some("square") => self.wrap = true,
            _ => {}
        }
        if let Some(fit) = pr.child(ns::A, "normAutofit") {
            self.font_scale = fit.attr("fontScale").and_then(|v| v.parse::<f64>().ok()).map(|v| v / 100_000.0).unwrap_or(1.0);
            self.spacing_cut = fit.attr("lnSpcReduction").and_then(|v| v.parse::<f64>().ok()).map(|v| v / 100_000.0).unwrap_or(0.0);
        } else if pr.child(ns::A, "noAutofit").is_some() || pr.child(ns::A, "spAutoFit").is_some() {
            self.font_scale = 1.0;
            self.spacing_cut = 0.0;
        }
        match pr.attr("vert") {
            Some("vert") | Some("eaVert") => self.vertical = 90,
            Some("vert270") => self.vertical = 270,
            Some("horz") => self.vertical = 0,
            _ => {}
        }
    }
}

/// The list-style elements a frame's paragraphs inherit, least specific first: the master's
/// text style (or the presentation's default for text outside placeholders), the master's
/// placeholder, the layout's placeholder, and the frame's own `a:lstStyle`.
pub struct Cascade<'a> {
    pub lists: Vec<&'a Element>,
    pub bodies: Vec<&'a Element>,
    /// Shape properties to fall back on for fill and line: master's, then layout's placeholder.
    pub sp_prs: Vec<&'a Element>,
    pub styles: Vec<&'a Element>,
}

pub fn cascade<'a>(doc: &'a Document, art: &'a ArtSet, slide: &Slide, frame: &'a Frame) -> Cascade<'a> {
    let (layout, master) = art.chain(slide.layout.as_deref());
    let mut c = Cascade { lists: Vec::new(), bodies: Vec::new(), sp_prs: Vec::new(), styles: Vec::new() };
    let ph = frame.placeholder.as_ref().map(|p| (p.kind.clone().unwrap_or_else(|| "obj".into()), p.index));
    match &ph {
        Some((kind, index)) => {
            let from_layout = layout.and_then(|l| l.matching(kind, *index));
            let master_kind = from_layout.map(|p| p.kind.clone()).unwrap_or_else(|| kind.clone());
            let from_master = master.and_then(|m| m.matching(&master_kind, None));
            let fam = super::art::family(&master_kind);
            let style_name = match fam {
                "title" => "titleStyle",
                "other" => "otherStyle",
                _ => "bodyStyle",
            };
            if let Some(s) = master.and_then(|m| m.tx_styles.as_ref()).and_then(|t| t.child(ns::P, style_name)) {
                c.lists.push(s);
            }
            for p in [from_master, from_layout].into_iter().flatten() {
                c.lists.extend(p.lst_style.as_ref());
                c.bodies.extend(p.body_pr.as_ref());
                c.sp_prs.extend(p.sp_pr.as_ref());
                c.styles.extend(p.style.as_ref());
            }
        }
        None => {
            if let Some(d) = doc.origin.get().and_then(|o| o.child(ns::P, "defaultTextStyle")) {
                c.lists.push(d);
            }
        }
    }
    let _ = (layout, master);
    if let FrameContent::Shape { text: Some(body), .. } = &frame.content {
        if let Some(o) = body.origin.get() {
            c.lists.extend(o.child(ns::A, "lstStyle"));
            c.bodies.extend(o.child(ns::A, "bodyPr"));
        }
    }
    c
}

impl Cascade<'_> {
    pub fn body(&self) -> Body {
        let mut b = Body::default();
        for pr in &self.bodies {
            b.apply(pr);
        }
        b
    }

    /// The paragraph-property elements a paragraph at `level` inherits, then its own.
    fn levels<'b>(&'b self, level: usize, own: Option<&'b Element>) -> Vec<&'b Element> {
        let name = format!("lvl{}pPr", level + 1);
        let mut out: Vec<&Element> = self.lists.iter().filter_map(|l| l.child(ns::A, &name)).collect();
        out.extend(own);
        out
    }
}

/// A paragraph's bullet, from its properties and what they inherit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bullet {
    /// A character bullet, or an automatic number scheme (`arabicPeriod`…) with its start.
    pub char: Option<String>,
    pub auto: Option<(String, u32)>,
    pub font: Option<String>,
    pub color: Option<Color>,
    /// Size relative to the text, as a fraction.
    pub size: Option<f64>,
}

fn bullet_of(levels: &[&Element]) -> Bullet {
    let mut b = Bullet::default();
    for el in levels {
        if el.child(ns::A, "buNone").is_some() {
            b.char = None;
            b.auto = None;
        }
        if let Some(c) = el.child(ns::A, "buChar") {
            b.char = c.attr("char").map(str::to_string);
            b.auto = None;
        }
        if let Some(a) = el.child(ns::A, "buAutoNum") {
            b.auto =
                Some((a.attr("type").unwrap_or("arabicPeriod").to_string(), a.attr("startAt").and_then(|v| v.parse().ok()).unwrap_or(1)));
            b.char = None;
        }
        if let Some(f) = el.child(ns::A, "buFont") {
            b.font = f.attr("typeface").map(str::to_string);
        }
        if el.child(ns::A, "buFontTx").is_some() {
            b.font = None;
        }
        if let Some(c) = el.child(ns::A, "buClr").and_then(Color::in_drawingml) {
            b.color = Some(c);
        }
        if el.child(ns::A, "buClrTx").is_some() {
            b.color = None;
        }
        if let Some(s) = el.child(ns::A, "buSzPct").and_then(|s| s.attr("val")).and_then(|v| v.parse::<f64>().ok()) {
            b.size = Some(s / 100_000.0);
        }
        if el.child(ns::A, "buSzTx").is_some() {
            b.size = None;
        }
    }
    b
}

/// Percent spacing before or after (`spcPct`), as a fraction of the line, from the cascade.
fn spacing_pct(levels: &[&Element], local: &str) -> Option<f64> {
    levels.iter().rev().find_map(|el| {
        let s = el.child(ns::A, local)?;
        if s.child(ns::A, "spcPts").is_some() {
            return Some(None);
        }
        Some(s.child(ns::A, "spcPct").and_then(|p| p.attr("val")).and_then(|v| v.parse::<f64>().ok()).map(|v| v / 100_000.0))
    })?
}

/// An automatic number in one of PowerPoint's schemes.
pub fn auto_number(n: u32, scheme: &str) -> String {
    let roman = |mut n: u32| {
        let table = [
            (1000, "m"),
            (900, "cm"),
            (500, "d"),
            (400, "cd"),
            (100, "c"),
            (90, "xc"),
            (50, "l"),
            (40, "xl"),
            (10, "x"),
            (9, "ix"),
            (5, "v"),
            (4, "iv"),
            (1, "i"),
        ];
        let mut s = String::new();
        for (v, r) in table {
            while n >= v {
                s.push_str(r);
                n -= v;
            }
        }
        s
    };
    let alpha = |n: u32| {
        let n = n.max(1) - 1;
        let c = (b'a' + (n % 26) as u8) as char;
        c.to_string().repeat(n as usize / 26 + 1)
    };
    let (body, wrap) = if let Some(rest) = scheme.strip_prefix("arabic") {
        (n.to_string(), rest)
    } else if let Some(rest) = scheme.strip_prefix("alphaLc") {
        (alpha(n), rest)
    } else if let Some(rest) = scheme.strip_prefix("alphaUc") {
        (alpha(n).to_uppercase(), rest)
    } else if let Some(rest) = scheme.strip_prefix("romanLc") {
        (roman(n), rest)
    } else if let Some(rest) = scheme.strip_prefix("romanUc") {
        (roman(n).to_uppercase(), rest)
    } else {
        (n.to_string(), "Period")
    };
    match wrap {
        "ParenBoth" => format!("({body})"),
        "ParenR" => format!("{body})"),
        "Plain" => body,
        "Minus" => format!("- {body} -"),
        _ => format!("{body}."),
    }
}

/// Everything needed to turn colors in a slide into paint.
pub struct Paint<'a> {
    pub doc: &'a Document,
    pub slide: &'a Slide,
}

impl Paint<'_> {
    pub fn color(&self, c: &Color) -> Rgba {
        crate::color(self.doc, self.slide, c)
    }

    /// The theme's minor (body) font: what text without a font is in.
    pub fn body_font(&self) -> String {
        self.doc.theme.as_ref().map(|t| t.fonts.minor.latin.clone()).filter(|f| !f.is_empty()).unwrap_or_else(|| "Calibri".into())
    }
}

/// The default text size in PowerPoint when nothing sets one: 18 pt.
const DEFAULT_PX: f64 = 18.0 * 96.0 / 72.0;

/// Text body specs, laid out down `width` pixels, ready to draw and edit. `paragraphs` and
/// `resolved` pair up (see [`crate::resolve`]); `text_color` is the shape style's font color, the
/// default for text that doesn't set one.
#[allow(clippy::too_many_arguments)]
pub fn layout_body(
    ctx: &mut Ctx,
    cache: &mut office_engine::flow::Cache,
    paint: &Paint,
    cascade: &Cascade,
    body: &Body,
    paragraphs: &[Paragraph],
    resolved: &[(ParaProps, Vec<RunProps>)],
    text_color: Option<&Color>,
    width: f64,
) -> FlowLayout {
    let mut blocks = Vec::with_capacity(paragraphs.len());
    let mut counters = [0u32; 9];
    let mut prev_level: Option<usize> = None;
    for (i, p) in paragraphs.iter().enumerate() {
        let empty = (ParaProps::default(), Vec::new());
        let (pp, runs) = resolved.get(i).unwrap_or(&empty);
        let level = p.props.level.or(pp.level).unwrap_or(0).min(8) as usize;
        let own = p.origin.get().and_then(|o| o.child(ns::A, "pPr"));
        let levels = cascade.levels(level, own);
        let bullet = bullet_of(&levels);
        let spec = para_spec(ctx, paint, body, p, pp, runs, &levels, &bullet, &mut counters, &mut prev_level, level, text_color, i == 0);
        let spec = Arc::new(spec);
        let layout = cache.layout(ctx, &spec, if body.wrap { width.max(1.0) } else { 100_000.0 });
        blocks.push(BlockBox::Para(ParaBox { path: Path::top(i as u32), spec, layout, keep: Keep::default(), y: 0.0 }));
    }
    let height = office_engine::flow::restack(&mut blocks);
    FlowLayout { blocks, height }
}

/// One paragraph's spec with nothing inherited but `runs` (text in a table cell).
pub fn cell_spec(ctx: &mut Ctx, paint: &Paint, p: &Paragraph, runs: &[RunProps], text_color: Option<&Color>) -> ParaSpec {
    let body = Body { insets: [0.0; 4], ..Body::default() };
    let mut pp = p.props.clone();
    pp.level = Some(0);
    let own = p.origin.get().and_then(|o| o.child(ns::A, "pPr"));
    let levels: Vec<&Element> = own.into_iter().collect();
    let bullet = bullet_of(&levels);
    para_spec(ctx, paint, &body, p, &pp, runs, &levels, &bullet, &mut [0; 9], &mut None, 0, text_color, true)
}

#[allow(clippy::too_many_arguments)]
fn para_spec(
    ctx: &mut Ctx,
    paint: &Paint,
    body: &Body,
    p: &Paragraph,
    pp: &ParaProps,
    runs: &[RunProps],
    levels: &[&Element],
    bullet: &Bullet,
    counters: &mut [u32; 9],
    prev_level: &mut Option<usize>,
    level: usize,
    text_color: Option<&Color>,
    first: bool,
) -> ParaSpec {
    let scale = body.font_scale;
    let style_of = |ctx: &mut Ctx, props: &RunProps, link: bool| -> StyleId {
        let mut props = props.clone();
        if props.font.is_none() {
            props.font = Some(paint.body_font());
        }
        if let Some(size) = props.size {
            props.size = Some(office_core::units::FontSize(((size.0 as f64) * scale).round() as u32));
        }
        if props.color.is_none() {
            props.color = Some(if link { Color::scheme("hlink") } else { text_color.cloned().unwrap_or_else(|| Color::scheme("tx1")) });
        }
        if link && props.underline.is_none() {
            props.underline = Some("single".into());
        }
        let mut s = TextStyle::from_props(&props, DEFAULT_PX * scale, &|c| paint.color(c));
        s.link = link;
        ctx.styles.add(ctx.measure, s)
    };
    let first_props = runs.first().cloned().unwrap_or_default();
    let mark_props = {
        let mut m = first_props.clone();
        if let Some(own) = &p.mark {
            m.overlay(own);
        }
        m
    };
    let mark = style_of(ctx, &mark_props, false);
    let flat = text::flat(p);
    let mut items: Vec<Item> = Vec::with_capacity(p.content.len() + 2);
    let mut run_i = 0;
    for (i, inline) in p.content.iter().enumerate() {
        let (start, end) = flat.spans[i];
        match inline {
            office_core::model::Inline::Text(r) => {
                let props = runs.get(run_i).cloned().unwrap_or_else(|| first_props.clone());
                run_i += 1;
                let style = style_of(ctx, &props, r.link.is_some());
                let mut s = 0usize;
                for (k, c) in r.text.char_indices() {
                    let kind = match c {
                        '\t' => ItemKind::Tab(style),
                        '\n' | '\u{b}' => ItemKind::Break(style),
                        _ => continue,
                    };
                    if k > s {
                        items.push(Item { start: start + s as u32, end: start + k as u32, kind: ItemKind::Text(style) });
                    }
                    items.push(Item { start: start + k as u32, end: start + k as u32 + c.len_utf8() as u32, kind });
                    s = k + c.len_utf8();
                }
                if r.text.len() > s {
                    items.push(Item { start: start + s as u32, end, kind: ItemKind::Text(style) });
                }
            }
            office_core::model::Inline::Break { .. } => items.push(Item { start, end, kind: ItemKind::Break(mark) }),
            office_core::model::Inline::Image(_) => items.push(Item { start, end, kind: ItemKind::Hidden }),
            office_core::model::Inline::Raw { xml } => {
                // Fields (slide number, date) show their text.
                let shown: String = xml.descendant(ns::A, "t").map(|t| t.text()).unwrap_or_default();
                if shown.is_empty() {
                    items.push(Item { start, end, kind: ItemKind::Hidden });
                } else {
                    items.push(Item { start, end, kind: ItemKind::Atomic { text: shown, style: mark, inline: i as u32 } });
                }
            }
        }
    }

    // Bullets and numbers: an empty paragraph shows none, as in PowerPoint.
    let has_text = !flat.text.trim().is_empty();
    if prev_level.is_none_or(|l| l < level) {
        for c in counters.iter_mut().skip(level) {
            *c = 0;
        }
    }
    let label = if !has_text {
        None
    } else if let Some(ch) = &bullet.char {
        let mut props = first_props.clone();
        if let Some(f) = &bullet.font {
            if !matches!(f.as_str(), "Wingdings" | "Symbol" | "Webdings") {
                props.font = Some(f.clone());
            }
        }
        if let Some(c) = &bullet.color {
            props.color = Some(c.clone());
        }
        if let Some(k) = bullet.size {
            props.size = Some(office_core::units::FontSize(((props.size.map(|s| s.0 as f64).unwrap_or(1800.0)) * k).round() as u32));
        }
        props.underline = None;
        props.bold = None;
        Some(Label { text: symbol_bullet(ch), style: style_of(ctx, &props, false) })
    } else if let Some((scheme, start)) = &bullet.auto {
        counters[level] += 1;
        let n = start + counters[level] - 1;
        let mut props = first_props.clone();
        if let Some(c) = &bullet.color {
            props.color = Some(c.clone());
        }
        props.underline = None;
        Some(Label { text: auto_number(n, scheme), style: style_of(ctx, &props, false) })
    } else {
        None
    };
    if has_text {
        *prev_level = Some(level);
    }

    let px = |e: Option<Emu>| e.map(emu_px).unwrap_or(0.0);
    let size_px = first_props.size.map(|s| s.points() * 96.0 / 72.0).unwrap_or(DEFAULT_PX) * scale;
    // Spacing in points, or as a share of a line.
    let line = size_px * 1.2;
    let space_before =
        if first { 0.0 } else { spacing_pct(levels, "spcBef").map(|f| f * line).unwrap_or_else(|| px(pp.space_before) * scale) };
    let space_after = spacing_pct(levels, "spcAft").map(|f| f * line).unwrap_or_else(|| px(pp.space_after) * scale);
    let spacing = match pp.line_spacing {
        Some(LineSpacing::Multiple(m)) => Spacing::Multiple((m.fraction() - body.spacing_cut).max(0.1)),
        Some(LineSpacing::Exactly(e)) => Spacing::Exactly(emu_px(e) * scale),
        Some(LineSpacing::AtLeast(e)) => Spacing::AtLeast(emu_px(e) * scale),
        None => Spacing::Multiple((1.0 - body.spacing_cut).max(0.1)),
    };
    ParaSpec {
        text: flat.text,
        items,
        align: match pp.align {
            Some(Align::Center) => para::Align::Center,
            Some(Align::Right) => para::Align::Right,
            Some(Align::Justify | Align::Distribute) => para::Align::Justify,
            _ => para::Align::Left,
        },
        // DrawingML's `algn` names sides, and `marL` is the start margin.
        rtl: pp.rtl == Some(true),
        indent_start: px(pp.indent_start),
        indent_end: px(pp.indent_end),
        first_line: px(pp.indent_first_line),
        space_before,
        space_after,
        spacing,
        tabs: Vec::new(),
        default_tab: 96.0,
        label,
        mark,
        line_gap: true,
    }
}

/// Bullet characters from symbol fonts, as the characters they look like.
fn symbol_bullet(ch: &str) -> String {
    match ch.chars().next() {
        Some('\u{F0B7}' | '\u{F06C}' | '\u{2022}') | None => "•".into(),
        Some('\u{F0A7}' | '\u{F06E}' | '§') => "▪".into(),
        Some('\u{F0D8}' | 'Ø') => "➢".into(),
        Some('\u{F0FC}' | 'ü') => "✓".into(),
        Some('q') => "❑".into(),
        Some('v') => "❖".into(),
        Some(c) if ('\u{F000}'..='\u{F0FF}').contains(&c) => "•".into(),
        Some(_) => ch.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(auto_number(3, "arabicPeriod"), "3.");
        assert_eq!(auto_number(2, "alphaLcParenR"), "b)");
        assert_eq!(auto_number(4, "romanUcPeriod"), "IV.");
        assert_eq!(auto_number(1, "arabicParenBoth"), "(1)");
    }
}
