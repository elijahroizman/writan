use std::borrow::Borrow;
use std::collections::{BTreeMap, HashMap};

use pdf_writer::{
    types::{ActionType, AnnotationType, FunctionShadingType, LineCapStyle, LineJoinStyle, MaskType, TextRenderingMode},
    Content, Filter, Finish, Name, Pdf, Rect, Ref, Str, TextStr,
};

use crate::color::{Color, Gradient};
use crate::fonts::{FontSpec, Fonts};
use crate::model::{PaintItemInput, PaintStyleInput, PdfDocumentInput, PdfMetadata};
use crate::svg_path::{self, Geometry, Matrix};
use crate::{flate, image_pdf};

pub fn render_document_to_pdf(
    input: &PdfDocumentInput,
    media: &HashMap<String, Vec<u8>>,
    fonts: &HashMap<String, Vec<u8>>,
) -> Result<Vec<u8>, String> {
    render_document_to_pdf_with(input, media, fonts)
}

pub fn render_document_to_pdf_slices(
    input: &PdfDocumentInput,
    media: &HashMap<&str, &[u8]>,
    fonts: &HashMap<&str, &[u8]>,
) -> Result<Vec<u8>, String> {
    render_document_to_pdf_with(input, media, fonts)
}

fn render_document_to_pdf_with<KM: Borrow<str>, VM: AsRef<[u8]>, KF: Borrow<str> + Ord, VF: AsRef<[u8]>>(
    input: &PdfDocumentInput,
    media: &HashMap<KM, VM>,
    fonts: &HashMap<KF, VF>,
) -> Result<Vec<u8>, String> {
    let mut engine = Engine::new(media, fonts);
    let specs: Vec<_> = input.styles.iter().map(|s| FontSpec::parse(&s.font)).collect();
    for page in &input.pages {
        let mut drawing = PageDrawing::new(page.width_pt, page.height_pt)?;
        if ![page.scale_x, page.scale_y, page.offset_x_pt, page.offset_y_pt].iter().all(|v| v.is_finite() && v.abs() <= 1e12)
            || page.scale_x <= 0.
            || page.scale_y <= 0.
        {
            return Err("Invalid PDF page transform".into());
        }
        let transform = Matrix([page.scale_x, 0., 0., -page.scale_y, page.offset_x_pt, page.height_pt - page.offset_y_pt]);
        transform.apply(&mut drawing.content);
        if let Some(clip) = page.clip_rect_px {
            clip_rect(&mut drawing.content, clip);
        }
        engine.draw_items(&mut drawing, &page.items, &input.styles, &specs, transform)?;
        for svg in &page.svgs {
            engine.draw_svg(&mut drawing, &svg.svg, [svg.x_px, svg.y_px, svg.w_px, svg.h_px], None)?;
        }
        engine.add_page(drawing)?;
    }
    engine.finish(&input.metadata)
}

pub(crate) struct PageDrawing {
    pub content: Content,
    width: f64,
    height: f64,
    xobjects: BTreeMap<Ref, String>,
    states: BTreeMap<Ref, String>,
    shadings: BTreeMap<Ref, String>,
    annots: Vec<Ref>,
}

impl PageDrawing {
    pub fn new(width: f64, height: f64) -> Result<Self, String> {
        if ![width, height].iter().all(|v| v.is_finite() && *v > 0. && *v <= 1e12) {
            return Err("Invalid PDF page dimensions".into());
        }
        Ok(Self {
            content: Content::new(),
            width,
            height,
            xobjects: BTreeMap::new(),
            states: BTreeMap::new(),
            shadings: BTreeMap::new(),
            annots: Vec::new(),
        })
    }
}

pub(crate) struct Engine<'a> {
    pdf: Pdf,
    next: Ref,
    pages: Vec<Ref>,
    media: HashMap<&'a str, &'a [u8]>,
    images: HashMap<String, (Ref, u8)>,
    opacities: HashMap<(u32, u32), Ref>,
    pub fonts: Fonts<'a>,
    pub font_names: BTreeMap<usize, String>,
}

impl<'a> Engine<'a> {
    pub fn new<KM: Borrow<str>, VM: AsRef<[u8]>, KF: Borrow<str> + Ord, VF: AsRef<[u8]>>(
        media: &'a HashMap<KM, VM>,
        blobs: &'a HashMap<KF, VF>,
    ) -> Self {
        let mut pdf = Pdf::new();
        pdf.set_version(1, 7);
        let media = media.iter().map(|(k, v)| (k.borrow(), v.as_ref())).collect();
        let fonts = Fonts::new(blobs);
        let font_names = (0..fonts.len()).map(|id| (id, format!("F{id}"))).collect();
        Self { pdf, next: Ref::new(5), pages: Vec::new(), media, images: HashMap::new(), opacities: HashMap::new(), fonts, font_names }
    }

    pub fn add_page(&mut self, drawing: PageDrawing) -> Result<(), String> {
        let content_id = self.next.bump();
        let page_id = self.next.bump();
        let content = flate(&drawing.content.finish())?;
        self.pdf.stream(content_id, &content).filter(Filter::FlateDecode);
        let mut page = self.pdf.page(page_id);
        page.parent(Ref::new(2)).contents(content_id).media_box(Rect::new(0., 0., drawing.width as f32, drawing.height as f32));
        if !drawing.annots.is_empty() {
            page.annotations(drawing.annots);
        }
        let mut resources = page.resources();
        resources.pair(Name(b"Font"), Ref::new(4));
        if !drawing.xobjects.is_empty() {
            resources.x_objects().pairs(drawing.xobjects.iter().map(|(&id, name)| (Name(name.as_bytes()), id)));
        }
        if !drawing.states.is_empty() {
            resources.ext_g_states().pairs(drawing.states.iter().map(|(&id, name)| (Name(name.as_bytes()), id)));
        }
        if !drawing.shadings.is_empty() {
            resources.shadings().pairs(drawing.shadings.iter().map(|(&id, name)| (Name(name.as_bytes()), id)));
        }
        resources.finish();
        page.finish();
        self.pages.push(page_id);
        Ok(())
    }

    pub fn finish(mut self, metadata: &PdfMetadata) -> Result<Vec<u8>, String> {
        if self.pages.is_empty() {
            return Err("PDF requires at least one page".into());
        }
        let font_refs = self.fonts.embed(&mut self.pdf, &mut self.next)?;
        let mut font_dict = self.pdf.indirect(Ref::new(4)).dict();
        for (key, id) in font_refs {
            font_dict.pair(Name(key.resource_name(&self.font_names).as_bytes()), id);
        }
        font_dict.finish();
        self.pdf.catalog(Ref::new(1)).pages(Ref::new(2));
        self.pdf.pages(Ref::new(2)).kids(self.pages.iter().copied()).count(self.pages.len() as i32);
        let mut info = self.pdf.document_info(Ref::new(3));
        info.producer(TextStr("Writan PDF Engine"));
        if let Some(title) = &metadata.title {
            info.title(TextStr(title));
        }
        if let Some(author) = &metadata.author {
            info.author(TextStr(author));
        }
        if let Some(subject) = &metadata.subject {
            info.subject(TextStr(subject));
        }
        if let Some(creator) = &metadata.creator {
            info.creator(TextStr(creator));
        }
        info.finish();
        Ok(self.pdf.finish())
    }

    fn set_opacity(&mut self, page: &mut PageDrawing, fill: f32, stroke: f32) {
        if fill == 1. && stroke == 1. {
            return;
        }
        let state = *self.opacities.entry((fill.to_bits(), stroke.to_bits())).or_insert_with(|| {
            let id = self.next.bump();
            self.pdf.ext_graphics(id).non_stroking_alpha(fill).stroking_alpha(stroke);
            id
        });
        apply_state(page, state);
    }

    pub fn draw_image(&mut self, page: &mut PageDrawing, key: &str, rect: [f64; 4], crop: Option<[f64; 4]>) -> Result<(), String> {
        let Some(bytes) = self.media.get(key) else {
            return Ok(());
        };
        if let Ok(svg) = std::str::from_utf8(bytes) {
            let prefix = svg.trim_start_matches(['\u{feff}', ' ', '\r', '\n', '\t']);
            if prefix.starts_with('<') {
                return self.draw_svg(page, prefix, rect, crop);
            }
        }
        let (id, orientation) = match self.images.get(key) {
            Some(&entry) => entry,
            None => {
                let entry =
                    image_pdf::embed(&mut self.pdf, &mut self.next, bytes).map_err(|e| format!("Could not embed image {key}: {e}"))?;
                self.images.insert(key.to_owned(), entry);
                entry
            }
        };
        let name = page.xobjects.entry(id).or_insert_with(|| format!("Im{}", id.get()));
        page.content.save_state();
        if crop.is_some() {
            clip_rect(&mut page.content, rect);
        }
        let [x, y, w, h] = cropped_rect(rect, crop);
        let matrix = match orientation {
            2 => Matrix([-w, 0., 0., -h, x + w, y + h]),
            3 => Matrix([-w, 0., 0., h, x + w, y]),
            4 => Matrix([w, 0., 0., h, x, y]),
            5 => Matrix([0., h, -w, 0., x + w, y]),
            6 => Matrix([0., h, w, 0., x, y]),
            7 => Matrix([0., -h, w, 0., x, y + h]),
            8 => Matrix([0., -h, -w, 0., x + w, y + h]),
            _ => Matrix([w, 0., 0., -h, x, y + h]),
        };
        matrix.apply(&mut page.content);
        page.content.x_object(Name(name.as_bytes())).restore_state();
        Ok(())
    }

    fn draw_svg(&mut self, page: &mut PageDrawing, svg: &str, rect: [f64; 4], crop: Option<[f64; 4]>) -> Result<(), String> {
        let [x, y, w, h] = cropped_rect(rect, crop);
        if w <= 0. || h <= 0. {
            return Ok(());
        }
        // Like the canvas export before it, skip a picture or chart this parser can't read
        // instead of failing the whole document. The exporter rasterizes what it can foresee.
        let Ok(elements) = svg_path::translate(svg, w, h) else {
            return Ok(());
        };
        page.content.save_state();
        clip_rect(&mut page.content, rect);
        for element in elements {
            page.content.save_state();
            Matrix::translate(x, y).then_local(element.transform).apply(&mut page.content);
            let style = element.style;
            match element.geometry {
                Geometry::Text { text, x, y, font, anchor } => {
                    if let Some(color) = visible(style.fill_color()) {
                        let shaped = self.fonts.shape(&text, &font, None, false, 0.)?;
                        self.set_opacity(page, color.alpha(), 1.);
                        fill_rgb(&mut page.content, color);
                        page.content.begin_text();
                        shaped.write(&mut page.content, &self.font_names, [x - shaped.width * anchor, y], [1., 1.]);
                        page.content.end_text();
                    }
                }
                geometry => {
                    let fill = if matches!(geometry, Geometry::Line(_)) { None } else { visible(style.fill_color()) };
                    let stroke = visible(style.stroke_color()).filter(|_| style.stroke_width > 0.);
                    if fill.is_some() || stroke.is_some() {
                        self.path_style(page, fill, stroke, style.stroke_width, style.dash.as_deref());
                        if style.line_cap.as_deref() == Some("round") {
                            page.content.set_line_cap(LineCapStyle::RoundCap);
                        } else if style.line_cap.as_deref() == Some("square") {
                            page.content.set_line_cap(LineCapStyle::ProjectingSquareCap);
                        }
                        if style.line_join.as_deref() == Some("round") {
                            page.content.set_line_join(LineJoinStyle::RoundJoin);
                        } else if style.line_join.as_deref() == Some("bevel") {
                            page.content.set_line_join(LineJoinStyle::BevelJoin);
                        }
                        geometry.write_path(&mut page.content)?;
                        paint_path(&mut page.content, fill, stroke, style.even_odd);
                    }
                }
            }
            page.content.restore_state();
        }
        page.content.restore_state();
        Ok(())
    }

    fn path_style(&mut self, page: &mut PageDrawing, fill: Option<Color>, stroke: Option<Color>, width: f64, dash: Option<&str>) {
        self.set_opacity(page, fill.map_or(1., Color::alpha), stroke.map_or(1., Color::alpha));
        if let Some(color) = fill {
            fill_rgb(&mut page.content, color);
        }
        if let Some(color) = stroke {
            stroke_rgb(&mut page.content, color);
            page.content.set_line_width(width as f32);
            set_dash(&mut page.content, dash, width);
        }
    }

    fn draw_items(
        &mut self,
        drawing: &mut PageDrawing,
        items: &[PaintItemInput],
        styles: &[PaintStyleInput],
        specs: &[FontSpec],
        transform: Matrix,
    ) -> Result<(), String> {
        for item in items {
            match item {
                PaintItemInput::Rect { x, y, w, h, fill } => {
                    self.draw_rect(drawing, [*x, *y, *w, *h], fill.as_deref())?;
                }
                PaintItemInput::Line { x1, y1, x2, y2, color, w, dash } => {
                    if let Some(color) =
                        visible(Color::parse(color)).filter(|_| *w > 0. && !matches!(dash.as_deref(), Some("none" | "hidden")))
                    {
                        drawing.content.save_state();
                        self.path_style(drawing, None, Some(color), *w, dash.as_deref());
                        if dash.as_deref() == Some("double") {
                            let length = (x2 - x1).hypot(y2 - y1);
                            if length > 0. {
                                let (dx, dy) = (-(y2 - y1) * w / (3. * length), (x2 - x1) * w / (3. * length));
                                drawing.content.set_line_width((w / 3.) as f32);
                                line(&mut drawing.content, [x1 + dx, y1 + dy, x2 + dx, y2 + dy]);
                                line(&mut drawing.content, [x1 - dx, y1 - dy, x2 - dx, y2 - dy]);
                            }
                        } else {
                            line(&mut drawing.content, [*x1, *y1, *x2, *y2]);
                        }
                        drawing.content.restore_state();
                    }
                }
                PaintItemInput::Image { x, y, w, h, media, crop, .. } => {
                    self.draw_image(drawing, media, [*x, *y, *w, *h], *crop)?;
                }
                PaintItemInput::Path { x, y, d, fill, stroke, sw, dash, fill_rule } => {
                    let fill = visible(fill.as_deref().and_then(Color::parse));
                    let width = sw.unwrap_or(1.);
                    let stroke = visible(stroke.as_deref().and_then(Color::parse)).filter(|_| width > 0.);
                    if fill.is_some() || stroke.is_some() {
                        drawing.content.save_state();
                        Matrix::translate(*x, *y).apply(&mut drawing.content);
                        self.path_style(drawing, fill, stroke, width, dash.as_deref());
                        svg_path::path(&mut drawing.content, d)?;
                        paint_path(&mut drawing.content, fill, stroke, fill_rule.as_deref() == Some("evenodd"));
                        drawing.content.restore_state();
                    }
                }
                PaintItemInput::Group { x, y, w, h, rot, fh, fv, clip, items, .. } => {
                    // The header/footer dimming group is editor chrome. Groups have no export opacity.
                    let local = Matrix::translate(x + w / 2., y + h / 2.)
                        .then_local(Matrix::rotate(rot.unwrap_or(0.)))
                        .then_local(Matrix::scale(if *fh { -1. } else { 1. }, if *fv { -1. } else { 1. }))
                        .then_local(Matrix::translate(-w / 2., -h / 2.));
                    drawing.content.save_state();
                    local.apply(&mut drawing.content);
                    if *clip {
                        clip_rect(&mut drawing.content, [0., 0., *w, *h]);
                    }
                    self.draw_items(drawing, items, styles, specs, transform.then_local(local))?;
                    drawing.content.restore_state();
                }
                PaintItemInput::Text { x, y, w, s, text, ws, link, rtl, invisible } => {
                    let style = styles.get(*s).ok_or_else(|| format!("Unknown PDF paint style {s}"))?;
                    let spec = &specs[*s];
                    let shaped = self.fonts.shape(text, spec, Some(*rtl), style.small_caps, ws.unwrap_or(0.))?;
                    let baseline = y - style.shift.unwrap_or(0.);
                    let ascent = style.ascent.unwrap_or(shaped.ascent);
                    let descent = style.descent.unwrap_or(shaped.descent);
                    let width = if *w > 0. { *w } else { shaped.width };
                    let rect = [*x, baseline - ascent, width, ascent + descent];
                    if !*invisible {
                        self.draw_rect(drawing, rect, style.background.as_deref())?;
                    }
                    let color = Color::parse(&style.color).unwrap_or(Color::BLACK);
                    if *invisible || color.alpha() > 0. {
                        let (draw_x, sx) = if *w > 0. && shaped.width > 0. {
                            if shaped.width > *w * 1.005 {
                                (*x, (*w / shaped.width).max(0.5))
                            } else if *rtl && (*w - shaped.width) > 0.5 {
                                (*x + (*w - shaped.width), 1.0)
                            } else {
                                (*x, 1.0)
                            }
                        } else {
                            (*x, 1.0)
                        };
                        drawing.content.save_state();
                        if !*invisible {
                            self.path_style(drawing, Some(color), Some(color), spec.size / 16., None);
                        }
                        drawing.content.begin_text();
                        if *invisible {
                            drawing.content.set_text_rendering_mode(TextRenderingMode::Invisible);
                        }
                        shaped.write(&mut drawing.content, &self.font_names, [draw_x, baseline], [sx, 1.]);
                        drawing.content.end_text();
                        if color.alpha() > 0. {
                            if let Some(decoration) = &style.decoration {
                                if *invisible {
                                    self.path_style(drawing, Some(color), Some(color), spec.size / 16., None);
                                }
                                draw_decoration(&mut drawing.content, [*x, baseline, width, spec.size], decoration);
                            }
                        }
                        drawing.content.restore_state();
                    }
                    if let Some(uri) = link.as_deref().filter(|uri| safe_uri(uri)) {
                        self.link(drawing, transform, rect, uri);
                    }
                }
            }
        }
        Ok(())
    }

    fn link(&mut self, page: &mut PageDrawing, transform: Matrix, [x, y, w, h]: [f64; 4], uri: &str) {
        let points = [transform.point(x, y), transform.point(x + w, y), transform.point(x, y + h), transform.point(x + w, y + h)];
        let [x1, y1, x2, y2] =
            points.iter().fold([page.width, page.height, 0., 0.], |[l, b, r, t], &[x, y]| [l.min(x), b.min(y), r.max(x), t.max(y)]);
        let rect = [x1.max(0.), y1.max(0.), x2.min(page.width), y2.min(page.height)];
        if rect[0] >= rect[2] || rect[1] >= rect[3] {
            return;
        }
        let id = self.next.bump();
        let mut annot = self.pdf.annotation(id);
        let [l, b, r, t] = rect.map(|v| v as f32);
        annot.subtype(AnnotationType::Link).rect(Rect::new(l, b, r, t)).border(0., 0., 0., None);
        annot.action().action_type(ActionType::Uri).uri(Str(uri.as_bytes()));
        page.annots.push(id);
    }

    fn draw_rect(&mut self, page: &mut PageDrawing, rect: [f64; 4], fill: Option<&str>) -> Result<(), String> {
        let Some(fill) = fill else {
            return Ok(());
        };
        let [x, y, w, h] = rect;
        if w <= 0. || h <= 0. {
            return Ok(());
        }
        if let Some(color) = visible(Color::parse(fill)) {
            page.content.save_state();
            self.path_style(page, Some(color), None, 0., None);
            rectangle(&mut page.content, rect);
            page.content.fill_nonzero().restore_state();
        } else if let Some(gradient) = Gradient::parse(fill, w, h) {
            let [dx, dy] = gradient.direction;
            let length = (dx.abs() * w + dy.abs() * h) / 2.;
            let center = [x + w / 2., y + h / 2.];
            let coords = [center[0] - dx * length, center[1] - dy * length, center[0] + dx * length, center[1] + dy * length];
            let id = self.shading(&gradient, coords, false);
            let name = format!("Sh{}", id.get());
            page.content.save_state();
            clip_rect(&mut page.content, rect);
            let alpha = gradient.stops[0].1.alpha();
            if gradient.stops.iter().all(|(_, c)| c.alpha() == alpha) {
                self.set_opacity(page, alpha, 1.);
            } else {
                let shade = self.shading(&gradient, coords, true);
                let form_ref = self.next.bump();
                let mut mask = Content::new();
                mask.shading(Name(b"S"));
                let data = flate(&mask.finish())?;
                let mut form = self.pdf.form_xobject(form_ref, &data);
                form.filter(Filter::FlateDecode);
                form.bbox(Rect::new(x as f32, y as f32, (x + w) as f32, (y + h) as f32));
                form.resources().shadings().pair(Name(b"S"), shade);
                form.group().transparency().isolated(true).color_space().device_gray();
                form.finish();
                let state = self.next.bump();
                self.pdf.ext_graphics(state).soft_mask().subtype(MaskType::Luminosity).group(form_ref);
                apply_state(page, state);
            }
            page.content.shading(Name(name.as_bytes())).restore_state();
            page.shadings.insert(id, name);
        }
        Ok(())
    }

    fn shading(&mut self, gradient: &Gradient, [x1, y1, x2, y2]: [f64; 4], alpha: bool) -> Ref {
        let mut stops = gradient.stops.clone();
        let first = stops[0];
        let last = stops[stops.len() - 1];
        if first.0 > 0. {
            stops.insert(0, (0., first.1));
        }
        if last.0 < 1. {
            stops.push((1., last.1));
        }
        let min = stops[0].0;
        let max = stops[stops.len() - 1].0;
        let offset = |v: f64| ((v - min) / (max - min)) as f32;
        let components = |color: Color| {
            if alpha {
                vec![color.alpha()]
            } else {
                color.0[..3].to_vec()
            }
        };
        let mut functions = Vec::new();
        let mut bounds = Vec::new();
        for pair in stops.windows(2) {
            let [(start, a), (end, b)] = [pair[0], pair[1]];
            if offset(start) >= offset(end) {
                continue;
            }
            let id = self.next.bump();
            self.pdf.exponential_function(id).domain([0., 1.]).c0(components(a)).c1(components(b)).n(1.);
            functions.push(id);
            bounds.push(offset(end));
        }
        bounds.pop();
        let function = self.next.bump();
        self.pdf
            .stitching_function(function)
            .domain([0., 1.])
            .functions(functions.iter().copied())
            .bounds(bounds)
            .encode((0..functions.len()).flat_map(|_| [0., 1.]));
        let id = self.next.bump();
        let mut shading = self.pdf.function_shading(id);
        shading
            .shading_type(FunctionShadingType::Axial)
            .function(function)
            .coords([x1 + (x2 - x1) * min, y1 + (y2 - y1) * min, x1 + (x2 - x1) * max, y1 + (y2 - y1) * max].map(|v| v as f32))
            .extend([true, true]);
        if alpha {
            shading.color_space().device_gray();
        } else {
            shading.color_space().device_rgb();
        }
        id
    }
}

fn cropped_rect([x, y, w, h]: [f64; 4], crop: Option<[f64; 4]>) -> [f64; 4] {
    let [l, t, r, b] = crop.unwrap_or([0.; 4]);
    let iw = w / (1. - l - r).max(0.01);
    let ih = h / (1. - t - b).max(0.01);
    [x - l * iw, y - t * ih, iw, ih]
}

fn visible(color: Option<Color>) -> Option<Color> {
    color.filter(|c| c.alpha() > 0.)
}

fn apply_state(page: &mut PageDrawing, id: Ref) {
    let name = page.states.entry(id).or_insert_with(|| format!("Gs{}", id.get()));
    page.content.set_parameters(Name(name.as_bytes()));
}

fn fill_rgb(content: &mut Content, Color([r, g, b, _]): Color) {
    content.set_fill_rgb(r, g, b);
}

fn stroke_rgb(content: &mut Content, Color([r, g, b, _]): Color) {
    content.set_stroke_rgb(r, g, b);
}

fn rectangle(content: &mut Content, [x, y, w, h]: [f64; 4]) {
    content.rect(x as f32, y as f32, w as f32, h as f32);
}

fn clip_rect(content: &mut Content, rect: [f64; 4]) {
    rectangle(content, rect);
    content.clip_nonzero().end_path();
}

fn paint_path(content: &mut Content, fill: Option<Color>, stroke: Option<Color>, even_odd: bool) {
    match (fill.is_some(), stroke.is_some(), even_odd) {
        (true, true, true) => content.fill_even_odd_and_stroke(),
        (true, true, false) => content.fill_nonzero_and_stroke(),
        (true, false, true) => content.fill_even_odd(),
        (true, false, false) => content.fill_nonzero(),
        (false, true, _) => content.stroke(),
        _ => content.end_path(),
    };
}

fn line(content: &mut Content, [x1, y1, x2, y2]: [f64; 4]) {
    content.move_to(x1 as f32, y1 as f32).line_to(x2 as f32, y2 as f32).stroke();
}

fn set_dash(content: &mut Content, dash: Option<&str>, width: f64) {
    let width = width as f32;
    let pattern = match dash {
        Some("dash" | "dashed") => vec![width * 3., width * 2.],
        Some("dot" | "sysDot" | "dotted") => {
            content.set_line_cap(LineCapStyle::RoundCap);
            vec![0., width * 2.]
        }
        Some(value) => value
            .split(|ch: char| ch == ',' || ch.is_whitespace())
            .filter(|s| !s.is_empty())
            .map(|value| value.parse::<f32>().ok().filter(|v| v.is_finite() && *v >= 0.))
            .collect::<Option<Vec<_>>>()
            .unwrap_or_default(),
        None => return,
    };
    if pattern.iter().any(|v| *v > 0.) {
        content.set_dash_pattern(pattern, 0.);
    }
}

fn draw_decoration(content: &mut Content, [x, y, w, size]: [f64; 4], decoration: &str) {
    let thickness = (size / 16.).max(0.4);
    let tokens: Vec<_> = decoration.split_whitespace().collect();
    let ends = [("underline", y + size * 0.115), ("line-through", y - size * 0.28), ("overline", y - size * 0.78)];
    if let Some(color) = tokens.iter().find_map(|s| Color::parse(s)) {
        stroke_rgb(content, color);
    }
    let dash = tokens.iter().find(|&&s| matches!(s, "dashed" | "dotted")).copied();
    content.set_line_width(thickness as f32);
    set_dash(content, dash, thickness);
    for (kind, baseline) in ends {
        if !tokens.contains(&kind) {
            continue;
        }
        if tokens.contains(&"wavy") && w > 0. {
            let count = (w / (4. * thickness)).ceil().clamp(1., 10_000.) as usize;
            let step = w / count as f64;
            content.move_to(x as f32, baseline as f32);
            for i in 0..count {
                let left = x + i as f64 * step;
                let bend = if i % 2 == 0 { thickness } else { -thickness };
                content.cubic_to(
                    (left + step / 3.) as f32,
                    (baseline + bend) as f32,
                    (left + 2. * step / 3.) as f32,
                    (baseline + bend) as f32,
                    (left + step) as f32,
                    baseline as f32,
                );
            }
            content.stroke();
        } else {
            line(content, [x, baseline, x + w, baseline]);
            if tokens.contains(&"double") {
                line(content, [x, baseline + 1.8 * thickness, x + w, baseline + 1.8 * thickness]);
            }
        }
    }
}

fn safe_uri(uri: &str) -> bool {
    if uri.chars().any(|ch| ch.is_control() || ch.is_whitespace() || ch == '\\') {
        return false;
    }
    let Some((scheme, rest)) = uri.split_once(':') else {
        return false;
    };
    if scheme.eq_ignore_ascii_case("mailto") {
        !rest.is_empty()
    } else if scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http") {
        rest.strip_prefix("//").is_some_and(|site| !site.is_empty() && !site.starts_with(['/', '?', '#']))
    } else {
        false
    }
}
