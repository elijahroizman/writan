//! Quick hashes of model paragraphs, for reusing what was worked out about one the last time it
//! was laid out (its spec: styles resolved, runs turned into items) when nothing about it changed.

use std::hash::{Hash, Hasher};

use office_core::drawing::{Color, ColorBase};
use office_core::model::{Inline, LineSpacing, ParaProps, Paragraph, RunProps};

/// FNV-1a.
pub struct Fnv(pub u64);

impl Default for Fnv {
    fn default() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= *b as u64;
            self.0 = self.0.wrapping_mul(0x100_0000_01b3);
        }
    }
}

fn color(h: &mut Fnv, c: &Option<Color>) {
    let Some(c) = c else { return 0u8.hash(h) };
    match &c.base {
        ColorBase::Rgb { value } => (1u8, value.0, value.1, value.2).hash(h),
        ColorBase::Scheme { name } => (2u8, name).hash(h),
        ColorBase::System { name, last } => (3u8, name, last.map(|l| (l.0, l.1, l.2))).hash(h),
        ColorBase::Preset { name } => (4u8, name).hash(h),
        ColorBase::Hsl { hue, sat, lum } => (5u8, hue, sat, lum).hash(h),
        ColorBase::ScRgb { r, g, b } => (6u8, r, g, b).hash(h),
    }
    for t in &c.transforms {
        (&t.kind, t.value).hash(h);
    }
}

pub fn run_props(h: &mut Fnv, r: &RunProps) {
    (&r.font, &r.font_east_asian, &r.font_complex, r.size.map(|s| s.0), r.bold, r.italic, &r.underline).hash(h);
    (r.strike.map(|s| s as u8), r.caps, r.small_caps, r.baseline.map(|b| b as u8), &r.style, &r.lang).hash(h);
    (r.size_complex.map(|s| s.0), r.bold_complex, r.italic_complex, r.rtl).hash(h);
    color(h, &r.color);
    color(h, &r.highlight);
}

pub fn para_props(h: &mut Fnv, p: &ParaProps) {
    (&p.style, p.align.map(|a| a as u8), p.level, p.indent_start.map(|e| e.0), p.indent_end.map(|e| e.0), p.indent_first_line.map(|e| e.0))
        .hash(h);
    (p.space_before.map(|e| e.0), p.space_after.map(|e| e.0), p.rtl).hash(h);
    match p.line_spacing {
        Some(LineSpacing::Multiple(m)) => (1u8, m.0 as i64).hash(h),
        Some(LineSpacing::Exactly(e)) => (2u8, e.0).hash(h),
        Some(LineSpacing::AtLeast(e)) => (3u8, e.0).hash(h),
        None => 0u8.hash(h),
    }
}

/// What a paragraph says: its properties, its content (text, formatting, links, objects) and the
/// markup it came from. Raw markup counts by its identity: it's replaced, never changed in place.
pub fn paragraph(h: &mut Fnv, p: &Paragraph) {
    para_props(h, &p.props);
    match &p.mark {
        Some(m) => run_props(h, m),
        None => 0u8.hash(h),
    }
    p.origin.id().hash(h);
    p.content.len().hash(h);
    for inline in &p.content {
        match inline {
            Inline::Text(r) => {
                0u8.hash(h);
                r.text.hash(h);
                run_props(h, &r.props);
                r.link.as_ref().map(|l| (&l.url, &l.anchor)).hash(h);
                r.origin.id().hash(h);
            }
            Inline::Break { kind, .. } => (1u8, *kind as u8).hash(h),
            Inline::Image(img) => (2u8, &img.media, img.width.0, img.height.0, img.origin.id()).hash(h),
            Inline::Raw { xml } => (3u8, &xml.local, xml.children.as_ptr() as usize, xml.children.len(), xml.attrs.len()).hash(h),
        }
    }
}
