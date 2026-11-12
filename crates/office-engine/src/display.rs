//! The display list: what the app paints. Pages, slides and grids are lists of items in pixels,
//! text referring to a [`crate::style::PaintStyle`] by number, pictures to a media id. The app
//! turns them into DOM, so text stays real text (selectable by assistive tech, crisp at any
//! zoom), and the engine never touches the DOM.

use serde::Serialize;

use crate::flow::{BlockBox, BorderLine, FlowLayout, ParaBox, TableBox};
use crate::para::{Ctx, FragKind, LineEnd};

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "t", rename_all = "camelCase")]
pub enum Item {
    /// Text on a baseline at (`x`, `y`), `w` wide, in style `s`. `ws` is extra space after each
    /// space character (justified text).
    Text {
        x: f64,
        y: f64,
        w: f64,
        s: u32,
        text: String,
        #[serde(skip_serializing_if = "is_zero")]
        ws: f64,
        /// A hyperlink's address, for ⌘-click.
        #[serde(skip_serializing_if = "Option::is_none")]
        link: Option<String>,
        /// Right-to-left text: the host draws it with that base direction. The engine has
        /// already split and ordered mixed text, so each piece runs one way.
        #[serde(skip_serializing_if = "is_false")]
        rtl: bool,
    },
    Rect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        fill: Option<String>,
    },
    Line {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        color: String,
        w: f64,
        #[serde(skip_serializing_if = "is_solid")]
        dash: &'static str,
    },
    /// A picture: `crop` is the share cut off each side (left, top, right, bottom), 0 to 1.
    Image {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        media: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        crop: Option<[f64; 4]>,
        /// Which object this is, for selecting it: the paragraph path and offset in Word, the
        /// frame id on a slide.
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// An SVG path at (`x`, `y`).
    Path {
        x: f64,
        y: f64,
        d: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        fill: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        stroke: Option<String>,
        #[serde(skip_serializing_if = "is_zero")]
        sw: f64,
        #[serde(skip_serializing_if = "is_solid")]
        dash: &'static str,
    },
    /// Items placed, rotated and flipped together: a shape and its text, a group.
    Group {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        #[serde(skip_serializing_if = "is_zero")]
        rot: f64,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        fh: bool,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        fv: bool,
        /// Clip children to the box (text that overflows a table cell).
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        clip: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        items: Vec<Item>,
    },
}

fn is_false(v: &bool) -> bool {
    !*v
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

fn is_solid(d: &&str) -> bool {
    d.is_empty() || *d == "solid"
}

/// Rounds to hundredths of a pixel: plenty for painting, and it keeps the JSON short.
pub fn r(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Draws what a layer owns inside text: pictures and other objects in paragraphs.
pub trait Objects {
    /// Draws the object at inline `inline` of the paragraph at `para` into `out`, in the box
    /// `rect` (x, y, width, height).
    fn object(&mut self, para: &crate::pos::Path, inline: u32, rect: Rect, out: &mut Vec<Item>);
}

/// A link's address for the text at an offset of a paragraph, if it's linked.
pub type Links<'a> = &'a dyn Fn(&crate::pos::Path, u32) -> Option<String>;

/// Where painting goes: the items drawn, and the layer's help with objects and links.
pub struct Sink<'a> {
    pub objects: &'a mut dyn Objects,
    pub links: Links<'a>,
    pub out: &'a mut Vec<Item>,
}

/// No objects to draw.
pub struct NoObjects;

impl Objects for NoObjects {
    fn object(&mut self, _: &crate::pos::Path, _: u32, _: Rect, _: &mut Vec<Item>) {}
}

/// Paints a flow with its top left at (`x`, `y`), drawing only what's within `rows` (from the
/// flow's top), so a page can paint its share of a flow cut across pages.
pub fn paint_flow(ctx: &mut Ctx, flow: &FlowLayout, x: f64, y: f64, rows: (f64, f64), sink: &mut Sink) {
    for b in &flow.blocks {
        match b {
            BlockBox::Para(p) => paint_para(ctx, p, x, y, rows, sink),
            BlockBox::Table(t) => paint_table(ctx, t, x, y, rows, sink),
            BlockBox::Empty { .. } => {}
        }
    }
}

/// Paints a paragraph's lines that fall within `rows`.
pub fn paint_para(ctx: &mut Ctx, p: &ParaBox, x: f64, y: f64, rows: (f64, f64), sink: &mut Sink) {
    for line in &p.layout.lines {
        let top = p.y + line.top;
        if top + line.height <= rows.0 + 0.01 || top >= rows.1 - 0.01 {
            continue;
        }
        paint_line(ctx, p, line, x, y + top, sink);
    }
}

/// Paints one line with its top at `top`.
pub fn paint_line(ctx: &mut Ctx, p: &ParaBox, line: &crate::para::Line, x: f64, top: f64, sink: &mut Sink) {
    let Sink { objects, links, out } = sink;
    let baseline = top + line.baseline;
    for f in &line.frags {
        match &f.kind {
            FragKind::Text(s) => {
                let mut text = &p.spec.text[f.start as usize..f.end as usize];
                // A line ending in a break keeps the break out of its text.
                if line.ends != LineEnd::Wrap && f.end == line.end {
                    text = text.trim_end_matches(['\n', crate::text::PAGE_BREAK, crate::text::COLUMN_BREAK]);
                }
                if text.is_empty() {
                    continue;
                }
                let style = ctx.styles.get(*s);
                let shown = style.transform(text).into_owned();
                out.push(Item::Text {
                    x: r(x + f.x),
                    y: r(baseline),
                    w: r(f.width),
                    s: s.0,
                    text: shown,
                    ws: r(f.space),
                    link: links(&p.path, f.start),
                    rtl: f.rtl,
                });
            }
            FragKind::Label { text, style } => {
                out.push(Item::Text {
                    x: r(x + f.x),
                    y: r(baseline),
                    w: r(f.width),
                    s: style.0,
                    text: text.clone(),
                    ws: 0.0,
                    link: None,
                    rtl: f.rtl,
                });
            }
            FragKind::Atomic { text, style, .. } => {
                out.push(Item::Text {
                    x: r(x + f.x),
                    y: r(baseline),
                    w: r(f.width),
                    s: style.0,
                    text: text.clone(),
                    ws: 0.0,
                    link: None,
                    rtl: f.rtl && !text.is_ascii(),
                });
            }
            FragKind::Object { inline, height } => {
                objects.object(&p.path, *inline, (r(x + f.x), r(baseline - height), r(f.width), r(*height)), out)
            }
            FragKind::Tab(s) => {
                // Underlined or highlighted tabs show their line or color.
                let style = ctx.styles.get(*s);
                if style.underline.is_some() || style.highlight.is_some() {
                    out.push(Item::Text {
                        x: r(x + f.x),
                        y: r(baseline),
                        w: r(f.width),
                        s: s.0,
                        text: "\u{a0}".repeat(((f.width / 4.0).ceil() as usize).max(1)),
                        ws: 0.0,
                        link: None,
                        rtl: false,
                    });
                }
            }
        }
    }
}

/// Paints a table's cells, fills and borders within `rows`.
pub fn paint_table(ctx: &mut Ctx, t: &TableBox, x: f64, y: f64, rows: (f64, f64), sink: &mut Sink) {
    let tx = x + t.x;
    let ty = y + t.y;
    for cell in &t.cells {
        let top = t.y + cell.y;
        let bottom = top + cell.height;
        if bottom <= rows.0 + 0.01 || top >= rows.1 - 0.01 {
            continue;
        }
        let (cx, cy) = (tx + cell.x, ty + cell.y);
        // Only the part of the cell on this page.
        let vis_top = top.max(rows.0) - t.y;
        let vis_bottom = bottom.min(rows.1) - t.y;
        if let Some(fill) = &cell.look.fill {
            sink.out.push(Item::Rect {
                x: r(cx),
                y: r(ty + vis_top),
                w: r(cell.width),
                h: r(vis_bottom - vis_top),
                fill: Some(fill.clone()),
            });
        }
        let inner_x = cx + cell.look.margins[3];
        let inner_y = cy + cell.content_y;
        let inner_rows = (rows.0 - (t.y + cell.y + cell.content_y), rows.1 - (t.y + cell.y + cell.content_y));
        paint_flow(ctx, &cell.content, inner_x, inner_y, inner_rows, sink);
        let [bt, br, bb, bl] = &cell.look.borders;
        let (x0, x1) = (cx, cx + cell.width);
        let (y0, y1) = (ty + vis_top, ty + vis_bottom);
        let mut line = |b: &Option<BorderLine>, x1: f64, y1: f64, x2: f64, y2: f64| {
            if let Some(b) = b {
                sink.out.push(Item::Line {
                    x1: r(x1),
                    y1: r(y1),
                    x2: r(x2),
                    y2: r(y2),
                    color: b.color.clone(),
                    w: r(b.width),
                    dash: b.style,
                });
            }
        };
        // A border cut by a page break is drawn at the cut too, as Word does.
        line(bt, x0, y0, x1, y0);
        line(br, x1, y0, x1, y1);
        line(bb, x0, y1, x1, y1);
        line(bl, x0, y0, x0, y1);
    }
}

/// A rectangle on screen: (x, y, width, height).
pub type Rect = (f64, f64, f64, f64);

/// The rectangles covering offsets `from..to` of a laid-out paragraph whose box is at
/// (`x`, `y`), one per line, for drawing a selection. `to_end` extends the last line's
/// rectangle a little past its text (the paragraph mark is selected too).
pub fn selection_rects(ctx: &mut Ctx, p: &ParaBox, x: f64, y: f64, from: u32, to: u32, to_end: bool) -> Vec<Rect> {
    let mut out = Vec::new();
    for line in &p.layout.lines {
        if to < line.start || from > line.end || (from == line.end && line.ends == LineEnd::Wrap && from != to) {
            continue;
        }
        let a = from.max(line.start);
        let b = to.min(line.end);
        if line.bidi {
            out.extend(bidi_line_rects(ctx, p, line, (x, y), (a, b), (from, to, to_end)));
            continue;
        }
        let x0 = crate::para::x_in_line(ctx, &p.spec, line, a);
        let mut x1 = crate::para::x_in_line(ctx, &p.spec, line, b);
        let through_end = to > line.end || (to == line.end && line.ends == LineEnd::Wrap);
        if through_end || (to_end && b == line.end) {
            x1 += 5.0;
        }
        if x1 > x0 || from != to {
            out.push((r(x + x0), r(y + p.y + line.top), r((x1 - x0).max(2.0)), r(line.height)));
        }
    }
    out
}

/// [`selection_rects`] for a line with right-to-left text: a selection in text order covers
/// pieces scattered across the line, so each frag gets its own rectangle (merged where they
/// touch).
fn bidi_line_rects(
    ctx: &mut Ctx,
    p: &ParaBox,
    line: &crate::para::Line,
    (x, y): (f64, f64),
    (a, b): (u32, u32),
    (from, to, to_end): (u32, u32, bool),
) -> Vec<Rect> {
    let mut spans: Vec<(f64, f64)> = Vec::new();
    for f in &line.frags {
        if matches!(f.kind, FragKind::Label { .. }) || f.end <= a || f.start >= b {
            continue;
        }
        let x0 = crate::para::frag_x(ctx, &p.spec, f, a.max(f.start));
        let x1 = if b >= f.end { f.end_x() } else { crate::para::frag_x(ctx, &p.spec, f, b) };
        spans.push((x0.min(x1), x0.max(x1)));
    }
    // Through the end of the line: a little past it, on its end side.
    let through_end = to > line.end || (to == line.end && line.ends == LineEnd::Wrap);
    if through_end || (to_end && b == line.end) {
        let end = crate::para::x_in_line(ctx, &p.spec, line, line.end);
        let end = if p.spec.rtl { spans.iter().map(|s| s.0).fold(end, f64::min) } else { spans.iter().map(|s| s.1).fold(end, f64::max) };
        spans.push(if p.spec.rtl { (end - 5.0, end) } else { (end, end + 5.0) });
    }
    if spans.is_empty() {
        if from == to {
            return Vec::new();
        }
        // Nothing but the line's end selected: a sliver at the caret.
        let at = crate::para::x_in_line(ctx, &p.spec, line, a);
        spans.push(if p.spec.rtl { (at - 2.0, at) } else { (at, at + 2.0) });
    }
    spans.sort_by(|l, r| l.0.total_cmp(&r.0));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for s in spans {
        match merged.last_mut() {
            Some(last) if s.0 <= last.1 + 0.5 => last.1 = last.1.max(s.1),
            _ => merged.push(s),
        }
    }
    merged.into_iter().map(|(x0, x1)| (r(x + x0), r(y + p.y + line.top), r((x1 - x0).max(2.0)), r(line.height))).collect()
}

/// The caret for `offset` in a laid-out paragraph whose box is at (`x`, `y`): a vertical bar
/// as (x, top, height).
pub fn caret_rect(ctx: &mut Ctx, p: &ParaBox, x: f64, y: f64, offset: u32, before: bool) -> (f64, f64, f64) {
    caret_rect_near(ctx, p, x, y, offset, before, None)
}

/// [`caret_rect`], where the offset shows in two places (text changing direction), at the one
/// nearer `near` (where it was clicked).
pub fn caret_rect_near(ctx: &mut Ctx, p: &ParaBox, x: f64, y: f64, offset: u32, before: bool, near: Option<f64>) -> (f64, f64, f64) {
    let (mut cx, li) = p.layout.caret(ctx, &p.spec, offset, before);
    let line = &p.layout.lines[li];
    if let Some(near) = near {
        if let Some(other) = crate::para::other_x_in_line(ctx, &p.spec, line, offset) {
            if (x + other - near).abs() < (x + cx - near).abs() {
                cx = other;
            }
        }
    }
    (r(x + cx), r(y + p.y + line.top), r(line.height))
}

/// Paint styles the host hasn't been sent yet.
pub fn styles_since(styles: &crate::para::Styles, sent: usize) -> Vec<crate::style::PaintStyle> {
    styles.iter().skip(sent).map(|s| s.paint()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_serialize_compactly() {
        let item = Item::Text { x: 1.0, y: 2.0, w: 3.0, s: 0, text: "hi".into(), ws: 0.0, link: None, rtl: false };
        let json = serde_json_like(&item);
        assert!(json.contains("\"t\":\"text\""));
        assert!(!json.contains("ws"));
    }

    fn serde_json_like(item: &Item) -> String {
        serde_json::to_string(item).unwrap()
    }
}
