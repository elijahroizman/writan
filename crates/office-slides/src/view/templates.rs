//! Starter slides, sized in the same reference coordinates as the original editor templates.
//! Frames use theme colors and theme fonts so a template follows its destination deck.

use office_core::drawing::{Color, Fill, Line, ShapeProps, Xfrm};
use office_core::model::{Align, Frame, FrameContent, LineSpacing, Origin, Paragraph, RunProps, Slide};
use office_core::units::{Emu, FontSize};
use office_core::xml::{ns, Element};

use super::commands::set_bullet;
use super::{Sel, SlidesView};

const MARGIN: f64 = 457_200.0;
const CONTENT: f64 = 12_192_000.0 - MARGIN * 2.0;
const HALF: f64 = (CONTENT - 320_000.0) / 2.0;
const RIGHT: f64 = MARGIN + HALF + 320_000.0;

type Rect = (f64, f64, f64, f64);

struct Builder {
    sx: f64,
    sy: f64,
    frames: Vec<Frame>,
}

impl Builder {
    #[allow(clippy::too_many_arguments)]
    fn text(&mut self, rect: Rect, name: &str, value: &str, pt: f64, color: &str, bold: bool, align: Align, anchor: &str) {
        let paragraphs = value
            .split('\n')
            .map(|value| {
                let mut p = Paragraph::text(value);
                p.props.align = Some(align);
                let props = RunProps {
                    font: Some("+mn-lt".into()),
                    size: Some(FontSize::from_points(pt)),
                    color: Some(Color::scheme(color)),
                    bold: Some(bold),
                    ..Default::default()
                };
                for r in p.runs_mut() {
                    r.props = props.clone();
                }
                p.mark = Some(props);
                p
            })
            .collect();
        let xf = Xfrm {
            x: Emu((rect.0 * self.sx).round() as i64),
            y: Emu((rect.1 * self.sy).round() as i64),
            width: Emu((rect.2 * self.sx).round() as i64),
            height: Emu((rect.3 * self.sy).round() as i64),
            ..Default::default()
        };
        let mut f = Frame::text_box(self.frames.len() as u32 + 2, xf, paragraphs);
        f.name = name.into();
        // Text uses the old template's vertical anchoring and zero inset in its box.
        if let FrameContent::Shape { text: Some(body), .. } = &mut f.content {
            let mut pr = Element::new(ns::A, "bodyPr").with_attr("", "anchor", anchor);
            for key in ["lIns", "tIns", "rIns", "bIns"] {
                pr.set_attr("", key, "0");
            }
            body.origin = Origin::new(Element::new(ns::P, "txBody").with_child(pr));
        }
        f.origin = Origin::new(
            Element::new(ns::P, "sp")
                .with_child(Element::new(ns::P, "nvSpPr").with_child(Element::new(ns::P, "cNvSpPr").with_attr("", "txBox", "1"))),
        );
        self.frames.push(f);
    }

    fn shape(&mut self, rect: Rect, name: &str, fill: &str, rounded: bool) {
        self.text(rect, name, "", 16.0, "tx2", false, Align::Center, "ctr");
        let f = self.frames.last_mut().unwrap();
        f.shape = ShapeProps {
            xfrm: f.shape.xfrm.take(),
            preset: Some(if rounded { "roundRect" } else { "rect" }.into()),
            fill: Some(Fill::Solid { color: Color::scheme(fill) }),
            line: Some(Line { fill: Some(Fill::None), width: None, dash: None }),
        };
        f.origin = Origin::default();
        f.content = FrameContent::Shape { text: None, connector: false };
    }

    #[allow(clippy::too_many_arguments)]
    fn panel(&mut self, rect: Rect, name: &str, value: &str, fill: &str, pt: f64, color: &str, bold: bool) {
        self.text(rect, name, value, pt, color, bold, Align::Center, "ctr");
        let f = self.frames.last_mut().unwrap();
        f.shape.preset = Some("roundRect".into());
        f.shape.fill = Some(Fill::Solid { color: Color::scheme(fill) });
        f.shape.line = Some(Line { fill: Some(Fill::None), width: None, dash: None });
        f.origin = Origin::default();
    }

    /// Sets the last text in the theme's heading font.
    fn heading(&mut self) {
        if let Some(FrameContent::Shape { text: Some(body), .. }) = self.frames.last_mut().map(|f| &mut f.content) {
            for p in &mut body.paragraphs {
                for r in p.runs_mut() {
                    r.props.font = Some("+mj-lt".into());
                }
                if let Some(mark) = &mut p.mark {
                    mark.font = Some("+mj-lt".into());
                }
            }
        }
    }

    fn title(&mut self) {
        self.text((MARGIN, 420_000.0, CONTENT, 900_000.0), "Title", "Slide Title", 32.0, "tx1", true, Align::Left, "b");
        self.heading();
        self.shape((MARGIN, 1_420_000.0, 1_600_000.0, 60_000.0), "Title Rule", "accent1", false);
    }
}

/// Relative luminance (WCAG), 0 for black to 1 for white.
fn luminance(c: office_core::drawing::Rgb) -> f64 {
    let lin = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.0) + 0.7152 * lin(c.1) + 0.0722 * lin(c.2)
}

impl SlidesView {
    /// The theme's light or dark color, whichever reads better on `fill` (white on a dark
    /// accent, near-black on a yellow one), as the slide at `at` shows them.
    fn ink_on(&self, at: usize, fill: &str) -> &'static str {
        let slides = self.slides();
        let Some(slide) = slides.get(at.min(slides.len().saturating_sub(1))) else { return "lt1" };
        let lum = |name: &str| luminance(crate::color(&self.doc, slide, &Color::scheme(name)).rgb);
        let f = lum(fill);
        let contrast = |l: f64| (f.max(l) + 0.05) / (f.min(l) + 0.05);
        if contrast(lum("lt1")) >= contrast(lum("dk1")) {
            "lt1"
        } else {
            "dk1"
        }
    }

    /// Adds a starter slide on the blank layout (or the first layout), in one undo step.
    pub fn add_template(&mut self, after: usize, template: &str) {
        let (w, h) = self.size();
        let mut b = Builder { sx: w * 9525.0 / 12_192_000.0, sy: h * 9525.0 / 6_858_000.0, frames: Vec::new() };
        let on_accent1 = self.ink_on(after, "accent1");
        match template {
            "title" => {
                b.text((MARGIN, 2_300_000.0, CONTENT, 1_300_000.0), "Title", "Presentation Title", 44.0, "tx1", true, Align::Left, "b");
                b.heading();
                b.shape((MARGIN, 3_760_000.0, 2_200_000.0, 80_000.0), "Accent Bar", "accent1", false);
                b.text(
                    (MARGIN, 3_980_000.0, CONTENT, 640_000.0),
                    "Subtitle",
                    "Subtitle or speaker name",
                    20.0,
                    "tx2",
                    false,
                    Align::Left,
                    "t",
                );
            }
            "titleAndContent" => {
                b.title();
                b.text(
                    (MARGIN, 1_720_000.0, CONTENT, 4_400_000.0),
                    "Content",
                    "First key point\nSecond key point\nThird key point",
                    18.0,
                    "tx1",
                    false,
                    Align::Left,
                    "t",
                );
                if let FrameContent::Shape { text: Some(body), .. } = &mut b.frames.last_mut().unwrap().content {
                    for p in &mut body.paragraphs {
                        set_bullet(p, Some(false));
                        p.props.line_spacing = Some(LineSpacing::Multiple(office_core::units::Percent(160_000)));
                    }
                }
            }
            "sectionHeader" => {
                b.shape((0.0, 0.0, 4_000_000.0, 6_858_000.0), "Section Panel", "accent1", false);
                b.text(
                    (500_000.0, 2_650_000.0, 3_000_000.0, 1_500_000.0),
                    "Section Number",
                    "01",
                    60.0,
                    on_accent1,
                    true,
                    Align::Left,
                    "ctr",
                );
                b.text(
                    (4_500_000.0, 2_750_000.0, 12_192_000.0 - 4_500_000.0 - MARGIN, 1_100_000.0),
                    "Section Title",
                    "Section Title",
                    36.0,
                    "tx1",
                    true,
                    Align::Left,
                    "b",
                );
                b.heading();
                b.text(
                    (4_500_000.0, 3_980_000.0, 12_192_000.0 - 4_500_000.0 - MARGIN, 700_000.0),
                    "Section Caption",
                    "A short introduction to this section",
                    16.0,
                    "tx2",
                    false,
                    Align::Left,
                    "t",
                );
            }
            "twoContent" => {
                b.title();
                b.panel((MARGIN, 1_720_000.0, HALF, 4_400_000.0), "Left Content", "First content area", "bg2", 16.0, "tx2", false);
                b.panel((RIGHT, 1_720_000.0, HALF, 4_400_000.0), "Right Content", "Second content area", "bg2", 16.0, "tx2", false);
            }
            "comparison" => {
                b.title();
                for (x, side, option, accent, text) in [
                    (MARGIN, "Left", "Option A", "accent1", "First content area"),
                    (RIGHT, "Right", "Option B", "accent2", "Second content area"),
                ] {
                    b.panel(
                        (x, 1_720_000.0, HALF, 700_000.0),
                        &format!("{side} Header"),
                        option,
                        accent,
                        18.0,
                        self.ink_on(after, accent),
                        true,
                    );
                    b.panel((x, 2_540_000.0, HALF, 3_580_000.0), &format!("{side} Body"), text, "bg2", 16.0, "tx2", false);
                }
            }
            "titleOnly" => b.title(),
            "quote" => {
                b.text((MARGIN, 1_100_000.0, 1_600_000.0, 1_500_000.0), "Quote Mark", "“", 96.0, "accent1", true, Align::Left, "t");
                b.text(
                    (1_300_000.0, 2_300_000.0, 9_592_000.0, 1_800_000.0),
                    "Quote",
                    "A memorable quote that captures the message of this presentation.",
                    28.0,
                    "tx1",
                    false,
                    Align::Center,
                    "ctr",
                );
                b.heading();
                if let FrameContent::Shape { text: Some(body), .. } = &mut b.frames.last_mut().unwrap().content {
                    for p in &mut body.paragraphs {
                        p.props.line_spacing = Some(LineSpacing::Multiple(office_core::units::Percent(130_000)));
                        for r in p.runs_mut() {
                            r.props.italic = Some(true);
                        }
                    }
                }
                b.text(
                    (1_300_000.0, 4_350_000.0, 9_592_000.0, 600_000.0),
                    "Attribution",
                    "Speaker Name, Role",
                    16.0,
                    "tx2",
                    false,
                    Align::Center,
                    "t",
                );
            }
            "closing" => {
                b.text((MARGIN, 2_450_000.0, CONTENT, 1_300_000.0), "Closing Title", "Thank You", 44.0, "tx1", true, Align::Center, "b");
                b.heading();
                b.shape((12_192_000.0 / 2.0 - 1_100_000.0, 3_900_000.0, 2_200_000.0, 80_000.0), "Accent Bar", "accent1", false);
                b.text(
                    (MARGIN, 4_150_000.0, CONTENT, 640_000.0),
                    "Contact",
                    "Questions? Reach out any time.",
                    16.0,
                    "tx2",
                    false,
                    Align::Center,
                    "t",
                );
            }
            "blank" => {}
            _ => return,
        }
        let layout = crate::layout(&self.doc, "blank").or_else(|| crate::layouts(&self.doc).first().copied()).map(|l| l.part.clone());
        let part = Some(crate::free_part_in(&self.doc, &[], self.new_part_seed().as_deref()));
        let to = after.saturating_add(1).min(self.slides().len());
        let origin = Element::new(ns::P, "sld").with_child(Element::new(ns::P, "cSld").with_child(Element::new(ns::P, "bg").with_child(
            Element::new(ns::P, "bgPr").with_child(Element::new(ns::A, "solidFill").with_child(Color::scheme("bg1").to_drawingml())),
        )));
        self.change_deck(|slides| {
            slides.insert(to, Slide { frames: b.frames, part, layout, origin: Origin::new(origin), ..Default::default() })
        });
        self.sel = Sel { active: to, frames: Vec::new(), editing: None };
    }
}
