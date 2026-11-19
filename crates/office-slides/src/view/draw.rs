//! Drawing slides: backgrounds, the layout's and master's shapes, and each frame (its shape's
//! outline, fill and line, its text, a picture, a table, a group's frames) as display items.

use office_core::drawing::{Color, Fill, Line, Xfrm};
use office_core::model::{Document, Frame, FrameContent, Slide, Table};
use office_core::xml::{ns, Element};
use office_engine::display::{self, r, Item, NoObjects, Sink};
use office_engine::flow::{self, FlowLayout, Keep, Resolver, TableLook};
use office_engine::geometry::{self, PathFill};
use office_engine::para::{Ctx, ParaSpec};
use office_engine::pos::Path;

use super::art::ArtSet;
use super::table_style::{self, TableStyles};
use super::text::{self, emu_px, Body, Paint};

/// A frame's box on the slide, in pixels.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Boxed {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub rot: f64,
    pub flip_h: bool,
    pub flip_v: bool,
}

impl Boxed {
    pub fn of(x: &Xfrm) -> Boxed {
        Boxed {
            x: emu_px(x.x),
            y: emu_px(x.y),
            w: emu_px(x.width),
            h: emu_px(x.height),
            rot: x.rotation.degrees(),
            flip_h: x.flip_h,
            flip_v: x.flip_v,
        }
    }

    /// Whether a slide point is inside, taking rotation into account.
    pub fn contains(&self, px: f64, py: f64, slack: f64) -> bool {
        let (lx, ly) = self.to_local(px, py);
        lx >= -slack && ly >= -slack && lx <= self.w + slack && ly <= self.h + slack
    }

    /// A slide point in the box's own coordinates (unrotated, from its top left).
    pub fn to_local(&self, px: f64, py: f64) -> (f64, f64) {
        let (cx, cy) = (self.x + self.w / 2.0, self.y + self.h / 2.0);
        let a = -self.rot.to_radians();
        let (dx, dy) = (px - cx, py - cy);
        let (mut lx, mut ly) = (dx * a.cos() - dy * a.sin(), dx * a.sin() + dy * a.cos());
        if self.flip_h {
            lx = -lx;
        }
        if self.flip_v {
            ly = -ly;
        }
        (lx + self.w / 2.0, ly + self.h / 2.0)
    }
}

/// A group's mapping from its children's coordinates to the slide's.
#[derive(Copy, Clone, Debug)]
pub struct Map {
    pub dx: f64,
    pub dy: f64,
    pub sx: f64,
    pub sy: f64,
}

impl Map {
    pub const IDENTITY: Map = Map { dx: 0.0, dy: 0.0, sx: 1.0, sy: 1.0 };

    pub fn apply(&self, b: Boxed) -> Boxed {
        Boxed { x: self.dx + b.x * self.sx, y: self.dy + b.y * self.sy, w: b.w * self.sx, h: b.h * self.sy, ..b }
    }

    /// The mapping for a group's children, from the group's own `a:xfrm`.
    pub fn child(&self, group: &Xfrm) -> Map {
        let outer = self.apply(Boxed::of(group));
        let (cx, cy, cw, ch) =
            group.child.map(|(a, b, c, d)| (emu_px(a), emu_px(b), emu_px(c), emu_px(d))).unwrap_or((outer.x, outer.y, outer.w, outer.h));
        let sx = if cw > 0.0 { outer.w / cw } else { 1.0 };
        let sy = if ch > 0.0 { outer.h / ch } else { 1.0 };
        Map { dx: outer.x - cx * sx, dy: outer.y - cy * sy, sx, sy }
    }
}

/// Drawing a slide needs the document, the template art, and the engine's measuring.
pub struct Drawer<'a, 'b, 'c> {
    pub doc: &'a Document,
    pub art: &'a ArtSet,
    pub tables: &'a TableStyles,
    pub ctx: &'b mut Ctx<'c>,
    pub cache: &'b mut flow::Cache,
}

/// What a frame resolves to, for drawing and for the inspector.
pub struct Look {
    pub fill: Option<String>,
    pub line: Option<(String, f64, &'static str)>,
    pub text_color: Option<Color>,
}

impl Drawer<'_, '_, '_> {
    /// Everything on a slide: background, template shapes, then the slide's own frames.
    pub fn slide(&mut self, slide: &Slide, out: &mut Vec<Item>, w: f64, h: f64) {
        let paint = Paint { doc: self.doc, slide };
        let (layout, master) = self.art.chain(slide.layout.as_deref());
        let bg = slide
            .origin
            .get()
            .and_then(|o| o.path(ns::P, &["cSld", "bg"]))
            .or_else(|| layout.and_then(|l| l.background.as_ref()))
            .or_else(|| master.and_then(|m| m.background.as_ref()));
        out.push(Item::Rect { x: 0.0, y: 0.0, w: r(w), h: r(h), fill: Some(background(&paint, bg).unwrap_or_else(|| "#ffffff".into())) });
        if let Some(pic) = bg
            .and_then(|b| b.descendant(ns::A, "blip"))
            .and_then(|b| b.attr_ns(ns::R, "embed"))
            .and_then(|id| self.bg_media(slide, layout.is_some(), id))
        {
            out.push(Item::Image { x: 0.0, y: 0.0, w: r(w), h: r(h), media: pic, crop: None, id: None });
        }
        let show_master = slide.origin.get().and_then(|o| o.attr("showMasterSp")).and_then(office_core::units::parse_bool).unwrap_or(true)
            && layout.is_none_or(|l| l.show_master);
        let mut art_items = Vec::new();
        if show_master {
            if let Some(m) = master {
                for f in &m.frames {
                    self.frame(slide, layout.and_then(|l| l.master.as_deref()), f, Map::IDENTITY, None, &mut art_items);
                }
            }
        }
        if let Some(l) = layout {
            for f in &l.frames {
                self.frame(slide, slide.layout.as_deref(), f, Map::IDENTITY, None, &mut art_items);
            }
        }
        out.extend(art_items);
        for f in &slide.frames {
            self.frame(slide, slide.relationship_part(), f, Map::IDENTITY, Some(f.id), out);
        }
    }

    /// The media id of a background picture: a part in the package, by relationship.
    fn bg_media(&self, slide: &Slide, _from_template: bool, rid: &str) -> Option<String> {
        let pkg = self.doc.package.as_ref()?;
        for part in [slide.part.as_deref(), slide.layout.as_deref()].into_iter().flatten() {
            if let Ok(rels) = pkg.rels(part) {
                if let Some(rel) = rels.get(rid) {
                    return pkg.target(part, rel);
                }
            }
        }
        None
    }

    /// A frame's resolved box.
    pub fn boxed(&self, slide: &Slide, f: &Frame, map: Map) -> Boxed {
        let x = crate::resolve(self.doc, slide, f).xfrm.or_else(|| f.shape.xfrm.clone()).unwrap_or_default();
        map.apply(Boxed::of(&x))
    }

    /// How a frame's fill, line and text color resolve.
    pub fn look(&self, slide: &Slide, f: &Frame) -> Look {
        let paint = Paint { doc: self.doc, slide };
        let c = text::cascade(self.doc, self.art, slide, f);
        let style = f.origin.get().and_then(|o| o.child(ns::P, "style")).or_else(|| c.styles.last().copied());
        let style_color = |r: &str| {
            style.and_then(|s| s.child(ns::A, r)).filter(|e| e.attr("idx").is_none_or(|i| i != "0")).and_then(Color::in_drawingml)
        };
        let inherited_fill = || c.sp_prs.iter().rev().find_map(|s| Fill::in_children(s));
        let fill = match f.shape.fill.clone().or_else(inherited_fill) {
            Some(fill) => fill_css(&paint, &fill),
            None => style_color("fillRef").map(|c| paint.color(&c).css()),
        };
        let inherited_line = || c.sp_prs.iter().rev().find_map(|s| s.child(ns::A, "ln")).map(Line::parse);
        let theme_width = |idx: usize| {
            self.doc
                .theme
                .as_ref()
                .and_then(|t| t.format_scheme.as_ref())
                .and_then(|fs| fs.child(ns::A, "lnStyleLst"))
                .and_then(|l| l.children_named(ns::A, "ln").nth(idx.saturating_sub(1)))
                .and_then(|ln| ln.attr("w"))
                .and_then(|w| w.parse::<f64>().ok())
                .map(|w| w / 9525.0)
        };
        let line = match f.shape.line.clone().or_else(inherited_line) {
            Some(l) => line_css(&paint, &l, style_color("lnRef")),
            None => style.and_then(|s| s.child(ns::A, "lnRef")).filter(|e| e.attr("idx").is_some_and(|i| i != "0")).and_then(|lr| {
                let color = Color::in_drawingml(lr)?;
                let idx = lr.attr("idx").and_then(|i| i.parse().ok()).unwrap_or(1);
                Some((paint.color(&color).css(), theme_width(idx).unwrap_or(1.0), "solid"))
            }),
        };
        let text_color = style.and_then(|s| s.child(ns::A, "fontRef")).and_then(Color::in_drawingml);
        Look { fill, line, text_color }
    }

    /// Draws a frame. `id` is the top-level frame it belongs to (for selecting it), if any.
    fn frame(&mut self, slide: &Slide, part: Option<&str>, f: &Frame, map: Map, id: Option<u32>, out: &mut Vec<Item>) {
        if f.hidden {
            return;
        }
        let b = self.boxed(slide, f, map);
        let mut items = Vec::new();
        match &f.content {
            FrameContent::Group { frames } => {
                let x = f.shape.xfrm.clone().unwrap_or_default();
                let child = map.child(&x);
                // The children are placed in slide coordinates; the group item only rotates.
                let mut kids = Vec::new();
                for c in frames {
                    self.frame(slide, part, c, child, None, &mut kids);
                }
                for k in &mut kids {
                    shift(k, -b.x, -b.y);
                }
                items.extend(kids);
            }
            FrameContent::Shape { text, connector } => {
                let look = self.look(slide, f);
                self.outline(f, &b, &look, *connector, &mut items);
                if let Some(body) = text {
                    self.text(slide, f, body, &b, &look, &mut items);
                }
            }
            FrameContent::Picture { image } => {
                let crop = f.origin.get().and_then(|o| o.descendant(ns::A, "srcRect")).map(|rect| {
                    let v = |k: &str| rect.attr(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 100_000.0;
                    [v("l"), v("t"), v("r"), v("b")]
                });
                items.push(Item::Image { x: 0.0, y: 0.0, w: r(b.w), h: r(b.h), media: image.media.clone(), crop, id: None });
                let look = self.look(slide, f);
                if let Some((color, w, dash)) = look.line.filter(|_| f.shape.line.is_some()) {
                    items.push(Item::Path {
                        x: 0.0,
                        y: 0.0,
                        d: format!("M0 0H{0}V{1}H0Z", r(b.w), r(b.h)),
                        fill: None,
                        stroke: Some(color),
                        sw: r(w),
                        dash,
                    });
                }
            }
            FrameContent::Table { table } => self.table(slide, table, &b, &mut items),
            FrameContent::Raw { xml, chart_part } => {
                let target =
                    chart_part.clone().or_else(|| office_core::formats::pptx::chart_target(self.doc.package.as_ref()?, part?, xml));
                if let Some(chart) = target.and_then(|target| super::chart::draw(self.ctx, self.doc, slide, &target, b)) {
                    items.push(chart);
                } else {
                    self.raw(slide, part, xml, &b, &mut items);
                }
            }
        }
        out.push(Item::Group {
            x: r(b.x),
            y: r(b.y),
            w: r(b.w),
            h: r(b.h),
            rot: r(b.rot),
            fh: b.flip_h,
            fv: b.flip_v,
            clip: false,
            id: id.map(|i| i.to_string()),
            items,
        });
    }

    /// A shape's outline: its geometry, filled and stroked.
    fn outline(&self, f: &Frame, b: &Boxed, look: &Look, connector: bool, out: &mut Vec<Item>) {
        let o = f.origin.get();
        let sp_pr = o.and_then(|o| o.child(ns::P, "spPr"));
        let geom = if let Some(cust) = sp_pr.and_then(|s| s.child(ns::A, "custGeom")) {
            geometry::custom(cust, b.w, b.h)
        } else {
            let name = f.shape.preset.as_deref().unwrap_or(if connector { "line" } else { "rect" });
            let adjust: Vec<(String, String)> = sp_pr
                .and_then(|s| s.path(ns::A, &["prstGeom", "avLst"]))
                .map(|av| {
                    av.children_named(ns::A, "gd")
                        .filter_map(|g| Some((g.attr("name")?.to_string(), g.attr("fmla")?.to_string())))
                        .collect()
                })
                .unwrap_or_default();
            geometry::preset(name, b.w, b.h, &adjust).or_else(|| geometry::preset("rect", b.w, b.h, &[]))
        };
        let Some(g) = geom else { return };
        let fill = if connector { None } else { look.fill.clone() };
        for p in &g.paths {
            let path_fill = match (p.fill, &fill) {
                (PathFill::None, _) | (_, None) => None,
                (PathFill::Norm, Some(f)) => Some(f.clone()),
                (mode, Some(f)) => Some(shade(f, mode)),
            };
            let stroke = if p.stroke { look.line.as_ref() } else { None };
            if path_fill.is_none() && stroke.is_none() {
                continue;
            }
            out.push(Item::Path {
                x: 0.0,
                y: 0.0,
                d: p.d.clone(),
                fill: path_fill,
                stroke: stroke.map(|s| s.0.clone()),
                sw: stroke.map(|s| r(s.1)).unwrap_or(0.0),
                dash: stroke.map(|s| s.2).unwrap_or("solid"),
            });
        }
    }

    /// The text rectangle inside a shape, in its own coordinates: its geometry's, or the box.
    pub fn text_rect(&self, f: &Frame, b: &Boxed) -> (f64, f64, f64, f64) {
        let o = f.origin.get();
        let sp_pr = o.and_then(|o| o.child(ns::P, "spPr"));
        let g = if let Some(cust) = sp_pr.and_then(|s| s.child(ns::A, "custGeom")) {
            geometry::custom(cust, b.w, b.h)
        } else {
            geometry::preset(f.shape.preset.as_deref().unwrap_or("rect"), b.w, b.h, &[])
        };
        g.map(|g| g.text_rect).unwrap_or((0.0, 0.0, b.w, b.h))
    }

    /// Lays out a frame's text: where its flow sits in the frame, and the flow.
    pub fn text_flow(
        &mut self,
        slide: &Slide,
        f: &Frame,
        body: &office_core::model::TextBody,
        b: &Boxed,
        look: &Look,
    ) -> (f64, f64, Body, FlowLayout) {
        let paint = Paint { doc: self.doc, slide };
        let c = text::cascade(self.doc, self.art, slide, f);
        let settings = c.body();
        let resolved = crate::resolve(self.doc, slide, f).paragraphs;
        let (l, t, rr, bb) = self.text_rect(f, b);
        let vertical = settings.vertical != 0;
        let (bw, bh) = if vertical { (bb - t, rr - l) } else { (rr - l, bb - t) };
        let width = (bw - settings.insets[0] - settings.insets[2]).max(1.0);
        let flow =
            text::layout_body(self.ctx, self.cache, &paint, &c, &settings, &body.paragraphs, &resolved, look.text_color.as_ref(), width);
        let room = bh - settings.insets[1] - settings.insets[3];
        let dy = match settings.anchor {
            1 => (room - flow.height) / 2.0,
            2 => room - flow.height,
            _ => 0.0,
        };
        (l + settings.insets[0], t + settings.insets[1] + dy, settings, flow)
    }

    fn text(&mut self, slide: &Slide, f: &Frame, body: &office_core::model::TextBody, b: &Boxed, look: &Look, out: &mut Vec<Item>) {
        let (x, y, settings, flow) = self.text_flow(slide, f, body, b, look);
        let mut items = Vec::new();
        let links = |_: &Path, _: u32| None;
        if crate::animates_paragraphs(slide, f.id) {
            // Each paragraph in a group of its own, for the slideshow to build one at a time.
            for (i, block) in flow.blocks.iter().enumerate() {
                let office_engine::flow::BlockBox::Para(p) = block else { continue };
                let h = p.layout.lines.last().map_or(0.0, |l| l.top + l.height);
                let mut lines = Vec::new();
                display::paint_para(
                    self.ctx,
                    p,
                    x,
                    -p.y,
                    (f64::MIN, f64::MAX),
                    &mut Sink { objects: &mut NoObjects, links: &links, out: &mut lines },
                );
                items.push(Item::Group {
                    x: 0.0,
                    y: r(y + p.y),
                    w: r(b.w),
                    h: r(h),
                    rot: 0.0,
                    fh: false,
                    fv: false,
                    clip: false,
                    id: Some(format!("p{i}")),
                    items: lines,
                });
            }
        } else {
            display::paint_flow(
                self.ctx,
                &flow,
                x,
                y,
                (f64::MIN, f64::MAX),
                &mut Sink { objects: &mut NoObjects, links: &links, out: &mut items },
            );
        }
        if settings.vertical != 0 {
            let (w, h) = (b.h, b.w);
            out.push(Item::Group {
                x: r((b.w - w) / 2.0),
                y: r((b.h - h) / 2.0),
                w: r(w),
                h: r(h),
                rot: settings.vertical as f64,
                fh: false,
                fv: false,
                clip: false,
                id: None,
                items,
            });
        } else {
            out.extend(items);
        }
    }

    fn table(&mut self, slide: &Slide, t: &Table, b: &Boxed, out: &mut Vec<Item>) {
        let paint = Paint { doc: self.doc, slide };
        let style = table_style::style_for(self.tables, t);
        let look = table_style::look(&paint, t, &style, b.w);
        let mut resolver = CellText { paint: &paint, style: &style, table: t };
        let flow = flow::layout(self.ctx, self.cache, &mut resolver, &[office_core::model::Block::Table(t.clone())], &[], b.w);
        let _ = look;
        let links = |_: &Path, _: u32| None;
        display::paint_flow(self.ctx, &flow, 0.0, 0.0, (f64::MIN, f64::MAX), &mut Sink { objects: &mut NoObjects, links: &links, out });
    }

    /// Objects the model keeps whole (charts, diagrams, embedded objects): a SmartArt diagram's
    /// shapes, else their fallback picture if they have one, else a quiet box where they are.
    fn raw(&mut self, slide: &Slide, part: Option<&str>, xml: &Element, b: &Boxed, out: &mut Vec<Item>) {
        let diagram = xml.path(ns::A, &["graphic", "graphicData"]).filter(|d| office_core::formats::diagram::is_diagram(d));
        if let (Some(data), Some(pkg), Some(part)) = (diagram, self.doc.package.as_ref(), part) {
            let size = xml.child(ns::P, "xfrm").map(Xfrm::parse).map(|x| (x.width, x.height)).unwrap_or_default();
            let frames = office_core::formats::pptx::diagram_frames(pkg, part, data, size);
            if !frames.is_empty() {
                self.diagram(slide, Some(part), &frames, size, b, out);
                return;
            }
        }
        let blip = xml.descendant(ns::A, "blip").and_then(|bl| bl.attr_ns(ns::R, "embed"));
        if let (Some(rid), Some(pkg), Some(part)) = (blip, self.doc.package.as_ref(), part) {
            if let Some(media) = pkg.rels(part).ok().and_then(|rels| rels.get(rid).and_then(|rel| pkg.target(part, rel))) {
                out.push(Item::Image { x: 0.0, y: 0.0, w: r(b.w), h: r(b.h), media, crop: None, id: None });
                return;
            }
        }
        if b.w > 4.0 && b.h > 4.0 {
            out.push(Item::Path {
                x: 0.0,
                y: 0.0,
                d: format!("M0 0H{0}V{1}H0Z", r(b.w), r(b.h)),
                fill: Some("rgba(127,127,127,0.06)".into()),
                stroke: Some("rgba(127,127,127,0.35)".into()),
                sw: 1.0,
                dash: "dash",
            });
        }
    }
}

impl Drawer<'_, '_, '_> {
    /// A diagram's frames, laid out in its `size` (EMUs), drawn inside the box `b`, in the box's
    /// own coordinates.
    fn diagram(
        &mut self,
        slide: &Slide,
        part: Option<&str>,
        frames: &[Frame],
        size: (office_core::units::Emu, office_core::units::Emu),
        b: &Boxed,
        out: &mut Vec<Item>,
    ) {
        let scale = |box_px: f64, emu: office_core::units::Emu| if emu.0 > 0 { box_px / emu_px(emu) } else { 1.0 };
        let map = Map { dx: 0.0, dy: 0.0, sx: scale(b.w, size.0), sy: scale(b.h, size.1) };
        for f in frames {
            self.frame(slide, part, f, map, None, out);
        }
    }
}

/// Draws the SmartArt diagram in `graphic_data`, which sits in `part` of `doc`'s package and is
/// `size` (EMUs) big, into a `w` by `h` pixel box at the origin. For documents other than
/// presentations: a Word drawing is laid out like a slide's shapes.
pub fn paint_diagram(
    doc: &Document,
    ctx: &mut Ctx,
    part: &str,
    graphic_data: &Element,
    size: (office_core::units::Emu, office_core::units::Emu),
    w: f64,
    h: f64,
) -> Vec<Item> {
    let Some(pkg) = doc.package.as_ref() else { return Vec::new() };
    let frames = office_core::formats::pptx::diagram_frames(pkg, part, graphic_data, size);
    let mut out = Vec::new();
    if frames.is_empty() {
        return out;
    }
    let (art, tables, mut cache) = (ArtSet { parts: Default::default() }, TableStyles::default(), flow::Cache::new());
    let slide = Slide::default();
    let mut d = Drawer { doc, art: &art, tables: &tables, ctx, cache: &mut cache };
    d.diagram(&slide, Some(part), &frames, size, &Boxed { w, h, ..Default::default() }, &mut out);
    out
}

/// Moves a drawn item (and, for a group, nothing inside it: children are relative).
fn shift(item: &mut Item, dx: f64, dy: f64) {
    match item {
        Item::Text { x, y, .. }
        | Item::Image { x, y, .. }
        | Item::Path { x, y, .. }
        | Item::Group { x, y, .. }
        | Item::Rect { x, y, .. } => {
            *x = r(*x + dx);
            *y = r(*y + dy);
        }
        Item::Line { x1, y1, x2, y2, .. } => {
            *x1 = r(*x1 + dx);
            *x2 = r(*x2 + dx);
            *y1 = r(*y1 + dy);
            *y2 = r(*y2 + dy);
        }
    }
}

/// A fill as CSS: solid colors exactly, gradients as their first stop's color.
pub fn fill_css(paint: &Paint, fill: &Fill) -> Option<String> {
    match fill {
        Fill::None => None,
        Fill::Solid { color } => Some(paint.color(color).css()),
        Fill::Other { xml } => {
            if xml.is(ns::A, "gradFill") {
                let stops: Vec<&Element> =
                    xml.descendant(ns::A, "gsLst").map(|g| g.children_named(ns::A, "gs").collect()).unwrap_or_default();
                let mid = stops.get(stops.len() / 2).copied();
                return mid.and_then(Color::in_drawingml).map(|c| paint.color(&c).css());
            }
            if xml.is(ns::A, "pattFill") {
                return xml.child(ns::A, "fgClr").and_then(Color::in_drawingml).map(|c| paint.color(&c).css());
            }
            None
        }
    }
}

fn line_css(paint: &Paint, l: &Line, style: Option<Color>) -> Option<(String, f64, &'static str)> {
    let color = match &l.fill {
        Some(Fill::None) => return None,
        Some(f) => fill_css(paint, f)?,
        None => paint.color(&style?).css(),
    };
    let w = l.width.map(emu_px).unwrap_or(1.0).max(0.5);
    let dash = match l.dash.as_deref() {
        Some("dash" | "lgDash" | "sysDash" | "dashDot" | "lgDashDot" | "lgDashDotDot" | "sysDashDot" | "sysDashDotDot") => "dash",
        Some("dot" | "sysDot") => "dot",
        _ => "solid",
    };
    Some((color, w, dash))
}

/// A background's color: `p:bgPr`'s fill, or `p:bgRef`'s color.
fn background(paint: &Paint, bg: Option<&Element>) -> Option<String> {
    let bg = bg?;
    if let Some(pr) = bg.child(ns::P, "bgPr") {
        return Fill::in_children(pr).and_then(|f| fill_css(paint, &f));
    }
    bg.child(ns::P, "bgRef").and_then(Color::in_drawingml).map(|c| paint.color(&c).css())
}

/// A path's fill lightened or darkened, as DrawingML's path fill modes ask.
fn shade(css: &str, mode: PathFill) -> String {
    let Some(rgb) = css.strip_prefix('#').and_then(office_core::drawing::Rgb::parse) else { return css.to_string() };
    let k = match mode {
        PathFill::Lighten => 0.4,
        PathFill::LightenLess => 0.2,
        PathFill::Darken => -0.4,
        PathFill::DarkenLess => -0.2,
        _ => 0.0,
    };
    let ch = |v: u8| -> u8 {
        let v = v as f64;
        (if k > 0.0 { v + (255.0 - v) * k } else { v * (1.0 + k) }).round().clamp(0.0, 255.0) as u8
    };
    office_core::drawing::Rgba { rgb: office_core::drawing::Rgb(ch(rgb.0), ch(rgb.1), ch(rgb.2)), alpha: 1.0 }.css()
}

/// Text in table cells: the table style's text formatting for the cell's part, under the
/// paragraphs' own.
struct CellText<'a> {
    paint: &'a Paint<'a>,
    style: &'a table_style::TableStyle,
    table: &'a Table,
}

impl Resolver for CellText<'_> {
    fn para(&mut self, ctx: &mut Ctx, path: &Path, p: &office_core::model::Paragraph, width: f64) -> (std::sync::Arc<ParaSpec>, Keep) {
        let _ = width;
        let (row, col) = path.cell().map(|(_, r, c)| (r as usize, c as usize)).unwrap_or((0, 0));
        let part = table_style::text_for(self.table, self.style, row, col);
        let mut props = office_core::model::RunProps { size: Some(office_core::units::FontSize::from_points(18.0)), ..Default::default() };
        if let Some(b) = part.bold {
            props.bold = Some(b);
        }
        let resolved: Vec<office_core::model::RunProps> = p
            .runs()
            .map(|r| {
                let mut x = props.clone();
                x.overlay(&r.props);
                x
            })
            .collect();
        let spec = std::sync::Arc::new(text::cell_spec(ctx, self.paint, p, &resolved, part.color.as_ref()));
        (spec, Keep::default())
    }

    fn table(&mut self, _: &Path, t: &Table, width: f64) -> TableLook {
        table_style::look(self.paint, t, self.style, width)
    }
}
