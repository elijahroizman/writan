use std::collections::{BTreeMap, HashMap};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;

use pdf_writer::{
    types::{CidFontType, FontFlags, SystemInfo, UnicodeCmap},
    Content, Filter, Finish, Name, Pdf, Rect, Ref, Str,
};
use rustybuzz::{Direction, Feature, UnicodeBuffer};
use subsetter::GlyphRemapper;
use ttf_parser::{Face, GlyphId, Tag};
use unicode_bidi::{Level, ParagraphBidiInfo};
use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

use crate::flate;

#[derive(Clone)]
pub(crate) struct FontSpec {
    pub size: f64,
    families: Vec<String>,
    bold: bool,
    italic: bool,
}

fn family_key(family: &str) -> String {
    let name = family.trim_matches([' ', '\'', '"']).to_ascii_lowercase();
    match name.as_str() {
        "calibri" | "aptos" | "carlito" => "carlito".into(),
        "cambria" | "caladea" => "caladea".into(),
        "arial" | "helvetica" | "segoe ui" | "liberation sans" | "sans-serif" | "system-ui" => "liberation-sans".into(),
        "times new roman" | "times" | "georgia" | "liberation serif" | "serif" => "liberation-serif".into(),
        "courier new" | "courier" | "liberation mono" | "monospace" => "liberation-mono".into(),
        "noto sans arabic" => "noto-arabic".into(),
        "cambria math" | "noto sans math" => "noto-math".into(),
        "noto sans" => "noto-sans".into(),
        _ => name.replace(' ', "-"),
    }
}

fn family_list(input: &str) -> Vec<String> {
    let mut quote = None;
    let mut start = 0;
    let mut families = Vec::new();
    for (i, ch) in input.char_indices() {
        match (ch, quote) {
            (ch, Some(end)) if ch == end => quote = None,
            ('"' | '\'', None) => quote = Some(ch),
            (',', None) => {
                families.push(family_key(&input[start..i]));
                start = i + 1;
            }
            _ => {}
        }
    }
    families.push(family_key(&input[start..]));
    families
}

impl FontSpec {
    pub fn parse(css: &str) -> Self {
        let (mut size, mut families, mut bold, mut italic) = (16., vec![], false, false);
        let mut tail = css.trim_start();
        while !tail.is_empty() {
            let end = tail.find(|ch: char| ch.is_whitespace() || ch == '/').unwrap_or(tail.len());
            let token = &tail[..end];
            tail = tail[end..].trim_start();
            let px = token.strip_suffix("px").and_then(|v| v.parse::<f64>().ok());
            let pt = token.strip_suffix("pt").and_then(|v| v.parse::<f64>().ok());
            if let Some(value) = px.or(pt.map(|v| v * 4. / 3.)) {
                if value.is_finite() && value > 0. {
                    size = value;
                }
                if let Some(line_height) = tail.strip_prefix('/') {
                    let line_height = line_height.trim_start();
                    tail = line_height.find(char::is_whitespace).map_or("", |i| &line_height[i..]);
                }
                families = family_list(tail.trim());
                break;
            }
            if token == "bold" || token == "bolder" || token.parse::<u16>().is_ok_and(|n| n >= 600) {
                bold = true;
            }
            if matches!(token, "italic" | "oblique") {
                italic = true;
            }
            if end == 0 {
                break;
            }
        }
        Self { size, families, bold, italic }
    }

    pub fn from_family(family: &str, size: f64, bold: bool, italic: bool) -> Self {
        Self { size, families: family_list(family), bold, italic }
    }
}

struct Font<'a> {
    bytes: &'a [u8],
    face: rustybuzz::Face<'a>,
    postscript_name: String,
    remapper: GlyphRemapper,
    unicode: Vec<BTreeMap<u16, String>>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct FontId {
    face: usize,
    encoding: usize,
}

impl FontId {
    pub fn resource_name(self, bases: &BTreeMap<usize, String>) -> String {
        let base = &bases[&self.face];
        if self.encoding == 0 {
            base.clone()
        } else {
            format!("{base}_{}", self.encoding)
        }
    }
}

pub(crate) struct Fonts<'a> {
    fonts: Vec<Font<'a>>,
    by_key: HashMap<&'a str, usize>,
}

#[derive(Clone, Copy)]
struct Glyph {
    encoding: usize,
    cid: u16,
    x: f64,
    y: f64,
    advance: f64,
}

struct Cluster {
    font: usize,
    size: f64,
    glyphs: Vec<Glyph>,
    actual_text: Option<String>,
}

pub(crate) struct ShapedText {
    pub width: f64,
    pub ascent: f64,
    pub descent: f64,
    clusters: Vec<Cluster>,
}

#[derive(PartialEq, Eq)]
struct RunStyle {
    font: usize,
    script: Script,
    caps: Caps,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Caps {
    Normal,
    Native,
    Synthetic,
}

const SMALL_CAPS: Tag = Tag::from_bytes(b"smcp");

struct Run {
    range: Range<usize>,
    style: RunStyle,
}

impl<'a> Fonts<'a> {
    pub fn len(&self) -> usize {
        self.fonts.len()
    }

    pub fn new(blobs: &'a HashMap<String, Vec<u8>>) -> Self {
        let mut entries: Vec<_> = blobs.iter().collect();
        entries.sort_unstable_by_key(|(key, _)| *key);
        let mut fonts = Vec::new();
        let mut by_key = HashMap::new();
        for (key, bytes) in entries {
            let Some(face) = rustybuzz::Face::from_slice(bytes, 0) else {
                continue;
            };
            if face.tables().glyf.is_none() {
                continue;
            }
            let postscript_name = face
                .names()
                .into_iter()
                .filter(|name| name.name_id == ttf_parser::name_id::POST_SCRIPT_NAME)
                .find_map(|name| name.to_string())
                .unwrap_or_else(|| key.replace(':', "-"));
            by_key.insert(key.as_str(), fonts.len());
            fonts.push(Font { bytes, face, postscript_name, remapper: GlyphRemapper::new(), unicode: vec![BTreeMap::new()] });
        }
        Self { fonts, by_key }
    }

    fn candidates(&self, spec: &FontSpec) -> Vec<usize> {
        let mut fonts = Vec::new();
        for family in spec.families.iter().map(String::as_str).chain(["liberation-sans", "noto-arabic", "noto-math", "noto-sans"]) {
            let variants = match (spec.bold, spec.italic) {
                (true, true) => &["bold-italic", "bold", "italic", "regular"][..],
                (true, false) => &["bold", "regular"],
                (false, true) => &["italic", "regular"],
                (false, false) => &["regular"],
            };
            if let Some(&font) = variants.iter().find_map(|variant| self.by_key.get(format!("{family}:{variant}").as_str())) {
                if !fonts.contains(&font) {
                    fonts.push(font);
                }
            }
        }
        if fonts.is_empty() {
            fonts.extend(0..self.fonts.len());
        }
        fonts
    }

    fn choose_font(&self, candidates: &[usize], text: &str, small: bool) -> usize {
        let letters = text.chars().filter(|&ch| !is_format_only(ch));
        let mut best = (candidates[0], usize::MAX);
        for &font in candidates {
            let face = &self.fonts[font].face;
            let missing = letters
                .clone()
                .filter(|&ch| {
                    if small {
                        ch.to_uppercase().any(|upper| face.glyph_index(upper).is_none())
                    } else {
                        face.glyph_index(ch).is_none()
                    }
                })
                .count();
            if missing == 0 {
                return font;
            }
            if missing < best.1 {
                best = (font, missing);
            }
        }
        best.0
    }

    pub fn shape(&mut self, text: &str, spec: &FontSpec, rtl: Option<bool>, small_caps: bool, spacing: f64) -> Result<ShapedText, String> {
        let mut shaped = ShapedText { width: 0., ascent: 0., descent: 0., clusters: Vec::new() };
        if text.is_empty() {
            return Ok(shaped);
        }
        let candidates = self.candidates(spec);
        if candidates.is_empty() {
            return Err("PDF text requires a valid TrueType font in the font pack".into());
        }
        let primary = &self.fonts[candidates[0]].face;
        let unit = spec.size / f64::from(primary.units_per_em());
        shaped.ascent = f64::from(primary.ascender()) * unit;
        shaped.descent = f64::from(-i32::from(primary.descender())) * unit;

        let bidi = ParagraphBidiInfo::new(text, rtl.map(|rtl| if rtl { Level::rtl() } else { Level::ltr() }));
        let (levels, visual_runs) = bidi.visual_runs(0..text.len());
        let mut logical = BTreeMap::new();
        for bidi_run in visual_runs {
            let rtl = levels[bidi_run.start].is_rtl();
            let mut runs = Vec::<Run>::new();
            let mut script = Script::Common;
            for (offset, grapheme) in text[bidi_run.clone()].grapheme_indices(true) {
                let start = bidi_run.start + offset;
                let end = start + grapheme.len();
                let small = small_caps && grapheme.chars().any(char::is_lowercase);
                let font = self.choose_font(&candidates, grapheme, small);
                let caps = if !small {
                    Caps::Normal
                } else if self.fonts[font].face.tables().gsub.is_some_and(|table| table.features.find(SMALL_CAPS).is_some()) {
                    Caps::Native
                } else {
                    Caps::Synthetic
                };
                if let Some(strong) =
                    grapheme.chars().map(|ch| ch.script()).find(|s| !matches!(s, Script::Common | Script::Inherited | Script::Unknown))
                {
                    script = strong;
                }
                let style = RunStyle { font, script, caps };
                if let Some(previous) = runs.last_mut().filter(|run| run.style == style) {
                    previous.range.end = end;
                } else {
                    runs.push(Run { range: start..end, style });
                }
            }
            if rtl {
                runs.reverse();
            }
            for run in runs {
                self.shape_run(text, run, spec.size, rtl, spacing, &mut shaped.width, &mut logical);
            }
        }
        shaped.clusters = logical.into_values().collect();
        Ok(shaped)
    }

    #[allow(clippy::too_many_arguments)]
    fn shape_run(
        &mut self,
        text: &str,
        run: Run,
        size: f64,
        rtl: bool,
        spacing: f64,
        cursor: &mut f64,
        logical: &mut BTreeMap<usize, Cluster>,
    ) {
        let font = &mut self.fonts[run.style.font];
        let size = if run.style.caps == Caps::Synthetic { size * 0.78 } else { size };
        let unit = size / f64::from(font.face.units_per_em());
        let input = &text[run.range.clone()];
        let mut buffer = UnicodeBuffer::new();
        for (start, grapheme) in input.grapheme_indices(true) {
            for ch in grapheme.chars() {
                if run.style.caps == Caps::Synthetic {
                    for upper in ch.to_uppercase() {
                        buffer.add(upper, start as u32);
                    }
                } else {
                    buffer.add(ch, start as u32);
                }
            }
        }
        buffer.set_direction(if rtl { Direction::RightToLeft } else { Direction::LeftToRight });
        if let Ok(script) = run.style.script.short_name().parse() {
            buffer.set_script(script);
        }
        buffer.set_flags(rustybuzz::BufferFlags::REMOVE_DEFAULT_IGNORABLES);
        buffer.guess_segment_properties();
        let small_caps = [Feature::new(SMALL_CAPS, 1, ..)];
        let features = if run.style.caps == Caps::Native { &small_caps[..] } else { &[] };
        let result = rustybuzz::shape(&font.face, features, buffer);
        let infos = result.glyph_infos();
        let positions = result.glyph_positions();
        let mut boundaries: Vec<usize> = infos.iter().map(|info| info.cluster as usize).collect();
        boundaries.sort_unstable();
        boundaries.dedup();
        boundaries.push(input.len());

        let mut start = 0;
        while start < infos.len() {
            let cluster = infos[start].cluster;
            let end = start + infos[start..].partition_point(|info| info.cluster == cluster);
            let from = cluster as usize;
            let to = boundaries[boundaries.partition_point(|&offset| offset <= from)];
            let original = &input[if from == boundaries[0] { 0 } else { from }..to];
            let mut actual = end - start > 1 || original.chars().count() > 1 || run.style.caps == Caps::Synthetic;
            let mut glyphs = Vec::with_capacity(end - start);
            // Attach the Unicode to the advancing glyph rather than a zero-width mark.
            // Keep all shaped coordinates, and show the base first within the cluster.
            let base = positions[start..end].iter().position(|pos| pos.x_advance != 0).unwrap_or(0);
            for (index, (info, pos)) in infos[start..end].iter().zip(&positions[start..end]).enumerate() {
                let mut gid = info.glyph_id as u16;
                let mut advance = pos.x_advance;
                if gid == 0 {
                    if let Some(replacement) = font.face.glyph_index('\u{fffd}').or_else(|| font.face.glyph_index('?')) {
                        gid = replacement.0;
                        advance = i32::from(font.face.glyph_hor_advance(replacement).unwrap_or(0));
                    }
                    actual = true;
                }
                let cid = font.remapper.remap(gid);
                let mapping = if index == base { original } else { "\u{200b}" };
                let encoding = font.map_unicode(cid, mapping);
                let adv = f64::from(advance) * unit;
                glyphs.push(Glyph {
                    encoding,
                    cid,
                    x: *cursor + f64::from(pos.x_offset) * unit,
                    y: -f64::from(pos.y_offset) * unit,
                    advance: adv,
                });
                *cursor += adv;
            }
            glyphs[..=base].rotate_right(1);
            *cursor += original.chars().filter(|&ch| ch == ' ').count() as f64 * spacing;
            logical.insert(
                run.range.start + from,
                Cluster { font: run.style.font, size, glyphs, actual_text: actual.then(|| original.to_owned()) },
            );
            start = end;
        }
    }

    pub fn used(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.fonts.iter().enumerate().filter_map(|(id, font)| (!font.unicode[0].is_empty()).then_some((id, font.unicode.len())))
    }

    pub fn embed(&self, pdf: &mut Pdf, next: &mut Ref) -> Result<BTreeMap<FontId, Ref>, String> {
        let mut refs = BTreeMap::new();
        for (index, _) in self.used() {
            let font = &self.fonts[index];
            let cid = next.bump();
            let descriptor = next.bump();
            let file = next.bump();
            let subset =
                subsetter::subset(font.bytes, 0, &font.remapper).map_err(|e| format!("Could not subset {}: {e}", font.postscript_name))?;
            let mut hash = DefaultHasher::new();
            subset.hash(&mut hash);
            let mut tag = [b'A'; 6];
            let mut i = Hasher::finish(&hash);
            for letter in &mut tag {
                *letter += (i % 26) as u8;
                i /= 26;
            }
            let mut name = tag.to_vec();
            name.push(b'+');
            name.extend(font.postscript_name.bytes());
            let name = Name(&name);

            let subset_face = Face::parse(&subset, 0).map_err(|e| e.to_string())?;
            let units = 1000. / f32::from(subset_face.units_per_em());
            let subset_data = flate(&subset)?;
            pdf.stream(file, &subset_data).filter(Filter::FlateDecode).pair(Name(b"Length1"), subset.len() as i32);

            let info = SystemInfo { registry: Str(b"Adobe"), ordering: Str(b"Identity"), supplement: 0 };
            for (encoding, map) in font.unicode.iter().enumerate() {
                let font_ref = next.bump();
                let to_unicode = next.bump();
                refs.insert(FontId { face: index, encoding }, font_ref);
                let mut cmap = UnicodeCmap::new(Name(b"Adobe-Identity-UCS"), info);
                for (&gid, text) in map {
                    cmap.pair_with_multiple(gid, text.chars());
                }
                let cmap = flate(&cmap.finish())?;
                pdf.stream(to_unicode, &cmap).filter(Filter::FlateDecode);
                pdf.type0_font(font_ref)
                    .base_font(name)
                    .encoding_predefined(Name(b"Identity-H"))
                    .descendant_font(cid)
                    .to_unicode(to_unicode);
            }
            let mut descendant = pdf.cid_font(cid);
            descendant
                .subtype(CidFontType::Type2)
                .base_font(name)
                .system_info(info)
                .font_descriptor(descriptor)
                .default_width(1000.)
                .cid_to_gid_map_predefined(Name(b"Identity"));
            descendant.widths().consecutive(
                0,
                (0..subset_face.number_of_glyphs()).map(|gid| f32::from(subset_face.glyph_hor_advance(GlyphId(gid)).unwrap_or(0)) * units),
            );
            descendant.finish();

            let mut flags = FontFlags::SYMBOLIC;
            flags.set(FontFlags::FIXED_PITCH, font.face.is_monospaced());
            flags.set(FontFlags::ITALIC, font.face.is_italic());
            let bb = font.face.global_bounding_box();
            pdf.font_descriptor(descriptor)
                .name(name)
                .flags(flags)
                .bbox(Rect::new(
                    f32::from(bb.x_min) * units,
                    f32::from(bb.y_min) * units,
                    f32::from(bb.x_max) * units,
                    f32::from(bb.y_max) * units,
                ))
                .italic_angle(font.face.italic_angle())
                .ascent(f32::from(font.face.ascender()) * units)
                .descent(f32::from(font.face.descender()) * units)
                .cap_height(f32::from(font.face.capital_height().unwrap_or(font.face.ascender())) * units)
                .stem_v(if font.face.is_bold() { 120. } else { 80. })
                .font_file2(file);
        }
        Ok(refs)
    }
}

impl Font<'_> {
    // Distinct strings (or Arabic dots shared by several letters) can shape to
    // the same glyph. Give them separate CMaps over the same embedded subset.
    fn map_unicode(&mut self, gid: u16, text: &str) -> usize {
        let encoding = self.unicode.iter().position(|map| map.get(&gid).is_none_or(|value| value == text)).unwrap_or_else(|| {
            self.unicode.push(BTreeMap::new());
            self.unicode.len() - 1
        });
        self.unicode[encoding].entry(gid).or_insert_with(|| text.to_owned());
        encoding
    }
}

// Default-ignorable controls affect shaping or bidi, but do not need a cmap entry.
fn is_format_only(ch: char) -> bool {
    matches!(ch, '\u{00ad}' | '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{fe00}'..='\u{fe0f}' | '\u{e0100}'..='\u{e01ef}')
}

enum TjItem {
    Bytes(Vec<u8>),
    Adjust(f32),
}

struct TextBatch {
    key: (FontId, f64),
    y: f64,
    pen_x: f64,
    items: Vec<TjItem>,
}

fn flush_batch(
    content: &mut Content,
    batch: Option<TextBatch>,
    sy: f64,
    active_font: &mut Option<(FontId, f64)>,
    names: &BTreeMap<usize, String>,
) {
    let Some(batch) = batch else {
        return;
    };
    if *active_font != Some(batch.key) {
        content.set_font(Name(batch.key.0.resource_name(names).as_bytes()), batch.key.1 as f32);
        *active_font = Some(batch.key);
    }
    let _ = sy;
    if batch.items.len() == 1 {
        if let TjItem::Bytes(bytes) = &batch.items[0] {
            content.show(Str(bytes));
            return;
        }
    }
    let mut positioned = content.show_positioned();
    let mut items = positioned.items();
    for item in &batch.items {
        match item {
            TjItem::Bytes(bytes) => {
                items.show(Str(bytes));
            }
            TjItem::Adjust(adj) => {
                items.adjust(*adj);
            }
        }
    }
}

impl ShapedText {
    pub fn write(&self, content: &mut Content, names: &BTreeMap<usize, String>, [x, baseline]: [f64; 2], [sx, sy]: [f64; 2]) {
        content.set_horizontal_scaling((100. * sx) as f32);
        let mut active_font = None;
        let mut batch: Option<TextBatch> = None;
        for cluster in &self.clusters {
            if let Some(actual) = cluster.actual_text.as_deref() {
                flush_batch(content, batch.take(), sy, &mut active_font, names);
                begin_actual_text(content, actual);
                for glyph in &cluster.glyphs {
                    let id = FontId { face: cluster.font, encoding: glyph.encoding };
                    let key = (id, cluster.size);
                    if active_font != Some(key) {
                        content.set_font(Name(id.resource_name(names).as_bytes()), cluster.size as f32);
                        active_font = Some(key);
                    }
                    content.set_text_matrix([1., 0., 0., -(sy as f32), (x + glyph.x * sx) as f32, (baseline + glyph.y * sy) as f32]);
                    content.show(Str(&glyph.cid.to_be_bytes()));
                }
                content.end_marked_content();
                continue;
            }
            for glyph in &cluster.glyphs {
                let id = FontId { face: cluster.font, encoding: glyph.encoding };
                let key = (id, cluster.size);
                let gx = x + glyph.x * sx;
                let gy = baseline + glyph.y * sy;
                let can_extend = batch.as_ref().is_some_and(|b| b.key == key && (gy - b.y).abs() < 1e-3 && gx >= b.pen_x - 0.01);
                if can_extend {
                    let b = batch.as_mut().unwrap();
                    let scale = cluster.size * sx;
                    let dx = gx - b.pen_x;
                    let adj = if scale.abs() > f64::EPSILON { (-dx / scale * 1000.0) as f32 } else { 0. };
                    let mut extra = 0.;
                    if adj.abs() >= 0.5 {
                        b.items.push(TjItem::Adjust(adj));
                        extra = -(f64::from(adj) / 1000.0) * scale;
                    }
                    if let Some(TjItem::Bytes(bytes)) = b.items.last_mut() {
                        bytes.extend_from_slice(&glyph.cid.to_be_bytes());
                    } else {
                        b.items.push(TjItem::Bytes(glyph.cid.to_be_bytes().to_vec()));
                    }
                    b.pen_x += extra + glyph.advance * sx;
                } else {
                    flush_batch(content, batch.take(), sy, &mut active_font, names);
                    if active_font != Some(key) {
                        content.set_font(Name(id.resource_name(names).as_bytes()), cluster.size as f32);
                        active_font = Some(key);
                    }
                    content.set_text_matrix([1., 0., 0., -(sy as f32), gx as f32, gy as f32]);
                    batch = Some(TextBatch {
                        key,
                        y: gy,
                        pen_x: gx + glyph.advance * sx,
                        items: vec![TjItem::Bytes(glyph.cid.to_be_bytes().to_vec())],
                    });
                }
            }
        }
        flush_batch(content, batch, sy, &mut active_font, names);
    }
}

fn begin_actual_text(content: &mut Content, text: &str) {
    let bytes: Vec<_> = [0xfe, 0xff].into_iter().chain(text.encode_utf16().flat_map(u16::to_be_bytes)).collect();
    content.begin_marked_content_with_properties(Name(b"Span")).properties().pair(Name(b"ActualText"), Str(&bytes));
}
