//! Rich text, the same in all three formats: Word paragraphs, the text in a slide's shapes and
//! table cells, and Excel's rich-text cells.
//!
//! A run's text uses plain characters for what the formats mark up differently: `\t` for a tab,
//! `\n` for a line break (Word's `w:br`, PowerPoint's `a:br`), U+2011 for a non-breaking hyphen
//! and U+00AD for a soft hyphen. Page and column breaks are [`Inline::Break`].

use serde::{Deserialize, Serialize};

use super::{Image, Origin};
use crate::drawing::Color;
use crate::units::{Emu, FontSize, Percent};
use crate::xml::{ns, Element, Node};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Paragraph {
    #[serde(default, skip_serializing_if = "ParaProps::is_empty")]
    pub props: ParaProps,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<Inline>,
    /// Formatting of the paragraph mark (Word's `w:pPr/w:rPr`, PowerPoint's `a:endParaRPr`): what
    /// an empty paragraph's text looks like, and what typing at its end starts with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark: Option<RunProps>,
    #[serde(skip)]
    pub origin: Origin,
}

// Runs are nearly every inline; boxing them to shrink the rare ones would cost more than it saves.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum Inline {
    Text(Run),
    Break {
        kind: BreakKind,
        /// The run it was read from, so an untouched break (and its run's formatting) goes back
        /// as it was.
        #[serde(skip)]
        origin: Origin,
    },
    Image(Image),
    /// Markup the core doesn't model (fields, bookmarks, comment marks, equations…), whole. In
    /// Word this is a complete `w:r` or other paragraph child.
    Raw {
        xml: Element,
    },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BreakKind {
    Page,
    Column,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub text: String,
    #[serde(default, skip_serializing_if = "RunProps::is_empty")]
    pub props: RunProps,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<Link>,
    #[serde(skip)]
    pub origin: Origin,
}

/// A hyperlink on a run. Word wraps runs in `w:hyperlink`; PowerPoint puts `a:hlinkClick` in each
/// run's properties. Either way it's a property of the text here.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    /// A URL (or file path) outside the document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// A place inside it: a Word bookmark's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
}

/// Direct formatting on a run: only what the run itself says. What a style or placeholder
/// would add is the layers' business.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunProps {
    /// Latin font. Theme fonts are written DrawingML's way: `+mj-lt`, `+mn-lt`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_east_asian: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_complex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<FontSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    /// Word keeps complex-script text (Hebrew, Arabic, Thai…) formatted apart: its size, bold
    /// and italic (`w:szCs`, `w:bCs`, `w:iCs`) and font ([`RunProps::font_complex`]). Unset,
    /// they follow `size`, `bold` and `italic` (and are written so).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_complex: Option<FontSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold_complex: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic_complex: Option<bool>,
    /// Word's `w:rtl`: right-to-left text, all of it shown with the complex-script formatting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtl: Option<bool>,
    /// Word's names: `single`, `double`, `thick`, `dotted`, `wave`… and `none`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underline: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strike: Option<Strike>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caps: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub small_caps: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub highlight: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<Baseline>,
    /// A character style's id (Word's `w:rStyle`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// A language tag: `en-GB`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
}

impl RunProps {
    pub fn is_empty(&self) -> bool {
        self == &RunProps::default()
    }

    /// Lays `over` on these: each property `over` has replaces this one's. How styles,
    /// placeholders and direct formatting stack.
    pub fn overlay(&mut self, over: &RunProps) {
        macro_rules! take {
            ($($f:ident),*) => { $( if over.$f.is_some() { self.$f = over.$f.clone(); } )* };
        }
        take!(
            font,
            font_east_asian,
            font_complex,
            size,
            bold,
            italic,
            size_complex,
            bold_complex,
            italic_complex,
            rtl,
            underline,
            strike,
            caps,
            small_caps,
            color,
            highlight,
            baseline,
            style,
            lang
        );
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Strike {
    None,
    Single,
    Double,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Baseline {
    Superscript,
    Subscript,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParaProps {
    /// A paragraph style's id (Word's `w:pStyle`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
    /// How deep in a list or outline: PowerPoint's `lvl`, Word's list level (`w:ilvl`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,
    /// Space between the start edge (left, in left-to-right text) and the text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indent_start: Option<Emu>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indent_end: Option<Emu>,
    /// The first line's extra indent; negative for a hanging indent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indent_first_line: Option<Emu>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_before: Option<Emu>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_after: Option<Emu>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_spacing: Option<LineSpacing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtl: Option<bool>,
}

impl ParaProps {
    pub fn is_empty(&self) -> bool {
        self == &ParaProps::default()
    }

    /// Lays `over` on these, as [`RunProps::overlay`] does.
    pub fn overlay(&mut self, over: &ParaProps) {
        macro_rules! take {
            ($($f:ident),*) => { $( if over.$f.is_some() { self.$f = over.$f.clone(); } )* };
        }
        take!(style, align, level, indent_start, indent_end, indent_first_line, space_before, space_after, line_spacing, rtl);
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Align {
    Left,
    Center,
    Right,
    Justify,
    Distribute,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "value")]
pub enum LineSpacing {
    /// A multiple of single spacing: 100 000 is single, 150 000 one and a half.
    Multiple(Percent),
    Exactly(Emu),
    AtLeast(Emu),
}

impl Paragraph {
    /// A paragraph of one plain run.
    pub fn text(text: impl Into<String>) -> Paragraph {
        Paragraph { content: vec![Inline::Text(Run::new(text))], ..Default::default() }
    }

    /// The paragraph's text: runs, with page breaks as form feeds and pictures and raw markup
    /// left out.
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        for inline in &self.content {
            match inline {
                Inline::Text(r) => out.push_str(&r.text),
                Inline::Break { .. } => out.push('\u{c}'),
                Inline::Image(_) | Inline::Raw { .. } => {}
            }
        }
        out
    }

    /// Joins neighboring runs with the same formatting and link into one, as the formats split
    /// text differently: PowerPoint keeps each line break as a run of its own, Word splits runs
    /// for spell checking and revision marks. The joined run keeps the first run's origin.
    pub fn merge_runs(&mut self) {
        let mut merged: Vec<Inline> = Vec::with_capacity(self.content.len());
        for inline in self.content.drain(..) {
            if let (Some(Inline::Text(prev)), Inline::Text(run)) = (merged.last_mut(), &inline) {
                if prev.props == run.props && prev.link == run.link && same_run_formatting(&prev.origin, &run.origin) {
                    prev.text.push_str(&run.text);
                    continue;
                }
            }
            merged.push(inline);
        }
        self.content = merged;
    }

    pub fn runs(&self) -> impl Iterator<Item = &Run> {
        self.content.iter().filter_map(|i| match i {
            Inline::Text(r) => Some(r),
            _ => None,
        })
    }

    pub fn runs_mut(&mut self) -> impl Iterator<Item = &mut Run> {
        self.content.iter_mut().filter_map(|i| match i {
            Inline::Text(r) => Some(r),
            _ => None,
        })
    }
}

impl Run {
    pub fn new(text: impl Into<String>) -> Run {
        Run { text: text.into(), ..Default::default() }
    }

    pub fn with_props(mut self, props: RunProps) -> Run {
        self.props = props;
        self
    }
}

/// Word's and PowerPoint's underline names, Word's first. Word's names are the model's.
pub const UNDERLINES: &[(&str, &str)] = &[
    ("single", "sng"),
    ("double", "dbl"),
    ("thick", "heavy"),
    ("dotted", "dotted"),
    ("dottedHeavy", "dottedHeavy"),
    ("dash", "dash"),
    ("dashedHeavy", "dashHeavy"),
    ("dashLong", "dashLong"),
    ("dashLongHeavy", "dashLongHeavy"),
    ("dotDash", "dotDash"),
    ("dashDotHeavy", "dotDashHeavy"),
    ("dotDotDash", "dotDotDash"),
    ("dashDotDotHeavy", "dotDotDashHeavy"),
    ("wave", "wavy"),
    ("wavyHeavy", "wavyHeavy"),
    ("wavyDouble", "wavyDbl"),
    ("words", "words"),
    ("none", "none"),
];

/// PowerPoint's name for an underline.
pub fn underline_to_drawingml(word: &str) -> &str {
    UNDERLINES.iter().find(|(w, _)| *w == word).map(|(_, d)| *d).unwrap_or("sng")
}

/// The model's (Word's) name for one of PowerPoint's underlines.
pub fn underline_from_drawingml(dml: &str) -> &str {
    UNDERLINES.iter().find(|(_, d)| *d == dml).map(|(w, _)| *w).unwrap_or("single")
}

fn same_run_formatting(a: &Origin, b: &Origin) -> bool {
    let (a, b) = match (a.get(), b.get()) {
        (None, None) => return true,
        (Some(a), Some(b)) => (a, b),
        _ => return false,
    };
    let same_kind =
        a.local == b.local || (a.in_ns(ns::A) && matches!(a.local.as_str(), "r" | "br") && matches!(b.local.as_str(), "r" | "br"));
    let text = |e: &Element| {
        (e.local == "t" && (e.in_ns(ns::W) || e.in_ns(ns::A) || e.in_ns(ns::X)))
            || (e.in_ns(ns::W) && matches!(e.local.as_str(), "tab" | "br" | "noBreakHyphen" | "softHyphen"))
    };
    let formatting = |n: &&Node| !matches!(n, Node::Element(e) if text(e));
    same_kind
        && a.ns == b.ns
        && a.attrs == b.attrs
        && a.children.iter().filter(formatting).eq(b.children.iter().filter(formatting))
        && a.elements().filter(|e| text(e)).flat_map(|e| &e.attrs).eq(b.elements().filter(|e| text(e)).flat_map(|e| &e.attrs))
}
