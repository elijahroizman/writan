//! Laying out a paragraph: breaking its text into lines across a width, aligning them, placing
//! tabs, pictures and list labels, and working out each line's height. The same code lays out
//! a Word paragraph, the text in a PowerPoint shape and a wrapped spreadsheet cell.
//!
//! A layer describes the paragraph as a [`ParaSpec`]: its text with one character standing for
//! each object (a picture, a break, a field), and items saying what each stretch of that text
//! is. Offsets are byte offsets into that text, and they're also how positions and selections
//! address the paragraph.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use unicode_bidi::{bidi_class, BidiClass, BidiInfo, Level};
use unicode_linebreak::{linebreaks, BreakOpportunity};

use crate::measure::{Fnv, FontId, Measure, Measurer, Metrics};
use crate::style::TextStyle;

/// A style's number in a [`Styles`] table.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StyleId(pub u32);

/// The text styles a view has used, numbered so layouts and display lists can refer to them.
#[derive(Default)]
pub struct Styles {
    list: Vec<(TextStyle, FontId)>,
    index: HashMap<u64, Vec<u32>>,
}

impl Styles {
    pub fn new() -> Styles {
        Styles::default()
    }

    pub fn add(&mut self, measure: &mut Measure, style: TextStyle) -> StyleId {
        let key = style_hash(&style);
        if let Some(ids) = self.index.get(&key) {
            if let Some(id) = ids.iter().find(|&&i| self.list[i as usize].0 == style) {
                return StyleId(*id);
            }
        }
        let id = self.list.len() as u32;
        let font = measure.font_id(&style.font);
        self.list.push((style, font));
        self.index.entry(key).or_default().push(id);
        StyleId(id)
    }

    pub fn get(&self, id: StyleId) -> &TextStyle {
        &self.list[id.0 as usize].0
    }

    pub fn font(&self, id: StyleId) -> FontId {
        self.list[id.0 as usize].1
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &TextStyle> {
        self.list.iter().map(|(s, _)| s)
    }
}

fn style_hash(s: &TextStyle) -> u64 {
    let mut h = Fnv(0xcbf2_9ce4_8422_2325);
    s.font.hash(&mut h);
    (s.color.rgb, s.color.alpha.to_bits(), s.highlight.map(|c| (c.rgb, c.alpha.to_bits()))).hash(&mut h);
    (&s.underline, s.strike.map(|x| x as u8), s.baseline.map(|x| x as u8), s.caps, s.small_caps, s.link).hash(&mut h);
    h.finish()
}

/// What a stretch of a paragraph's text is.
#[derive(Clone, Debug, PartialEq)]
pub enum ItemKind {
    Text(StyleId),
    /// A tab character.
    Tab(StyleId),
    /// A line break (`\n`).
    Break(StyleId),
    /// A page break: ends the line, and the layer starts a new page after it.
    PageBreak,
    ColumnBreak,
    /// A picture or other box sitting on the baseline. `inline` is its index in the model
    /// paragraph's content, for drawing it.
    Object {
        width: f64,
        height: f64,
        inline: u32,
    },
    /// Something drawn as text but edited as one character: a field's result.
    Atomic {
        text: String,
        style: StyleId,
        inline: u32,
    },
    /// Kept in the text but not drawn (markup with nothing to show: a bookmark).
    Hidden,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub start: u32,
    pub end: u32,
    pub kind: ItemKind,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Spacing {
    /// A multiple of the line's natural height.
    Multiple(f64),
    Exactly(f64),
    AtLeast(f64),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TabKind {
    Left,
    Center,
    Right,
    Decimal,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Tab {
    pub pos: f64,
    pub kind: TabKind,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
    Justify,
}

/// A list label (a number or a bullet) drawn in front of the first line.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub text: String,
    pub style: StyleId,
}

/// A paragraph to lay out. Lengths in pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct ParaSpec {
    pub text: String,
    pub items: Vec<Item>,
    /// Left, center, right or justified, as seen on the page (a layer whose format says "start"
    /// and "end" turns them into sides).
    pub align: Align,
    /// A right-to-left paragraph: lines start at the right, indents and tab stops count from
    /// the right edge, and text runs in the Unicode bidirectional algorithm's right-to-left
    /// order. Left-to-right paragraphs reorder right-to-left text inside them too.
    pub rtl: bool,
    /// From the start edge: the left in a left-to-right paragraph, the right in a right-to-left one.
    pub indent_start: f64,
    pub indent_end: f64,
    /// Extra indent of the first line; negative hangs it.
    pub first_line: f64,
    pub space_before: f64,
    pub space_after: f64,
    pub spacing: Spacing,
    pub tabs: Vec<Tab>,
    pub default_tab: f64,
    pub label: Option<Label>,
    /// The paragraph mark's style: how tall an empty paragraph is.
    pub mark: StyleId,
    /// Single spacing includes the font's line gap (Word's does; PowerPoint's doesn't).
    pub line_gap: bool,
}

impl ParaSpec {
    /// A paragraph of plain text in one style, left aligned and single spaced.
    pub fn plain(text: &str, style: StyleId) -> ParaSpec {
        let mut items = Vec::new();
        let mut start = 0;
        for (i, c) in text.char_indices() {
            let kind = match c {
                '\t' => ItemKind::Tab(style),
                '\n' => ItemKind::Break(style),
                _ => continue,
            };
            if i > start {
                items.push(Item { start: start as u32, end: i as u32, kind: ItemKind::Text(style) });
            }
            items.push(Item { start: i as u32, end: i as u32 + 1, kind });
            start = i + 1;
        }
        if text.len() > start {
            items.push(Item { start: start as u32, end: text.len() as u32, kind: ItemKind::Text(style) });
        }
        ParaSpec {
            text: text.to_string(),
            items,
            align: Align::Left,
            rtl: false,
            indent_start: 0.0,
            indent_end: 0.0,
            first_line: 0.0,
            space_before: 0.0,
            space_after: 0.0,
            spacing: Spacing::Multiple(1.0),
            tabs: Vec::new(),
            default_tab: 48.0,
            label: None,
            mark: style,
            line_gap: true,
        }
    }

    /// A hash of everything that decides the layout, for caching layouts across edits.
    pub fn key(&self, width: f64) -> u64 {
        let mut h = Fnv(0xcbf2_9ce4_8422_2325);
        self.text.hash(&mut h);
        for it in &self.items {
            (it.start, it.end).hash(&mut h);
            match &it.kind {
                ItemKind::Text(s) => (0u8, s).hash(&mut h),
                ItemKind::Tab(s) => (1u8, s).hash(&mut h),
                ItemKind::Break(s) => (2u8, s).hash(&mut h),
                ItemKind::PageBreak => 3u8.hash(&mut h),
                ItemKind::ColumnBreak => 4u8.hash(&mut h),
                ItemKind::Object { width, height, inline } => (5u8, width.to_bits(), height.to_bits(), inline).hash(&mut h),
                ItemKind::Atomic { text, style, inline } => (6u8, text, style, inline).hash(&mut h),
                ItemKind::Hidden => 7u8.hash(&mut h),
            }
        }
        (self.align as u8, self.indent_start.to_bits(), self.indent_end.to_bits(), self.first_line.to_bits()).hash(&mut h);
        (self.space_before.to_bits(), self.space_after.to_bits(), self.default_tab.to_bits(), width.to_bits()).hash(&mut h);
        match self.spacing {
            Spacing::Multiple(v) => (0u8, v.to_bits()).hash(&mut h),
            Spacing::Exactly(v) => (1u8, v.to_bits()).hash(&mut h),
            Spacing::AtLeast(v) => (2u8, v.to_bits()).hash(&mut h),
        }
        for t in &self.tabs {
            (t.pos.to_bits(), t.kind as u8).hash(&mut h);
        }
        self.label.as_ref().map(|l| (&l.text, l.style)).hash(&mut h);
        (self.mark, self.line_gap, self.rtl).hash(&mut h);
        h.finish()
    }

    fn item_at(&self, offset: u32) -> Option<&Item> {
        let i = self.items.partition_point(|it| it.end <= offset);
        self.items.get(i).filter(|it| it.start <= offset)
    }
}

/// How a line ends.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LineEnd {
    /// The text ran on past the width.
    Wrap,
    /// A line break.
    Break,
    PageBreak,
    ColumnBreak,
    /// The end of the paragraph.
    Para,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FragKind {
    Text(StyleId),
    Tab(StyleId),
    Object { inline: u32, height: f64 },
    Atomic { text: String, style: StyleId, inline: u32 },
    Label { text: String, style: StyleId },
}

/// A piece of a line: a stretch of text in one style, a tab, a picture, a label.
#[derive(Clone, Debug, PartialEq)]
pub struct Frag {
    pub x: f64,
    pub width: f64,
    pub start: u32,
    pub end: u32,
    pub kind: FragKind,
    /// Extra space after each space character, from justifying the line.
    pub space: f64,
    /// Runs right to left: its text starts at `x + width`. Set for text at an odd bidi
    /// level, and for tabs and labels in a right-to-left paragraph.
    pub rtl: bool,
}

impl Frag {
    /// The x where the frag's first character starts.
    pub fn start_x(&self) -> f64 {
        if self.rtl {
            self.x + self.width
        } else {
            self.x
        }
    }

    /// The x where the frag's last character ends.
    pub fn end_x(&self) -> f64 {
        if self.rtl {
            self.x
        } else {
            self.x + self.width
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// From the top of the paragraph's first line (space before isn't counted).
    pub top: f64,
    pub height: f64,
    /// From the line's top.
    pub baseline: f64,
    pub start: u32,
    pub end: u32,
    pub frags: Vec<Frag>,
    pub ends: LineEnd,
    /// Caret x when the line has no text, after indents, a list label and alignment.
    pub origin: f64,
    /// Frags are in text order. With mixed directions their x order differs; this says so.
    pub bidi: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParaLayout {
    pub width: f64,
    pub lines: Vec<Line>,
    pub space_before: f64,
    pub space_after: f64,
    /// The lines' height, without the space before and after.
    pub height: f64,
}

/// What laying out needs: the host's measurer, the engine's cache in front of it, and the styles.
pub struct Ctx<'a> {
    pub host: &'a mut dyn Measurer,
    pub measure: &'a mut Measure,
    pub styles: &'a mut Styles,
}

impl Ctx<'_> {
    fn width(&mut self, style: StyleId, text: &str) -> f64 {
        let s = self.styles.get(style);
        let font = self.styles.font(style);
        if s.caps || s.small_caps {
            let upper = s.transform(text).into_owned();
            self.measure.width(self.host, font, &upper)
        } else {
            self.measure.width(self.host, font, text)
        }
    }

    fn metrics(&mut self, style: StyleId) -> (Metrics, f64) {
        let s = self.styles.get(style);
        let shift = s.baseline_shift();
        let font = self.styles.font(style);
        let m = self.measure.metrics(self.host, font);
        if s.baseline.is_some() {
            // Raised or lowered text sits inside the line its full size would make.
            let scale = 1.0 / crate::style::SCRIPT_SCALE;
            let full = Metrics { ascent: m.ascent * scale, descent: m.descent * scale, gap: m.gap * scale };
            let lifted = Metrics { ascent: m.ascent + shift, descent: m.descent - shift, gap: m.gap };
            (Metrics { ascent: full.ascent.max(lifted.ascent), descent: full.descent.max(lifted.descent), gap: m.gap * scale }, shift)
        } else {
            (m, 0.0)
        }
    }

    /// Width of `text` from `start` within an item of this kind; for tabs and objects their laid
    /// out width counts as a whole.
    pub fn text_width(&mut self, style: StyleId, text: &str) -> f64 {
        self.width(style, text)
    }
}

/// A piece of text that can't be broken inside, with what follows it.
#[derive(Clone, Debug)]
struct Piece {
    start: u32,
    end: u32,
    /// Without trailing spaces.
    width: f64,
    /// Of trailing spaces, which may hang past the line's end.
    trailing: f64,
    kind: PieceKind,
    /// A line may break after this piece.
    break_after: bool,
}

#[derive(Clone, Debug)]
enum PieceKind {
    Text(StyleId),
    Tab(StyleId),
    Object { inline: u32, height: f64 },
    Atomic { text: String, style: StyleId, inline: u32 },
    End(LineEnd, Option<StyleId>),
    Hidden,
}

/// Lays `spec` out across `width` pixels.
pub fn layout(ctx: &mut Ctx, spec: &ParaSpec, width: f64) -> ParaLayout {
    let pieces = pieces(ctx, spec);
    let levels = bidi_levels(&spec.text, spec.rtl);
    let mut b = Builder {
        ctx,
        spec,
        width,
        lines: Vec::new(),
        frags: Vec::new(),
        x: 0.0,
        line_start: 0,
        top: 0.0,
        pending_tabs: Vec::new(),
        levels,
    };
    b.start_line(true);
    let mut i = 0;
    while i < pieces.len() {
        // The run of pieces up to the next break opportunity is a word: it moves to the next
        // line whole unless it doesn't fit on one by itself.
        let mut j = i;
        while !pieces[j].break_after && j + 1 < pieces.len() && !matches!(pieces[j + 1].kind, PieceKind::End(..) | PieceKind::Tab(_)) {
            j += 1;
        }
        if let PieceKind::End(end, style) = pieces[i].kind {
            b.finish_line(pieces[i].start, pieces[i].end, end, style);
            if end != LineEnd::Para {
                b.start_line(false);
            }
            i += 1;
            continue;
        }
        if let PieceKind::Tab(style) = pieces[i].kind {
            b.place_tab(&pieces[i], style);
            i += 1;
            continue;
        }
        let word = &pieces[i..=j];
        let word_width: f64 = word.iter().take(word.len() - 1).map(|p| p.width + p.trailing).sum::<f64>() + word[word.len() - 1].width;
        let limit = b.right();
        if b.x + word_width > limit + 0.01 && b.has_content() {
            let at = word[0].start;
            b.finish_line(at, at, LineEnd::Wrap, None);
            b.start_line(false);
        }
        if b.x + word_width > b.right() + 0.01 && !b.has_content() {
            // Longer than a whole line: break it between characters.
            for p in word {
                b.place_split(p);
            }
        } else {
            for p in word {
                b.place(p);
            }
        }
        i = j + 1;
    }
    let height = b.lines.last().map(|l| l.top + l.height).unwrap_or(0.0);
    ParaLayout { width, lines: b.lines, space_before: spec.space_before, space_after: spec.space_after, height }
}

fn pieces(ctx: &mut Ctx, spec: &ParaSpec) -> Vec<Piece> {
    let text = spec.text.as_str();
    let breaks: Vec<u32> = linebreaks(text)
        .filter(|(_, o)| *o == BreakOpportunity::Allowed || *o == BreakOpportunity::Mandatory)
        .map(|(i, _)| i as u32)
        .collect();
    let can_break = |at: u32| breaks.binary_search(&at).is_ok();
    let mut out = Vec::with_capacity(spec.items.len() * 4);
    for item in &spec.items {
        match &item.kind {
            ItemKind::Text(style) => {
                let lo = breaks.partition_point(|&b| b <= item.start);
                let mut start = item.start;
                let mut cuts: Vec<u32> = breaks[lo..].iter().copied().take_while(|&b| b < item.end).collect();
                cuts.push(item.end);
                for cut in cuts {
                    if cut <= start {
                        continue;
                    }
                    let seg = &text[start as usize..cut as usize];
                    let body = seg.trim_end_matches(' ');
                    let spaces = seg.len() - body.len();
                    let width = ctx.width(*style, body);
                    let trailing = if spaces > 0 { ctx.width(*style, " ") * spaces as f64 } else { 0.0 };
                    out.push(Piece { start, end: cut, width, trailing, kind: PieceKind::Text(*style), break_after: can_break(cut) });
                    start = cut;
                }
            }
            ItemKind::Tab(style) => out.push(Piece {
                start: item.start,
                end: item.end,
                width: 0.0,
                trailing: 0.0,
                kind: PieceKind::Tab(*style),
                break_after: true,
            }),
            ItemKind::Break(style) => out.push(Piece {
                start: item.start,
                end: item.end,
                width: 0.0,
                trailing: 0.0,
                kind: PieceKind::End(LineEnd::Break, Some(*style)),
                break_after: true,
            }),
            ItemKind::PageBreak | ItemKind::ColumnBreak => {
                let end = if item.kind == ItemKind::PageBreak { LineEnd::PageBreak } else { LineEnd::ColumnBreak };
                out.push(Piece {
                    start: item.start,
                    end: item.end,
                    width: 0.0,
                    trailing: 0.0,
                    kind: PieceKind::End(end, None),
                    break_after: true,
                })
            }
            ItemKind::Object { width, height, inline } => out.push(Piece {
                start: item.start,
                end: item.end,
                width: *width,
                trailing: 0.0,
                kind: PieceKind::Object { inline: *inline, height: *height },
                break_after: true,
            }),
            ItemKind::Atomic { text: shown, style, inline } => {
                let width = ctx.width(*style, shown);
                out.push(Piece {
                    start: item.start,
                    end: item.end,
                    width,
                    trailing: 0.0,
                    kind: PieceKind::Atomic { text: shown.clone(), style: *style, inline: *inline },
                    break_after: can_break(item.end),
                })
            }
            ItemKind::Hidden => out.push(Piece {
                start: item.start,
                end: item.end,
                width: 0.0,
                trailing: 0.0,
                kind: PieceKind::Hidden,
                break_after: can_break(item.end),
            }),
        }
    }
    let end = text.len() as u32;
    out.push(Piece { start: end, end, width: 0.0, trailing: 0.0, kind: PieceKind::End(LineEnd::Para, None), break_after: true });
    out
}

struct Builder<'a, 'b, 'c> {
    ctx: &'a mut Ctx<'b>,
    spec: &'c ParaSpec,
    width: f64,
    lines: Vec<Line>,
    frags: Vec<Frag>,
    x: f64,
    line_start: u32,
    top: f64,
    /// Center, right and decimal tabs on this line: (frag index, stop, kind).
    pending_tabs: Vec<(usize, f64, TabKind)>,
    /// Each byte's bidi embedding level, when the paragraph has any right-to-left text or
    /// direction (`None` for plain left-to-right text, which needs no reordering).
    levels: Option<Vec<u8>>,
}

/// The paragraph's bidi embedding levels by byte, or `None` when it's left to right with
/// nothing right to left in it.
fn bidi_levels(text: &str, rtl: bool) -> Option<Vec<u8>> {
    if !rtl
        && !text.chars().any(|c| {
            matches!(bidi_class(c), BidiClass::R | BidiClass::AL | BidiClass::AN | BidiClass::RLE | BidiClass::RLO | BidiClass::RLI)
        })
    {
        return None;
    }
    let base = if rtl { Level::rtl() } else { Level::ltr() };
    let info = BidiInfo::new(text, Some(base));
    Some(info.levels.iter().map(|l| l.number()).collect())
}

/// Whether the first letter in `s` that has a direction is right to left (`None` with none).
pub fn first_strong_rtl(s: &str) -> Option<bool> {
    s.chars().find_map(|c| match bidi_class(c) {
        BidiClass::L => Some(false),
        BidiClass::R | BidiClass::AL => Some(true),
        _ => None,
    })
}

/// The order to show `levels`' runs in, left to right (rule L2 of the bidi algorithm).
fn visual_order(levels: &[u8]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..levels.len()).collect();
    let (Some(&max), Some(&min)) = (levels.iter().max(), levels.iter().min()) else { return order };
    let lowest_odd = min | 1;
    let mut level = max;
    while level >= lowest_odd {
        let mut i = 0;
        while i < order.len() {
            if levels[order[i]] >= level {
                let mut j = i;
                while j < order.len() && levels[order[j]] >= level {
                    j += 1;
                }
                order[i..j].reverse();
                i = j;
            } else {
                i += 1;
            }
        }
        level -= 1;
    }
    order
}

impl Builder<'_, '_, '_> {
    fn left(&self, first: bool) -> f64 {
        self.spec.indent_start + if first { self.spec.first_line } else { 0.0 }
    }

    fn right(&self) -> f64 {
        (self.width - self.spec.indent_end).max(self.x.min(self.width))
    }

    fn has_content(&self) -> bool {
        self.frags.iter().any(|f| !matches!(f.kind, FragKind::Label { .. }))
    }

    fn start_line(&mut self, first: bool) {
        self.frags.clear();
        self.pending_tabs.clear();
        self.x = self.left(first);
        if first {
            if let Some(label) = &self.spec.label {
                // The label hangs at the first line's indent; the text starts at the paragraph's
                // indent, or the next tab stop if the label runs past it (Word's tab after the
                // number).
                let w = self.ctx.width(label.style, &label.text);
                let at = self.x;
                self.frags.push(Frag {
                    x: at,
                    width: w,
                    start: 0,
                    end: 0,
                    kind: FragKind::Label { text: label.text.clone(), style: label.style },
                    space: 0.0,
                    rtl: false,
                });
                let after = at + w;
                self.x = if after < self.spec.indent_start - 0.5 { self.spec.indent_start } else { self.next_stop(after).0 };
            }
        }
    }

    /// The next tab stop after `x`: a set stop, or the next default one.
    fn next_stop(&self, x: f64) -> (f64, TabKind) {
        if let Some(t) = self.spec.tabs.iter().find(|t| t.pos > x + 0.5) {
            return (t.pos, t.kind);
        }
        let step = if self.spec.default_tab > 1.0 { self.spec.default_tab } else { 48.0 };
        (((x + 0.5) / step).floor() * step + step, TabKind::Left)
    }

    fn place(&mut self, p: &Piece) {
        let (kind, width) = match &p.kind {
            PieceKind::Text(s) => (FragKind::Text(*s), p.width + p.trailing),
            PieceKind::Object { inline, height } => (FragKind::Object { inline: *inline, height: *height }, p.width),
            PieceKind::Atomic { text, style, inline } => (FragKind::Atomic { text: text.clone(), style: *style, inline: *inline }, p.width),
            PieceKind::Hidden => return,
            PieceKind::Tab(_) | PieceKind::End(..) => unreachable!(),
        };
        // Text in the same style as the last fragment joins it.
        if let (FragKind::Text(s), Some(last)) = (&kind, self.frags.last_mut()) {
            if last.kind == FragKind::Text(*s) && last.end == p.start {
                last.end = p.end;
                last.width += width;
                self.x += width;
                return;
            }
        }
        self.frags.push(Frag { x: self.x, width, start: p.start, end: p.end, kind, space: 0.0, rtl: false });
        self.x += width;
    }

    /// Places a piece too long for a line, character by character, breaking lines as it fills.
    fn place_split(&mut self, p: &Piece) {
        let PieceKind::Text(style) = p.kind else {
            if self.x + p.width > self.right() && self.has_content() {
                self.finish_line(p.start, p.start, LineEnd::Wrap, None);
                self.start_line(false);
            }
            return self.place(p);
        };
        let text = &self.spec.text[p.start as usize..p.end as usize];
        let mut seg_start = p.start;
        let mut iter = text.char_indices().peekable();
        while let Some((i, _)) = iter.next() {
            let next = iter.peek().map(|(n, _)| *n).unwrap_or(text.len());
            let upto = &self.spec.text[seg_start as usize..p.start as usize + next];
            let w = self.ctx.width(style, upto);
            if self.x + w > self.right() + 0.01 && (p.start as usize + i) as u32 > seg_start {
                let at = p.start + i as u32;
                let done = &self.spec.text[seg_start as usize..at as usize];
                let dw = self.ctx.width(style, done);
                self.place(&Piece {
                    start: seg_start,
                    end: at,
                    width: dw,
                    trailing: 0.0,
                    kind: PieceKind::Text(style),
                    break_after: false,
                });
                self.finish_line(at, at, LineEnd::Wrap, None);
                self.start_line(false);
                seg_start = at;
            }
        }
        let rest = &self.spec.text[seg_start as usize..p.end as usize];
        let body = rest.trim_end_matches(' ');
        let w = self.ctx.width(style, body);
        let trailing = if rest.len() > body.len() { self.ctx.width(style, " ") * (rest.len() - body.len()) as f64 } else { 0.0 };
        self.place(&Piece { start: seg_start, end: p.end, width: w, trailing, kind: PieceKind::Text(style), break_after: p.break_after });
    }

    fn place_tab(&mut self, p: &Piece, style: StyleId) {
        let (stop, kind) = self.next_stop(self.x);
        if stop > self.right() + 0.5 && self.has_content() {
            // No stop left on the line: the tab moves to the next line, like a word.
            self.finish_line(p.start, p.start, LineEnd::Wrap, None);
            self.start_line(false);
            return self.place_tab(p, style);
        }
        self.resolve_tabs();
        let width = if kind == TabKind::Left { (stop - self.x).max(0.0) } else { 0.0 };
        if kind != TabKind::Left {
            self.pending_tabs.push((self.frags.len(), stop, kind));
        }
        self.frags.push(Frag { x: self.x, width, start: p.start, end: p.end, kind: FragKind::Tab(style), space: 0.0, rtl: false });
        self.x += width;
    }

    /// Sizes the center, right and decimal tabs waiting on the text after them.
    fn resolve_tabs(&mut self) {
        for (index, stop, kind) in std::mem::take(&mut self.pending_tabs) {
            let after: f64 = self.frags[index + 1..].iter().map(|f| f.width).sum();
            let tab_x = self.frags[index].x;
            let want = match kind {
                TabKind::Center => stop - after / 2.0,
                _ => stop - after,
            };
            let width = (want - tab_x).max(0.0);
            self.frags[index].width = width;
            let mut x = tab_x + width;
            for f in &mut self.frags[index + 1..] {
                f.x = x;
                x += f.width;
            }
            self.x = x;
        }
    }

    /// Puts a finished line's frags in bidi order, then mirrors a right-to-left paragraph's
    /// line, whose frags were placed from the start edge. Text whose direction changes is split
    /// into a frag per run. Tabs and the label stay where they are: each stretch between them is
    /// reordered on its own. Says whether the line needed any of this.
    fn reorder(&mut self, frags: &mut Vec<Frag>, from: u32, to: u32) -> bool {
        let rtl = self.spec.rtl;
        let Some(levels) = &self.levels else { return false };
        let text = self.spec.text.as_str();
        let to = to.min(text.len() as u32).max(from);
        // The line's levels, with rule L1: separators, and spaces before them or at the end of
        // the line, go back to the paragraph's level.
        let base = rtl as u8;
        let mut line: Vec<u8> = levels[from as usize..to as usize].to_vec();
        let mut trailing = true;
        for (i, c) in text[from as usize..to as usize].char_indices().rev() {
            let class = bidi_class(c);
            let reset = match class {
                BidiClass::S | BidiClass::B => {
                    trailing = true;
                    true
                }
                BidiClass::WS
                | BidiClass::FSI
                | BidiClass::LRI
                | BidiClass::RLI
                | BidiClass::PDI
                | BidiClass::BN
                | BidiClass::LRE
                | BidiClass::RLE
                | BidiClass::LRO
                | BidiClass::RLO
                | BidiClass::PDF => trailing,
                _ => {
                    trailing = false;
                    false
                }
            };
            if reset {
                line[i..i + c.len_utf8()].fill(base);
            }
        }
        let level_at = |offset: u32| -> u8 { line.get(offset.saturating_sub(from) as usize).copied().unwrap_or(base) };

        // Split text frags where the level changes.
        let mut runs: Vec<(Frag, u8)> = Vec::with_capacity(frags.len());
        let last = frags.len().saturating_sub(1);
        for (index, f) in frags.drain(..).enumerate() {
            match f.kind {
                FragKind::Text(style) if f.end > f.start => {
                    let s = &text[f.start as usize..f.end as usize];
                    let mut cuts: Vec<(u32, u32, u8)> = Vec::new();
                    for (i, c) in s.char_indices() {
                        let at = f.start + i as u32;
                        let l = level_at(at);
                        match cuts.last_mut() {
                            Some(last) if last.2 == l => last.1 = at + c.len_utf8() as u32,
                            _ => cuts.push((at, at + c.len_utf8() as u32, l)),
                        }
                    }
                    if cuts.len() <= 1 {
                        let l = cuts.first().map(|c| c.2).unwrap_or(base);
                        runs.push((f, l));
                        continue;
                    }
                    for (a, b, l) in cuts {
                        let part = &text[a as usize..b as usize];
                        // Justifying stretched every space but those ending the line.
                        let mut spaces = part.matches(' ').count();
                        if index == last && b == f.end {
                            spaces -= part.len() - part.trim_end_matches(' ').len();
                        }
                        let width = self.ctx.width(style, part) + spaces as f64 * f.space;
                        // Where the frag started, so the stretch keeps its place on the line.
                        runs.push((Frag { x: f.x, width, start: a, end: b, kind: f.kind.clone(), space: f.space, rtl: false }, l));
                    }
                }
                FragKind::Tab(_) | FragKind::Label { .. } => runs.push((f, base)),
                _ => {
                    let l = level_at(f.start);
                    runs.push((f, l));
                }
            }
        }

        // Lay each stretch between tabs out again in visual order, from the start edge.
        let mut mixed = rtl;
        let mut i = 0;
        while i < runs.len() {
            if matches!(runs[i].0.kind, FragKind::Tab(_) | FragKind::Label { .. }) {
                i += 1;
                continue;
            }
            let mut j = i;
            while j < runs.len() && !matches!(runs[j].0.kind, FragKind::Tab(_) | FragKind::Label { .. }) {
                j += 1;
            }
            let stretch = &mut runs[i..j];
            let levels: Vec<u8> = stretch.iter().map(|r| r.1).collect();
            if levels.iter().any(|&l| l != base) {
                mixed = true;
                let mut order = visual_order(&levels);
                if rtl {
                    // From the start edge, which is the right.
                    order.reverse();
                }
                let mut x = stretch[0].0.x;
                let mut placed = vec![0.0; stretch.len()];
                for &k in &order {
                    placed[k] = x;
                    x += stretch[k].0.width;
                }
                for (k, r) in stretch.iter_mut().enumerate() {
                    r.0.x = placed[k];
                }
            }
            i = j;
        }
        for (mut f, l) in runs {
            f.rtl = l % 2 == 1;
            if rtl {
                f.x = self.width - f.x - f.width;
            }
            frags.push(f);
        }
        mixed
    }

    fn finish_line(&mut self, end_start: u32, end_end: u32, ends: LineEnd, break_style: Option<StyleId>) {
        self.resolve_tabs();
        let first = self.lines.is_empty();
        let mut frags = std::mem::take(&mut self.frags);
        // Trailing spaces hang past the end: they don't count for alignment.
        let mut content_end = self.x;
        if let Some(last) = frags.iter().rev().find(|f| !matches!(f.kind, FragKind::Label { .. })) {
            if let FragKind::Text(s) = last.kind {
                let text = &self.spec.text[last.start as usize..last.end as usize];
                let spaces = text.len() - text.trim_end_matches(' ').len();
                if spaces > 0 {
                    content_end = last.x + last.width - self.ctx.width(s, " ") * spaces as f64;
                }
            }
        }
        let left = self.left(first);
        let free = (self.width - self.spec.indent_end - content_end).max(0.0);
        // Lines are laid out from the start edge; a right-to-left paragraph is mirrored after,
        // so its right alignment is no shift at all.
        let shift = match (self.spec.align, self.spec.rtl) {
            (Align::Center, _) => free / 2.0,
            (Align::Right, false) | (Align::Left, true) => free,
            _ => 0.0,
        };
        if self.spec.align == Align::Justify && ends == LineEnd::Wrap && free > 0.0 {
            let stretch_from = frags.iter().rposition(|f| matches!(f.kind, FragKind::Tab(_))).map(|i| i + 1).unwrap_or(0);
            let count = |f: &Frag| -> usize {
                let text = &self.spec.text[f.start as usize..f.end as usize];
                match f.kind {
                    FragKind::Text(_) => text.matches(' ').count(),
                    _ => 0,
                }
            };
            let last_index = frags.len().saturating_sub(1);
            let mut spaces: usize = frags[stretch_from..].iter().map(count).sum();
            if let Some(last) = frags.get(last_index) {
                let text = &self.spec.text[last.start as usize..last.end as usize];
                spaces -= text.len() - text.trim_end_matches(' ').len();
            }
            if spaces > 0 {
                let each = free / spaces as f64;
                let mut add = 0.0;
                for (i, f) in frags.iter_mut().enumerate().skip(stretch_from) {
                    f.x += add;
                    if let FragKind::Text(_) = f.kind {
                        let text = &self.spec.text[f.start as usize..f.end as usize];
                        let mut n = text.matches(' ').count();
                        if i == last_index {
                            n -= text.len() - text.trim_end_matches(' ').len();
                        }
                        f.space = each;
                        f.width += each * n as f64;
                        add += each * n as f64;
                    }
                }
            }
        } else if shift > 0.0 {
            for f in &mut frags {
                f.x += shift;
            }
        }
        let _ = left;
        let bidi = self.reorder(&mut frags, self.line_start, if ends == LineEnd::Wrap { end_start } else { end_end });

        // Height: the tallest text and objects on the line, then line spacing.
        let mut ascent: f64 = 0.0;
        let mut descent: f64 = 0.0;
        let mut gap: f64 = 0.0;
        let mut any = false;
        for f in &frags {
            match &f.kind {
                FragKind::Text(s) | FragKind::Tab(s) | FragKind::Atomic { style: s, .. } | FragKind::Label { style: s, .. } => {
                    let (m, _) = self.ctx.metrics(*s);
                    ascent = ascent.max(m.ascent);
                    descent = descent.max(m.descent);
                    gap = gap.max(m.gap);
                    any = true;
                }
                FragKind::Object { height, .. } => {
                    ascent = ascent.max(*height);
                    any = true;
                }
            }
        }
        if !any || frags.iter().all(|f| matches!(f.kind, FragKind::Object { .. })) {
            let (m, _) = self.ctx.metrics(break_style.unwrap_or(self.spec.mark));
            if !any {
                ascent = m.ascent;
            }
            descent = descent.max(m.descent);
            gap = gap.max(m.gap);
        }
        // The line gap goes above the text, as Word puts it.
        let natural = ascent + descent + if self.spec.line_gap { gap } else { 0.0 };
        let (height, baseline) = match self.spec.spacing {
            Spacing::Multiple(m) => {
                let h = natural * m;
                (h, h - descent)
            }
            Spacing::Exactly(h) => (h, if natural > 0.0 { h * ascent / natural } else { h * 0.8 }),
            Spacing::AtLeast(h) => {
                let h = h.max(natural);
                (h, h - descent)
            }
        };
        let start = self.line_start;
        let end = if ends == LineEnd::Wrap { end_start } else { end_end };
        let _ = end_start;
        // `self.x` is where the next glyph would be placed, before the alignment shift.
        // An empty line has no fragment to move, so the caret has to remember it.
        let mut origin = self.x + shift;
        if self.spec.rtl {
            origin = self.width - origin;
        }
        self.lines.push(Line { top: self.top, height, baseline, start, end, frags, ends, origin, bidi });
        self.top += height;
        self.line_start = end;
    }
}

// ---- positions in a laid-out paragraph ----

impl ParaLayout {
    /// The line an offset is on. An offset where a line wraps belongs to the next line, unless
    /// `before` asks for the end of the earlier one (the caret after the last character typed).
    pub fn line_of(&self, offset: u32, before: bool) -> usize {
        for (i, l) in self.lines.iter().enumerate() {
            let last = i + 1 == self.lines.len();
            if offset < l.end || last || (offset == l.end && (before || l.ends != LineEnd::Wrap)) {
                if offset == l.end && l.ends != LineEnd::Wrap && l.ends != LineEnd::Para && !before && !last {
                    // Right after a line break: the next line.
                    return i + 1;
                }
                return i;
            }
        }
        0
    }

    /// Where the caret for `offset` stands: x from the paragraph's left, and the line.
    pub fn caret(&self, ctx: &mut Ctx, spec: &ParaSpec, offset: u32, before: bool) -> (f64, usize) {
        let li = self.line_of(offset, before);
        let line = &self.lines[li];
        (x_in_line(ctx, spec, line, offset), li)
    }

    /// The offset nearest `x` on line `li`.
    pub fn offset_at(&self, ctx: &mut Ctx, spec: &ParaSpec, li: usize, x: f64) -> u32 {
        let line = &self.lines[li.min(self.lines.len() - 1)];
        let frags: Vec<&Frag> = line.frags.iter().filter(|f| !matches!(f.kind, FragKind::Label { .. })).collect();
        // The end of a wrapped line is the start of the next; a click past it goes before its
        // trailing space instead.
        let line_end = if line.ends == LineEnd::Wrap {
            let text = &spec.text[line.start as usize..line.end as usize];
            let trimmed = text.trim_end_matches(' ').len();
            if trimmed < text.len() {
                line.start + trimmed as u32
            } else {
                prev_boundary(&spec.text, line.end).max(line.start)
            }
        } else {
            line.end
        };
        let Some(first) = frags.first() else { return line.start };
        if !line.bidi {
            if x <= first.x {
                return first.start;
            }
            for f in &frags {
                if x >= f.x + f.width {
                    continue;
                }
                return match f.kind {
                    FragKind::Text(_) => {
                        let mut best = f.start;
                        let mut prev_x = f.x;
                        let text = &spec.text[f.start as usize..f.end as usize];
                        for (i, c) in text.char_indices() {
                            let next = f.start + (i + c.len_utf8()) as u32;
                            let nx = x_in_line(ctx, spec, line, next);
                            if x < (prev_x + nx) / 2.0 {
                                return best;
                            }
                            best = next;
                            prev_x = nx;
                        }
                        best.min(line_end)
                    }
                    _ => {
                        if x < f.x + f.width / 2.0 {
                            f.start
                        } else {
                            f.end.min(line_end)
                        }
                    }
                };
            }
            return line_end;
        }
        // Mixed directions: the frag under x (or the nearest), then its nearest boundary. Past
        // the end of the line (the left of a right-to-left one) is the line's end.
        let left = frags.iter().map(|f| f.x).fold(f64::INFINITY, f64::min);
        let right = frags.iter().map(|f| f.x + f.width).fold(f64::NEG_INFINITY, f64::max);
        if (spec.rtl && x <= left) || (!spec.rtl && x >= right) {
            return line_end;
        }
        let distance = |f: &Frag| {
            if x < f.x {
                f.x - x
            } else if x > f.x + f.width {
                x - f.x - f.width
            } else {
                0.0
            }
        };
        let f = frags.iter().min_by(|a, b| distance(a).total_cmp(&distance(b))).copied().unwrap_or(first);
        let off = match f.kind {
            FragKind::Text(_) => {
                let text = &spec.text[f.start as usize..f.end as usize];
                let mut best = (f.start, (frag_x(ctx, spec, f, f.start) - x).abs());
                for (i, c) in text.char_indices() {
                    let next = f.start + (i + c.len_utf8()) as u32;
                    let d = (frag_x(ctx, spec, f, next) - x).abs();
                    if d < best.1 {
                        best = (next, d);
                    }
                }
                best.0
            }
            _ => {
                let before = if f.rtl { x > f.x + f.width / 2.0 } else { x < f.x + f.width / 2.0 };
                if before {
                    f.start
                } else {
                    f.end
                }
            }
        };
        off.min(line_end)
    }
}

/// The x of `offset` inside frag `f`, which holds it.
pub fn frag_x(ctx: &mut Ctx, spec: &ParaSpec, f: &Frag, offset: u32) -> f64 {
    if offset <= f.start {
        return f.start_x();
    }
    match f.kind {
        FragKind::Text(style) => {
            let part = &spec.text[f.start as usize..offset.min(f.end) as usize];
            let spaces = part.matches(' ').count() as f64;
            let advance = ctx.width(style, part) + spaces * f.space;
            if f.rtl {
                f.x + f.width - advance
            } else {
                f.x + advance
            }
        }
        _ => f.end_x(),
    }
}

/// Where text changes direction one offset shows in two places: after the character before it,
/// and before the one after it. The second, when there is one and it's elsewhere.
pub fn other_x_in_line(ctx: &mut Ctx, spec: &ParaSpec, line: &Line, offset: u32) -> Option<f64> {
    if !line.bidi {
        return None;
    }
    let f = line.frags.iter().find(|f| !matches!(f.kind, FragKind::Label { .. }) && f.start == offset && f.end > offset)?;
    let x = f.start_x();
    ((x - x_in_line(ctx, spec, line, offset)).abs() > 0.5).then_some(x)
}

/// The x of `offset` on `line`. Where two frags meet, the caret stands after the character
/// before it, as Word puts it.
pub fn x_in_line(ctx: &mut Ctx, spec: &ParaSpec, line: &Line, offset: u32) -> f64 {
    let mut last_x = None;
    for f in &line.frags {
        if matches!(f.kind, FragKind::Label { .. }) {
            continue;
        }
        if offset < f.start {
            return last_x.unwrap_or(f.start_x());
        }
        if offset <= f.end {
            return frag_x(ctx, spec, f, offset);
        }
        last_x = Some(f.end_x());
    }
    // No text on this line. `origin` is the aligned start, including a list label and indents.
    last_x.unwrap_or(line.origin)
}

/// The character boundary before `offset`.
pub fn prev_boundary(text: &str, offset: u32) -> u32 {
    let mut i = (offset as usize).min(text.len());
    if i == 0 {
        return 0;
    }
    i -= 1;
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i as u32
}

/// The character boundary after `offset`.
pub fn next_boundary(text: &str, offset: u32) -> u32 {
    let mut i = offset as usize + 1;
    if i >= text.len() {
        return text.len() as u32;
    }
    while i < text.len() && !text.is_char_boundary(i) {
        i += 1;
    }
    i as u32
}

/// The word around `offset`: letters and digits, or a run of anything else that isn't space.
pub fn word_at(text: &str, offset: u32) -> (u32, u32) {
    let kind = |c: char| {
        if in_word(c) || c == '_' {
            1
        } else if c.is_whitespace() {
            0
        } else {
            2
        }
    };
    let at = (offset as usize).min(text.len());
    let after = text[at..].chars().next();
    let before = text[..at].chars().next_back();
    let k = match (before.map(kind), after.map(kind)) {
        (_, Some(1)) | (Some(1), _) => 1,
        (_, Some(k)) if k != 0 => k,
        (Some(k), _) => k,
        _ => return (offset, offset),
    };
    let start = text[..at].char_indices().rev().take_while(|(_, c)| kind(*c) == k).last().map(|(i, _)| i).unwrap_or(at);
    let end = at + text[at..].chars().take_while(|c| kind(*c) == k).map(char::len_utf8).sum::<usize>();
    (start as u32, end as u32)
}

/// Where a word movement from `offset` lands: the start of the previous word, or the end of
/// the next, as Option-arrow (Ctrl-arrow) does.
pub fn word_move(text: &str, offset: u32, forward: bool) -> u32 {
    let at = (offset as usize).min(text.len());
    if forward {
        let rest = &text[at..];
        let skip_space: usize = rest.chars().take_while(|c| !c.is_alphanumeric()).map(char::len_utf8).sum();
        let word: usize = rest[skip_space..].chars().take_while(|c| in_word(*c)).map(char::len_utf8).sum();
        (at + skip_space + word) as u32
    } else {
        let head = &text[..at];
        let skip_space: usize = head.chars().rev().take_while(|c| !c.is_alphanumeric()).map(char::len_utf8).sum();
        let word: usize = head[..at - skip_space].chars().rev().take_while(|c| in_word(*c)).map(char::len_utf8).sum();
        (at - skip_space - word) as u32
    }
}

/// A character that's part of a word: letters and digits, apostrophes, the marks written over
/// and under letters (Hebrew points, Arabic vowels, accents), and Hebrew's geresh and gershayim
/// (צה״ל).
fn in_word(c: char) -> bool {
    c.is_alphanumeric()
        || matches!(c, '\'' | '’' | '\u{5F3}' | '\u{5F4}')
        || matches!(c as u32, 0x0300..=0x036F | 0x0591..=0x05C7 | 0x0610..=0x061A | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06ED)
}

impl Item {
    pub fn style(&self) -> Option<StyleId> {
        match &self.kind {
            ItemKind::Text(s) | ItemKind::Tab(s) | ItemKind::Break(s) => Some(*s),
            ItemKind::Atomic { style, .. } => Some(*style),
            _ => None,
        }
    }
}

impl ParaSpec {
    /// The style text typed at `offset` would show in, before any formatting is applied: the
    /// text before it, or the mark's in an empty paragraph.
    pub fn style_at(&self, offset: u32) -> StyleId {
        let before = if offset > 0 { self.item_at(offset - 1).and_then(Item::style) } else { None };
        before.or_else(|| self.item_at(offset).and_then(Item::style)).unwrap_or(self.mark)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::measure::FixedMeasurer;
    use office_core::drawing::{Rgb, Rgba};
    use office_core::model::RunProps;
    use office_core::units::FontSize;

    fn setup() -> (FixedMeasurer, Measure, Styles, StyleId) {
        let mut measure = Measure::new();
        let mut styles = Styles::new();
        // 10 px text: 5 px per character, 2.5 per space; ascent 8, descent 2.
        let props = RunProps { size: Some(FontSize::from_points(7.5)), ..Default::default() };
        let s = styles.add(&mut measure, TextStyle::from_props(&props, 10.0, &|_| Rgba { rgb: Rgb::BLACK, alpha: 1.0 }));
        (FixedMeasurer::default(), measure, styles, s)
    }

    fn lines_text(spec: &ParaSpec, l: &ParaLayout) -> Vec<String> {
        l.lines.iter().map(|line| spec.text[line.start as usize..line.end as usize].to_string()).collect()
    }

    #[test]
    fn wraps_at_spaces() {
        let (mut host, mut measure, mut styles, s) = setup();
        let mut ctx = Ctx { host: &mut host, measure: &mut measure, styles: &mut styles };
        let spec = ParaSpec::plain("aaaa bbbb cccc dddd", s);
        // 4 letters = 20 px, a space 2.5: two words fit in 45 px.
        let l = layout(&mut ctx, &spec, 45.0);
        assert_eq!(lines_text(&spec, &l), vec!["aaaa bbbb ", "cccc dddd"]);
        assert_eq!(l.lines[0].height, 10.0);
        assert_eq!(l.lines[1].top, 10.0);
        assert_eq!(l.height, 20.0);
        assert_eq!(l.lines[0].ends, LineEnd::Wrap);
        assert_eq!(l.lines[1].ends, LineEnd::Para);
    }

    #[test]
    fn breaks_long_words_between_characters() {
        let (mut host, mut measure, mut styles, s) = setup();
        let mut ctx = Ctx { host: &mut host, measure: &mut measure, styles: &mut styles };
        let spec = ParaSpec::plain("abcdefghij", s);
        let l = layout(&mut ctx, &spec, 22.0);
        assert_eq!(lines_text(&spec, &l), vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn line_breaks_and_empty_paragraphs() {
        let (mut host, mut measure, mut styles, s) = setup();
        let mut ctx = Ctx { host: &mut host, measure: &mut measure, styles: &mut styles };
        let spec = ParaSpec::plain("one\ntwo", s);
        let l = layout(&mut ctx, &spec, 500.0);
        assert_eq!(lines_text(&spec, &l), vec!["one\n", "two"]);
        assert_eq!(l.lines[0].ends, LineEnd::Break);
        let empty = layout(&mut ctx, &ParaSpec::plain("", s), 500.0);
        assert_eq!(empty.lines.len(), 1);
        assert_eq!(empty.height, 10.0);
    }

    #[test]
    fn alignment_and_justification() {
        let (mut host, mut measure, mut styles, s) = setup();
        let mut ctx = Ctx { host: &mut host, measure: &mut measure, styles: &mut styles };
        let mut spec = ParaSpec::plain("ab cd", s);
        spec.align = Align::Right;
        let l = layout(&mut ctx, &spec, 100.0);
        // "ab cd" is 22.5 px wide.
        assert_eq!(l.lines[0].frags[0].x, 77.5);
        spec.align = Align::Center;
        let l = layout(&mut ctx, &spec, 100.0);
        assert_eq!(l.lines[0].frags[0].x, 38.75);

        let mut spec = ParaSpec::plain("aa bb cc dd ee", s);
        spec.align = Align::Justify;
        let l = layout(&mut ctx, &spec, 40.0);
        // First line "aa bb cc " (trailing space hangs): 35 px of content, 5 px spread over 2 spaces.
        let f = &l.lines[0].frags[0];
        assert_eq!(f.space, 2.5);
        // The last line isn't stretched.
        assert_eq!(l.lines.last().unwrap().frags[0].space, 0.0);
    }

    #[test]
    fn tabs_and_labels() {
        let (mut host, mut measure, mut styles, s) = setup();
        let mut ctx = Ctx { host: &mut host, measure: &mut measure, styles: &mut styles };
        let mut spec = ParaSpec::plain("a\tb", s);
        spec.default_tab = 48.0;
        let l = layout(&mut ctx, &spec, 500.0);
        assert_eq!(l.lines[0].frags[1].width, 43.0);
        assert_eq!(l.lines[0].frags[2].x, 48.0);

        spec.tabs = vec![Tab { pos: 200.0, kind: TabKind::Right }];
        let l = layout(&mut ctx, &spec, 500.0);
        let b = &l.lines[0].frags[2];
        assert_eq!(b.x + b.width, 200.0);

        let mut spec = ParaSpec::plain("item", s);
        spec.indent_start = 24.0;
        spec.first_line = -24.0;
        spec.label = Some(Label { text: "1.".into(), style: s });
        let l = layout(&mut ctx, &spec, 500.0);
        assert_eq!(l.lines[0].frags[0].x, 0.0);
        assert_eq!(l.lines[0].frags[1].x, 24.0);
    }

    #[test]
    fn spacing() {
        let (mut host, mut measure, mut styles, s) = setup();
        let mut ctx = Ctx { host: &mut host, measure: &mut measure, styles: &mut styles };
        let mut spec = ParaSpec::plain("x", s);
        spec.spacing = Spacing::Multiple(2.0);
        let l = layout(&mut ctx, &spec, 100.0);
        assert_eq!((l.lines[0].height, l.lines[0].baseline), (20.0, 18.0));
        spec.spacing = Spacing::Exactly(30.0);
        assert_eq!(layout(&mut ctx, &spec, 100.0).lines[0].height, 30.0);
        spec.spacing = Spacing::AtLeast(4.0);
        assert_eq!(layout(&mut ctx, &spec, 100.0).lines[0].height, 10.0);
    }

    #[test]
    fn objects_sit_on_the_baseline() {
        let (mut host, mut measure, mut styles, s) = setup();
        let mut ctx = Ctx { host: &mut host, measure: &mut measure, styles: &mut styles };
        let mut spec = ParaSpec::plain("a\u{FFFC}b", s);
        spec.items = vec![
            Item { start: 0, end: 1, kind: ItemKind::Text(s) },
            Item { start: 1, end: 4, kind: ItemKind::Object { width: 50.0, height: 40.0, inline: 1 } },
            Item { start: 4, end: 5, kind: ItemKind::Text(s) },
        ];
        let l = layout(&mut ctx, &spec, 500.0);
        assert_eq!(l.lines[0].height, 42.0);
        assert_eq!(l.lines[0].baseline, 40.0);
        assert_eq!(l.lines[0].frags[2].x, 55.0);
    }

    #[test]
    fn carets_and_hits() {
        let (mut host, mut measure, mut styles, s) = setup();
        let mut ctx = Ctx { host: &mut host, measure: &mut measure, styles: &mut styles };
        let spec = ParaSpec::plain("aaaa bbbb cccc", s);
        let l = layout(&mut ctx, &spec, 45.0);
        // Offset 10 is where line two starts.
        assert_eq!(l.caret(&mut ctx, &spec, 10, false), (0.0, 1));
        assert_eq!(l.caret(&mut ctx, &spec, 10, true).1, 0);
        assert_eq!(l.caret(&mut ctx, &spec, 2, false), (10.0, 0));
        assert_eq!(l.offset_at(&mut ctx, &spec, 0, 11.0), 2);
        assert_eq!(l.offset_at(&mut ctx, &spec, 0, 12.6), 3);
        // Past the end of a wrapped line: before its trailing space.
        assert_eq!(l.offset_at(&mut ctx, &spec, 0, 44.0), 9);
        assert_eq!(l.offset_at(&mut ctx, &spec, 1, 400.0), 14);
    }

    #[test]
    fn empty_aligned_lines_position_carets_and_selections() {
        use crate::display::{caret_rect, selection_rects};
        use crate::flow::{Keep, ParaBox};
        use crate::pos::Path;
        use std::sync::Arc;

        let (mut host, mut measure, mut styles, s) = setup();
        let mut ctx = Ctx { host: &mut host, measure: &mut measure, styles: &mut styles };
        for align in [Align::Center, Align::Right] {
            for (indent_start, indent_end, first_line) in [(0.0, 0.0, 0.0), (20.0, 10.0, 15.0), (20.0, 10.0, -10.0)] {
                for text in ["", "\nab", "ab\n", "\n\n"] {
                    let mut spec = ParaSpec::plain(text, s);
                    spec.align = align;
                    spec.indent_start = indent_start;
                    spec.indent_end = indent_end;
                    spec.first_line = first_line;
                    let l = layout(&mut ctx, &spec, 100.0);
                    let p = ParaBox {
                        path: Path::top(0),
                        spec: Arc::new(spec.clone()),
                        layout: Arc::new(l.clone()),
                        keep: Keep::default(),
                        y: 3.0,
                    };
                    for (li, line) in l.lines.iter().enumerate().filter(|(_, line)| line.frags.is_empty()) {
                        let base = indent_start + if li == 0 { first_line } else { 0.0 };
                        let factor = if align == Align::Center { 0.5 } else { 1.0 };
                        let expected = base + (100.0 - indent_end - base).max(0.0) * factor;
                        assert_eq!(l.caret(&mut ctx, &spec, line.start, false), (expected, li), "{align:?}, {text:?}, line {li}");
                        assert_eq!(caret_rect(&mut ctx, &p, 7.0, 11.0, line.start, false), (7.0 + expected, 14.0 + line.top, line.height));
                        let rects = selection_rects(&mut ctx, &p, 7.0, 11.0, line.start, line.end, true);
                        assert!(rects.contains(&(7.0 + expected, 14.0 + line.top, 5.0, line.height)), "{align:?}, {text:?}: {rects:?}");

                        // A typed 5 px glyph takes half/all its width off the aligned origin.
                        let mut typed_text = text.to_string();
                        typed_text.insert(line.start as usize, 'x');
                        let mut typed = ParaSpec::plain(&typed_text, s);
                        typed.align = align;
                        typed.indent_start = indent_start;
                        typed.indent_end = indent_end;
                        typed.first_line = first_line;
                        let typed_layout = layout(&mut ctx, &typed, 100.0);
                        assert_eq!(typed_layout.lines[li].frags[0].x + 5.0 * factor, expected);
                    }
                }
            }
        }
    }

    #[test]
    fn empty_list_line_caret_starts_after_the_label() {
        let (mut host, mut measure, mut styles, s) = setup();
        let mut ctx = Ctx { host: &mut host, measure: &mut measure, styles: &mut styles };
        for (align, expected) in [(Align::Left, 24.0), (Align::Center, 57.0), (Align::Right, 90.0)] {
            let mut spec = ParaSpec::plain("", s);
            spec.align = align;
            spec.indent_start = 24.0;
            spec.indent_end = 10.0;
            spec.first_line = -24.0;
            spec.label = Some(Label { text: "1.".into(), style: s });
            let l = layout(&mut ctx, &spec, 100.0);
            assert_eq!(l.caret(&mut ctx, &spec, 0, false), (expected, 0));
            let mut typed = ParaSpec::plain("x", s);
            typed.align = align;
            typed.indent_start = spec.indent_start;
            typed.indent_end = spec.indent_end;
            typed.first_line = spec.first_line;
            typed.label = spec.label.clone();
            let typed_layout = layout(&mut ctx, &typed, 100.0);
            let shift = match align {
                Align::Center => 2.5,
                Align::Right => 5.0,
                _ => 0.0,
            };
            assert_eq!(typed_layout.lines[0].frags[1].x + shift, expected);
        }
    }

    /// Each text frag on a line: its text, x, width and direction.
    fn frags_of(spec: &ParaSpec, line: &Line) -> Vec<(String, f64, f64, bool)> {
        line.frags
            .iter()
            .filter(|f| matches!(f.kind, FragKind::Text(_)))
            .map(|f| (spec.text[f.start as usize..f.end as usize].to_string(), f.x, f.width, f.rtl))
            .collect()
    }

    #[test]
    fn right_to_left_paragraphs_start_at_the_right() {
        let (mut host, mut measure, mut styles, s) = setup();
        let mut ctx = Ctx { host: &mut host, measure: &mut measure, styles: &mut styles };
        // Six letters and a space: 32.5 px.
        let mut spec = ParaSpec::plain("אבג דהו", s);
        spec.rtl = true;
        spec.align = Align::Right;
        let l = layout(&mut ctx, &spec, 100.0);
        assert_eq!(frags_of(&spec, &l.lines[0]), vec![("אבג דהו".into(), 67.5, 32.5, true)]);
        // The caret starts at the right and moves left as the text goes on.
        assert_eq!(l.caret(&mut ctx, &spec, 0, false).0, 100.0);
        assert_eq!(l.caret(&mut ctx, &spec, 2, false).0, 95.0);
        assert_eq!(l.caret(&mut ctx, &spec, spec.text.len() as u32, false).0, 67.5);
        assert_eq!(l.offset_at(&mut ctx, &spec, 0, 99.0), 0);
        assert_eq!(l.offset_at(&mut ctx, &spec, 0, 94.0), 2);
        assert_eq!(l.offset_at(&mut ctx, &spec, 0, 10.0), spec.text.len() as u32);

        // Indents count from the right; "left" alignment is the far side.
        spec.indent_start = 20.0;
        let l = layout(&mut ctx, &spec, 100.0);
        assert_eq!(frags_of(&spec, &l.lines[0])[0].1, 47.5);
        spec.align = Align::Left;
        spec.indent_end = 10.0;
        let l = layout(&mut ctx, &spec, 100.0);
        assert_eq!(frags_of(&spec, &l.lines[0])[0].1, 10.0);
        spec.align = Align::Center;
        let l = layout(&mut ctx, &spec, 100.0);
        assert_eq!(frags_of(&spec, &l.lines[0])[0].1, 28.75);

        // An empty line's caret stands at the right.
        let mut empty = ParaSpec::plain("", s);
        empty.rtl = true;
        empty.align = Align::Right;
        empty.indent_start = 15.0;
        let l = layout(&mut ctx, &empty, 100.0);
        assert_eq!(l.caret(&mut ctx, &empty, 0, false).0, 85.0);
    }

    #[test]
    fn right_to_left_wrapping_justification_labels_and_tabs() {
        let (mut host, mut measure, mut styles, s) = setup();
        let mut ctx = Ctx { host: &mut host, measure: &mut measure, styles: &mut styles };
        let mut spec = ParaSpec::plain("אאאא בבבב גגגג", s);
        spec.rtl = true;
        spec.align = Align::Justify;
        let l = layout(&mut ctx, &spec, 45.0);
        assert_eq!(lines_text(&spec, &l), vec!["אאאא בבבב ", "גגגג"]);
        // The first line fills the width, from the right; the last starts at the right.
        let first = &frags_of(&spec, &l.lines[0])[0];
        assert!(first.3 && (first.1 - (45.0 - first.2)).abs() < 1e-9, "{first:?}");
        assert_eq!(frags_of(&spec, &l.lines[1])[0].1, 25.0);

        // A list label hangs at the right, the text starts left of it.
        let mut spec = ParaSpec::plain("אב", s);
        spec.rtl = true;
        spec.align = Align::Right;
        spec.indent_start = 24.0;
        spec.first_line = -24.0;
        spec.label = Some(Label { text: "1.".into(), style: s });
        let l = layout(&mut ctx, &spec, 100.0);
        let label = &l.lines[0].frags[0];
        assert!(matches!(label.kind, FragKind::Label { .. }) && label.rtl);
        assert_eq!((label.x, label.width), (90.0, 10.0));
        assert_eq!(frags_of(&spec, &l.lines[0])[0].1, 66.0);

        // Tab stops count from the right too.
        let mut spec = ParaSpec::plain("א\tב", s);
        spec.rtl = true;
        spec.align = Align::Right;
        spec.default_tab = 48.0;
        let l = layout(&mut ctx, &spec, 500.0);
        let f = frags_of(&spec, &l.lines[0]);
        assert_eq!((f[0].1, f[1].1), (495.0, 447.0));
    }

    #[test]
    fn mixed_directions_reorder_by_the_bidi_algorithm() {
        let (mut host, mut measure, mut styles, s) = setup();
        let mut ctx = Ctx { host: &mut host, measure: &mut measure, styles: &mut styles };
        // English inside Hebrew: the Hebrew before it is rightmost, the English keeps its order.
        let mut spec = ParaSpec::plain("אבג abc דהו", s);
        spec.rtl = true;
        spec.align = Align::Right;
        let l = layout(&mut ctx, &spec, 100.0);
        assert_eq!(
            frags_of(&spec, &l.lines[0]),
            vec![("אבג ".into(), 82.5, 17.5, true), ("abc".into(), 67.5, 15.0, false), (" דהו".into(), 50.0, 17.5, true)]
        );
        let abc = "אבג ".len() as u32;
        // In the English the caret moves right; where the runs meet it follows the text before.
        assert_eq!(l.caret(&mut ctx, &spec, abc, false).0, 82.5);
        assert_eq!(l.caret(&mut ctx, &spec, abc + 1, false).0, 72.5);
        assert_eq!(l.caret(&mut ctx, &spec, abc + 3, false).0, 82.5);
        assert_eq!(l.offset_at(&mut ctx, &spec, 0, 73.0), abc + 1);
        assert_eq!(l.offset_at(&mut ctx, &spec, 0, 56.0), spec.text.len() as u32 - 2);

        // Numbers in right-to-left text read left to right; the full stop ends up at the left.
        let mut spec = ParaSpec::plain("אבג 123.", s);
        spec.rtl = true;
        spec.align = Align::Right;
        let l = layout(&mut ctx, &spec, 100.0);
        assert_eq!(
            frags_of(&spec, &l.lines[0]),
            vec![("אבג ".into(), 82.5, 17.5, true), ("123".into(), 67.5, 15.0, false), (".".into(), 62.5, 5.0, true)]
        );

        // Hebrew inside an English paragraph runs right to left in place.
        let spec = ParaSpec::plain("ab אבג cd", s);
        let l = layout(&mut ctx, &spec, 100.0);
        assert_eq!(
            frags_of(&spec, &l.lines[0]),
            vec![("ab ".into(), 0.0, 12.5, false), ("אבג".into(), 12.5, 15.0, true), (" cd".into(), 27.5, 12.5, false)]
        );
        // Two letters into the Hebrew: 5 px left of its right edge.
        assert_eq!(l.caret(&mut ctx, &spec, 3 + 2, false).0, 22.5);
        // Plain left-to-right text isn't touched.
        let spec = ParaSpec::plain("ab cd", s);
        let l = layout(&mut ctx, &spec, 100.0);
        assert!(!l.lines[0].bidi && !l.lines[0].frags[0].rtl);
    }

    #[test]
    fn mixed_lines_keep_their_place() {
        let (mut host, mut measure, mut styles, s) = setup();
        let mut ctx = Ctx { host: &mut host, measure: &mut measure, styles: &mut styles };
        let starts = |spec: &ParaSpec, l: &ParaLayout| frags_of(spec, &l.lines[0]).iter().map(|f| f.1).fold(f64::INFINITY, f64::min);
        let ends = |spec: &ParaSpec, l: &ParaLayout| frags_of(spec, &l.lines[0]).iter().map(|f| f.1 + f.2).fold(0.0, f64::max);
        // Centered: "abc אבג" is 32.5 px.
        let mut spec = ParaSpec::plain("abc אבג", s);
        spec.align = Align::Center;
        let l = layout(&mut ctx, &spec, 100.0);
        assert_eq!(starts(&spec, &l), 33.75);
        // Indented, and after a list label.
        let mut spec = ParaSpec::plain("see שלום here", s);
        spec.indent_start = 24.0;
        spec.first_line = -24.0;
        spec.label = Some(Label { text: "1.".into(), style: s });
        let l = layout(&mut ctx, &spec, 200.0);
        assert_eq!(starts(&spec, &l), 24.0);
        // After a right tab.
        let mut spec = ParaSpec::plain("a\tאבג abc", s);
        spec.tabs = vec![Tab { pos: 200.0, kind: TabKind::Right }];
        let l = layout(&mut ctx, &spec, 500.0);
        let after: Vec<_> = frags_of(&spec, &l.lines[0]).into_iter().skip(1).collect();
        assert_eq!(after.iter().map(|f| f.1 + f.2).fold(0.0, f64::max), 200.0, "{after:?}");
        // A right-to-left paragraph's start indent, and its far-side alignment.
        let mut spec = ParaSpec::plain("אבג abc דהו", s);
        spec.rtl = true;
        spec.align = Align::Right;
        spec.indent_start = 20.0;
        let l = layout(&mut ctx, &spec, 100.0);
        assert_eq!(ends(&spec, &l), 80.0);
        let mut spec = ParaSpec::plain("abc דהו", s);
        spec.rtl = true;
        let l = layout(&mut ctx, &spec, 100.0);
        assert_eq!(starts(&spec, &l), 0.0);
        // Justified: the space ending the line isn't stretched.
        // 45 px of text on a 50 px line: 2.5 px more for each of the two spaces inside it, and
        // the trailing one hangs 2.5 px past the end.
        let mut spec = ParaSpec::plain("abc אבג dd ee", s);
        spec.align = Align::Justify;
        let l = layout(&mut ctx, &spec, 50.0);
        let first = frags_of(&spec, &l.lines[0]);
        assert_eq!(first.iter().map(|f| f.1 + f.2).fold(0.0, f64::max), 52.5, "{first:?}");
    }

    #[test]
    fn selections_across_directions_cover_each_piece() {
        use crate::display::selection_rects;
        use crate::flow::{Keep, ParaBox};
        use crate::pos::Path;
        use std::sync::Arc;

        let (mut host, mut measure, mut styles, s) = setup();
        let mut ctx = Ctx { host: &mut host, measure: &mut measure, styles: &mut styles };
        let mut spec = ParaSpec::plain("אבג abc דהו", s);
        spec.rtl = true;
        spec.align = Align::Right;
        let l = layout(&mut ctx, &spec, 100.0);
        let p = ParaBox { path: Path::top(0), spec: Arc::new(spec.clone()), layout: Arc::new(l), keep: Keep::default(), y: 0.0 };
        // From the second Hebrew letter to the middle of the English: two pieces apart.
        let from = 2;
        let to = "אבג a".len() as u32;
        let rects = selection_rects(&mut ctx, &p, 0.0, 0.0, from, to, false);
        assert_eq!(rects, vec![(67.5, 0.0, 5.0, 10.0), (82.5, 0.0, 12.5, 10.0)]);
        // Everything: one rectangle, a little past the end at the left.
        let rects = selection_rects(&mut ctx, &p, 0.0, 0.0, 0, spec.text.len() as u32, true);
        assert_eq!(rects, vec![(45.0, 0.0, 55.0, 10.0)]);
    }

    #[test]
    fn words() {
        assert_eq!(word_at("hello, world", 2), (0, 5));
        assert_eq!(word_at("hello, world", 5), (0, 5));
        assert_eq!(word_at("hello, world", 7), (7, 12));
        assert_eq!(word_move("hello, world", 0, true), 5);
        assert_eq!(word_move("hello, world", 5, true), 12);
        assert_eq!(word_move("hello, world", 12, false), 7);
        assert_eq!(word_move("hello, world", 7, false), 0);
        // Hebrew with points, and an abbreviation with gershayim, are each one word.
        let pointed = "שָׁלוֹם עוֹלָם";
        assert_eq!(word_at(pointed, 2), (0, "שָׁלוֹם".len() as u32));
        assert_eq!(word_move(pointed, 0, true), "שָׁלוֹם".len() as u32);
        let abbr = "של צה״ל היום";
        assert_eq!(word_at(abbr, "של צ".len() as u32), ("של ".len() as u32, "של צה״ל".len() as u32));
    }

    #[test]
    fn layout_keys_follow_content() {
        let (_, _, _, s) = setup();
        let a = ParaSpec::plain("abc", s);
        let mut b = a.clone();
        assert_eq!(a.key(100.0), b.key(100.0));
        assert_ne!(a.key(100.0), a.key(101.0));
        b.align = Align::Center;
        assert_ne!(a.key(100.0), b.key(100.0));
    }
}
