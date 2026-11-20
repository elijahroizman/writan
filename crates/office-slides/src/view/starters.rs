//! Starter decks for New presentation. Each sets the deck's theme (its colors and its heading
//! and body fonts) and starts with six example slides in that style, holding text to replace.
//! Shapes and text use the theme's colors and fonts wherever the design allows, so the New
//! slide layouts, and theme pickers in other apps, follow the deck.
//!
//! Positions are in points on a 960 × 540 slide (13.33 × 7.5 in). Color roles are the color
//! map's: `bg1` the background, `tx1` text, `bg2` panels and `tx2` muted text, for light and
//! dark decks alike.

use office_core::drawing::{Color, Fill, FontScheme, FontSet, Line, Rgb, ShapeProps, Theme, Xfrm};
use office_core::model::{Align, Body, Document, Frame, FrameContent, Kind, LineSpacing, Origin, Paragraph, RunProps, Slide, SlideSize};
use office_core::opc::Package;
use office_core::units::{Emu, FontSize, Percent};
use office_core::xml::{ns, Element};
use office_core::Compiler;

use crate::Decks;

/// The starter decks New presentation offers, in order.
pub const STARTERS: [&str; 6] = ["plain", "harbor", "graphite", "signal", "lagoon", "blank"];

const PT: f64 = 12_700.0;

type Rect = (f64, f64, f64, f64);

/// A deck's theme. Colors are given by role and placed in the theme's slots by [`Look::theme`].
struct Look {
    name: &'static str,
    /// Light text on a dark background: the color map shows `dk1` as the background.
    dark: bool,
    heading: &'static str,
    body: &'static str,
    background: &'static str,
    text: &'static str,
    panel: &'static str,
    muted: &'static str,
    accents: [&'static str; 6],
    hlink: &'static str,
    followed: &'static str,
}

impl Look {
    fn theme(&self) -> Theme {
        // A light deck keeps Office's convention: dark 1 is the text and light 1 the background.
        let (dk1, lt1, dk2, lt2) = if self.dark {
            (self.background, self.text, self.panel, self.muted)
        } else {
            (self.text, self.background, self.muted, self.panel)
        };
        let mut colors = vec![("dk1", dk1), ("lt1", lt1), ("dk2", dk2), ("lt2", lt2)];
        for (i, a) in self.accents.iter().enumerate() {
            colors.push((["accent1", "accent2", "accent3", "accent4", "accent5", "accent6"][i], a));
        }
        colors.push(("hlink", self.hlink));
        colors.push(("folHlink", self.followed));
        let set = |latin: &str| FontSet { latin: latin.into(), ..Default::default() };
        Theme {
            name: self.name.into(),
            color_scheme_name: self.name.into(),
            colors: colors.into_iter().map(|(slot, hex)| (slot.to_string(), rgb(hex))).collect(),
            fonts: FontScheme { name: self.name.into(), major: set(self.heading), minor: set(self.body) },
            format_scheme: None,
        }
    }
}

fn rgb(hex: &str) -> Color {
    Color::rgb(Rgb::parse(hex).expect("a starter color is #RRGGBB"))
}

/// `#RRGGBB`, or a theme color (`accent1`, `tx1`…).
fn color(c: &str) -> Color {
    if c.starts_with('#') {
        rgb(c)
    } else {
        Color::scheme(c)
    }
}

#[derive(Clone, Copy)]
enum Face {
    Heading,
    Body,
    Code,
}

/// How a text box's text looks.
#[derive(Clone, Copy)]
struct St {
    pt: f64,
    color: &'static str,
    face: Face,
    bold: bool,
    italic: bool,
    align: Align,
    anchor: &'static str,
    /// Line spacing as a multiple of single; 0 leaves it single.
    spacing: f64,
    /// Space after each paragraph, in points.
    after: f64,
}

fn st(pt: f64) -> St {
    St { pt, color: "tx1", face: Face::Body, bold: false, italic: false, align: Align::Left, anchor: "t", spacing: 0.0, after: 0.0 }
}

impl St {
    fn head(self) -> St {
        St { face: Face::Heading, ..self }
    }
    fn code(self) -> St {
        St { face: Face::Code, ..self }
    }
    fn bold(self) -> St {
        St { bold: true, ..self }
    }
    fn color(self, color: &'static str) -> St {
        St { color, ..self }
    }
    fn center(self) -> St {
        St { align: Align::Center, ..self }
    }
    fn right(self) -> St {
        St { align: Align::Right, ..self }
    }
    fn middle(self) -> St {
        St { anchor: "ctr", ..self }
    }
    fn bottom(self) -> St {
        St { anchor: "b", ..self }
    }
    fn spacing(self, spacing: f64) -> St {
        St { spacing, ..self }
    }
    fn after(self, after: f64) -> St {
        St { after, ..self }
    }
}

fn xfrm((x, y, w, h): Rect) -> Xfrm {
    let emu = |v: f64| Emu((v * PT).round() as i64);
    Xfrm { x: emu(x), y: emu(y), width: emu(w), height: emu(h), ..Default::default() }
}

/// `p:sp` with its non-visual properties in schema order, for a frame's origin.
fn sp(c_nv_sp_pr: Element) -> Element {
    Element::new(ns::P, "sp").with_child(
        Element::new(ns::P, "nvSpPr")
            .with_child(Element::new(ns::P, "cNvPr"))
            .with_child(c_nv_sp_pr)
            .with_child(Element::new(ns::P, "nvPr")),
    )
}

/// One slide being built.
struct Page {
    frames: Vec<Frame>,
    background: Option<&'static str>,
}

impl Page {
    fn new() -> Page {
        Page { frames: Vec::new(), background: None }
    }

    fn on(background: &'static str) -> Page {
        Page { frames: Vec::new(), background: Some(background) }
    }

    fn next_id(&self) -> u32 {
        self.frames.len() as u32 + 2
    }

    fn text(&mut self, rect: Rect, lines: &[&str], s: St) -> &mut Frame {
        let font = match s.face {
            Face::Heading => "+mj-lt",
            Face::Body => "+mn-lt",
            Face::Code => "Consolas",
        };
        let props = RunProps {
            font: Some(font.into()),
            size: Some(FontSize::from_points(s.pt)),
            color: Some(color(s.color)),
            bold: Some(s.bold),
            italic: s.italic.then_some(true),
            ..Default::default()
        };
        let paragraphs = lines
            .iter()
            .map(|line| {
                let mut p = Paragraph::text(*line);
                p.props.align = Some(s.align);
                if s.spacing > 0.0 {
                    p.props.line_spacing = Some(LineSpacing::Multiple(Percent((s.spacing * 100_000.0).round() as i32)));
                }
                if s.after > 0.0 {
                    p.props.space_after = Some(Emu((s.after * PT).round() as i64));
                }
                for r in p.runs_mut() {
                    r.props = props.clone();
                }
                p.mark = Some(props.clone());
                p
            })
            .collect();
        let mut f = Frame::text_box(self.next_id(), xfrm(rect), paragraphs);
        // The box is exactly where the text sits: no inset, anchored as the design has it.
        if let FrameContent::Shape { text: Some(body), .. } = &mut f.content {
            let mut pr = Element::new(ns::A, "bodyPr").with_attr("", "wrap", "square").with_attr("", "anchor", s.anchor);
            for key in ["lIns", "tIns", "rIns", "bIns"] {
                pr.set_attr("", key, "0");
            }
            body.origin = Origin::new(Element::new(ns::P, "txBody").with_child(pr));
        }
        f.origin = Origin::new(sp(Element::new(ns::P, "cNvSpPr").with_attr("", "txBox", "1")).with_child(Element::new(ns::P, "spPr")));
        self.frames.push(f);
        self.frames.last_mut().unwrap()
    }

    /// Recolors one paragraph of the last text box.
    fn recolor(&mut self, paragraph: usize, c: &'static str) {
        if let Some(FrameContent::Shape { text: Some(body), .. }) = self.frames.last_mut().map(|f| &mut f.content) {
            let p = &mut body.paragraphs[paragraph];
            for r in p.runs_mut() {
                r.props.color = Some(color(c));
            }
            if let Some(mark) = &mut p.mark {
                mark.color = Some(color(c));
            }
        }
    }

    fn shape(&mut self, rect: Rect, fill: &str, preset: &str, name: &str, origin: Origin) {
        let id = self.next_id();
        let mut f = Frame::text_box(id, xfrm(rect), Vec::new());
        f.name = format!("{name} {id}");
        f.shape = ShapeProps {
            xfrm: Some(xfrm(rect)),
            preset: Some(preset.into()),
            fill: Some(Fill::Solid { color: color(fill) }),
            line: Some(Line { fill: Some(Fill::None), width: None, dash: None }),
        };
        f.content = FrameContent::Shape { text: None, connector: false };
        f.origin = origin;
        self.frames.push(f);
    }

    fn rect(&mut self, rect: Rect, fill: &str) {
        self.shape(rect, fill, "rect", "Rectangle", Origin::default());
    }

    fn oval(&mut self, rect: Rect, fill: &str) {
        self.shape(rect, fill, "ellipse", "Oval", Origin::default());
    }

    /// A rectangle with corners of `radius` points.
    fn round(&mut self, rect: Rect, fill: &str, radius: f64) {
        let adj = (radius / rect.2.min(rect.3) * 100_000.0).round() as i64;
        let geom = Element::new(ns::A, "prstGeom").with_attr("", "prst", "roundRect").with_child(
            Element::new(ns::A, "avLst").with_child(Element::new(ns::A, "gd").with_attr("", "name", "adj").with_attr(
                "",
                "fmla",
                format!("val {adj}"),
            )),
        );
        let origin = Origin::new(sp(Element::new(ns::P, "cNvSpPr")).with_child(Element::new(ns::P, "spPr").with_child(geom)));
        self.shape(rect, fill, "roundRect", "Rounded Rectangle", origin);
    }

    fn slide(self) -> Slide {
        let origin = self.background.map(|bg| {
            Origin::new(Element::new(ns::P, "sld").with_child(Element::new(ns::P, "cSld").with_child(
                Element::new(ns::P, "bg").with_child(Element::new(ns::P, "bgPr").with_child(Fill::Solid { color: color(bg) }.to_xml())),
            )))
        });
        Slide { frames: self.frames, origin: origin.unwrap_or_default(), ..Default::default() }
    }
}

const BULLETS: [&str; 4] = ["•  First point", "•  Second point", "•  Third point", "•  Fourth point"];

fn plain() -> (Look, Vec<Slide>) {
    let look = Look {
        name: "Plain",
        dark: false,
        heading: "Arial",
        body: "Arial",
        background: "#FFFFFF",
        text: "#16181D",
        panel: "#F2F3F5",
        muted: "#5B6170",
        accents: ["#2B59C3", "#E0663A", "#2F9E77", "#D9A21B", "#7A5AC8", "#8C93A3"],
        hlink: "#2B59C3",
        followed: "#7A5AC8",
    };
    const RULE: &str = "#E2E4E9";
    let head = |p: &mut Page, title: &str| {
        p.text((56.0, 44.0, 848.0, 56.0), &[title], st(32.0).head().bold().bottom());
        p.rect((56.0, 110.0, 48.0, 3.0), "accent1");
    };

    let mut title = Page::new();
    title.text((56.0, 150.0, 848.0, 150.0), &["Presentation title"], st(54.0).head().bold().bottom());
    title.rect((56.0, 316.0, 64.0, 4.0), "accent1");
    title.text((56.0, 336.0, 848.0, 30.0), &["Subtitle, presenter or date"], st(20.0).color("tx2"));

    let mut agenda = Page::new();
    head(&mut agenda, "Agenda");
    for (i, topic) in ["First topic", "Second topic", "Third topic", "Fourth topic"].iter().enumerate() {
        let y = 140.0 + i as f64 * 84.0;
        agenda.rect((56.0, y, 848.0, 1.0), RULE);
        agenda.text((56.0, y + 20.0, 64.0, 44.0), &[&format!("0{}", i + 1)], st(20.0).bold().color("accent1").middle());
        agenda.text((128.0, y + 20.0, 776.0, 44.0), &[topic], st(26.0).middle());
    }
    agenda.rect((56.0, 476.0, 848.0, 1.0), RULE);

    let mut section = Page::new();
    section.text((56.0, 120.0, 848.0, 140.0), &["02"], st(96.0).head().bold().color("accent1").bottom());
    section.text((56.0, 272.0, 848.0, 64.0), &["Section title"], st(44.0).head().bold());
    section.text((56.0, 344.0, 848.0, 30.0), &["One line about this section"], st(20.0).color("tx2"));

    let mut content = Page::new();
    head(&mut content, "Slide title");
    content.text((56.0, 140.0, 520.0, 330.0), &BULLETS, st(24.0).spacing(1.6));
    content.rect((616.0, 140.0, 288.0, 240.0), "bg2");
    content.text((644.0, 166.0, 232.0, 22.0), &["HIGHLIGHT"], st(14.0).bold().color("accent1"));
    content.text((644.0, 196.0, 232.0, 80.0), &["75%"], st(64.0).head().bold());
    content.text((644.0, 284.0, 232.0, 70.0), &["What this number means"], st(18.0).color("tx2"));

    let mut numbers = Page::new();
    head(&mut numbers, "Key numbers");
    for (i, (n, label)) in [("120", "First measure"), ("98%", "Second measure"), ("3.5×", "Third measure")].iter().enumerate() {
        let x = 56.0 + i as f64 * 288.0;
        numbers.rect((x, 190.0, 272.0, 2.0), "tx1");
        numbers.text((x, 212.0, 272.0, 80.0), &[n], st(56.0).head().bold().color("accent1"));
        numbers.text((x, 300.0, 260.0, 60.0), &[label], st(20.0).color("tx2"));
    }

    let mut closing = Page::new();
    closing.text((56.0, 170.0, 848.0, 110.0), &["Questions?"], st(54.0).head().bold().bottom());
    closing.rect((56.0, 296.0, 64.0, 4.0), "accent1");
    closing.text((56.0, 316.0, 848.0, 30.0), &["name@example.com"], st(20.0).color("tx2"));

    (look, [title, agenda, section, content, numbers, closing].into_iter().map(Page::slide).collect())
}

fn harbor() -> (Look, Vec<Slide>) {
    let look = Look {
        name: "Harbor",
        dark: false,
        heading: "Calibri Light",
        body: "Calibri",
        background: "#FFFFFF",
        text: "#0F2A44",
        panel: "#EEF2F7",
        muted: "#5A6B80",
        accents: ["#2F80ED", "#F2A93B", "#1D446B", "#9CC3F5", "#3BA889", "#E5674E"],
        hlink: "#2F80ED",
        followed: "#1D446B",
    };
    const SOFT: &str = "#C9D6E6";
    let band = |p: &mut Page, title: &str| {
        p.rect((0.0, 0.0, 960.0, 96.0), "tx1");
        p.text((56.0, 20.0, 848.0, 56.0), &[title], st(30.0).head().color("bg1").middle());
    };
    let blocks = |p: &mut Page, x: f64, y: f64| {
        p.rect((x, y, 120.0, 120.0), "accent1");
        p.rect((x + 120.0, y, 120.0, 120.0), "accent4");
        p.rect((x + 120.0, y - 120.0, 120.0, 120.0), "accent2");
        p.rect((x, y + 120.0, 240.0, 120.0), "accent3");
    };

    let mut title = Page::new();
    title.rect((0.0, 0.0, 600.0, 540.0), "tx1");
    title.text((56.0, 120.0, 490.0, 190.0), &["Presentation title"], st(44.0).head().color("bg1").bottom());
    title.rect((56.0, 330.0, 56.0, 4.0), "accent2");
    title.text((56.0, 350.0, 490.0, 60.0), &["Prepared for client name", "Month year"], st(18.0).color(SOFT));
    title.rect((600.0, 0.0, 360.0, 540.0), "bg2");
    blocks(&mut title, 660.0, 300.0);

    let mut agenda = Page::new();
    band(&mut agenda, "Agenda");
    let topics = [
        ("Background", "Where things stand today"),
        ("Proposal", "What we recommend"),
        ("Timeline", "How the work is phased"),
        ("Next steps", "What we need from you"),
    ];
    for (i, (h, d)) in topics.iter().enumerate() {
        let (x, y) = (if i % 2 == 1 { 488.0 } else { 56.0 }, if i < 2 { 136.0 } else { 316.0 });
        agenda.rect((x, y, 416.0, 164.0), "bg2");
        agenda.text((x + 24.0, y + 20.0, 60.0, 34.0), &[&format!("0{}", i + 1)], st(24.0).bold().color("accent1"));
        agenda.text((x + 24.0, y + 58.0, 368.0, 32.0), &[h], st(24.0).bold());
        agenda.text((x + 24.0, y + 98.0, 368.0, 48.0), &[d], st(17.0).color("tx2"));
    }

    let mut section = Page::on("tx1");
    section.text((56.0, 196.0, 848.0, 30.0), &["02"], st(20.0).bold().color("accent2"));
    section.text((56.0, 228.0, 848.0, 70.0), &["Section title"], st(48.0).head().color("bg1"));
    section.text((56.0, 306.0, 848.0, 30.0), &["One line about this section"], st(20.0).color(SOFT));
    section.rect((820.0, 420.0, 70.0, 70.0), "accent1");
    section.rect((890.0, 420.0, 70.0, 70.0), "accent4");
    section.rect((890.0, 350.0, 70.0, 70.0), "accent2");
    section.rect((820.0, 490.0, 140.0, 50.0), "accent3");

    let mut content = Page::new();
    band(&mut content, "Slide title");
    content.text((56.0, 136.0, 500.0, 330.0), &BULLETS, st(22.0).spacing(1.45));
    content.rect((600.0, 136.0, 304.0, 330.0), "accent1");
    content.text((628.0, 164.0, 248.0, 24.0), &["Key result"], st(18.0).color("bg1"));
    content.text((628.0, 196.0, 248.0, 96.0), &["+40%"], st(72.0).bold().color("bg1"));
    content.text((628.0, 300.0, 248.0, 60.0), &["Where and when this was measured"], st(16.0).color("#DCE9FB"));

    let mut timeline = Page::new();
    band(&mut timeline, "Timeline");
    timeline.rect((56.0, 290.0, 848.0, 3.0), "tx1");
    for i in 0..4 {
        let x = 56.0 + i as f64 * 212.0;
        timeline.oval((x, 282.5, 18.0, 18.0), if i == 0 { "accent2" } else { "accent1" });
        timeline.text((x, 226.0, 200.0, 40.0), &[&format!("Phase {}", i + 1)], st(20.0).bold().bottom());
        timeline.text((x, 318.0, 196.0, 80.0), &["What happens in this phase"], st(18.0).color("tx2").spacing(1.2));
    }

    let mut closing = Page::on("tx1");
    closing.text((56.0, 160.0, 600.0, 110.0), &["Thank you"], st(54.0).head().color("bg1").bottom());
    closing.rect((56.0, 286.0, 56.0, 4.0), "accent2");
    closing.text((56.0, 306.0, 560.0, 60.0), &["Your name, role", "name@example.com"], st(18.0).color(SOFT));
    blocks(&mut closing, 700.0, 300.0);

    (look, [title, agenda, section, content, timeline, closing].into_iter().map(Page::slide).collect())
}

fn graphite() -> (Look, Vec<Slide>) {
    let look = Look {
        name: "Graphite",
        dark: true,
        heading: "Arial",
        body: "Arial",
        background: "#17191E",
        text: "#ECEDEF",
        panel: "#22252C",
        muted: "#9AA0AB",
        accents: ["#7CC4FA", "#F5B971", "#B4A6F7", "#7FD6A4", "#F08A8A", "#5E6673"],
        hlink: "#7CC4FA",
        followed: "#B4A6F7",
    };
    const LINE: &str = "#2E323A";

    let mut title = Page::new();
    for (i, accent) in ["accent1", "accent2", "accent3"].iter().enumerate() {
        title.rect((848.0 + i as f64 * 18.0, 56.0, 12.0, 12.0), accent);
    }
    title.text((56.0, 140.0, 848.0, 24.0), &["EVENT NAME · TALK"], st(14.0).bold().color("accent1"));
    title.text((56.0, 172.0, 760.0, 130.0), &["Title of your talk"], st(48.0).head().bold().bottom());
    title.text((56.0, 316.0, 760.0, 30.0), &["One line about the talk"], st(22.0).color("tx2"));
    title.rect((56.0, 440.0, 848.0, 1.0), LINE);
    title.text((56.0, 456.0, 480.0, 24.0), &["Your name · Role"], st(16.0).color("tx2"));
    title.text((504.0, 456.0, 400.0, 24.0), &["@handle"], st(16.0).code().color("tx2").right());

    let mut agenda = Page::new();
    agenda.text((56.0, 56.0, 848.0, 48.0), &["Agenda"], st(32.0).head().bold());
    for (i, topic) in ["First topic", "Second topic", "Third topic"].iter().enumerate() {
        let y = 150.0 + i as f64 * 104.0;
        agenda.rect((56.0, y - 20.0, 848.0, 1.0), LINE);
        agenda.text((56.0, y, 72.0, 40.0), &[&format!("0{}", i + 1)], st(22.0).code().color("accent1").middle());
        agenda.text((136.0, y, 768.0, 40.0), &[topic], st(26.0).bold().middle());
        agenda.text((136.0, y + 42.0, 768.0, 30.0), &["One line about this topic"], st(16.0).color("tx2"));
    }

    let mut section = Page::new();
    section.text((40.0, 40.0, 880.0, 270.0), &["02"], st(220.0).head().bold().color("#253140").bottom());
    section.text((56.0, 300.0, 848.0, 64.0), &["Section title"], st(48.0).head().bold());
    section.text((56.0, 372.0, 848.0, 30.0), &["One line about this section"], st(20.0).color("tx2"));
    section.rect((56.0, 424.0, 48.0, 3.0), "accent1");

    let mut code = Page::new();
    code.text((56.0, 48.0, 848.0, 48.0), &["Code example"], st(32.0).head().bold());
    code.round((56.0, 124.0, 560.0, 270.0), "bg2", 8.0);
    code.text(
        (84.0, 150.0, 504.0, 230.0),
        &["fn main() {", "    let greeting = \"Hello\";", "    // Replace this with your code", "    println!(\"{greeting}\");", "}"],
        st(19.0).code().spacing(1.45),
    );
    code.recolor(2, "tx2");
    code.text(
        (648.0, 124.0, 256.0, 340.0),
        &["→  First takeaway", "→  Second takeaway", "→  Third takeaway"],
        st(20.0).color("tx2").spacing(1.3).after(18.0),
    );

    let mut results = Page::new();
    results.text((56.0, 48.0, 848.0, 48.0), &["Results"], st(32.0).head().bold());
    for (i, (n, label, accent)) in
        [("2×", "First result", "accent1"), ("40%", "Second result", "accent2"), ("0", "Third result", "accent3")].iter().enumerate()
    {
        let x = 56.0 + i as f64 * 288.0;
        results.round((x, 136.0, 272.0, 220.0), "bg2", 8.0);
        results.text((x + 24.0, 168.0, 224.0, 80.0), &[n], st(56.0).head().bold().color(accent));
        results.text((x + 24.0, 260.0, 224.0, 80.0), &[label], st(18.0).color("tx2").spacing(1.2));
    }
    results.text((56.0, 384.0, 848.0, 22.0), &["How and where this was measured"], st(14.0).color("tx2"));

    let mut closing = Page::new();
    closing.text((56.0, 160.0, 848.0, 110.0), &["Thanks!"], st(72.0).head().bold().bottom());
    closing.rect((56.0, 290.0, 48.0, 3.0), "accent1");
    closing.text(
        (56.0, 312.0, 848.0, 60.0),
        &["github.com/your-name", "slides: example.com/talk"],
        st(18.0).code().color("tx2").spacing(1.3),
    );

    (look, [title, agenda, section, code, results, closing].into_iter().map(Page::slide).collect())
}

fn signal() -> (Look, Vec<Slide>) {
    let look = Look {
        name: "Signal",
        dark: false,
        heading: "Arial",
        body: "Arial",
        background: "#FFFFFF",
        text: "#0B0B0C",
        panel: "#F4F4F2",
        muted: "#55575C",
        accents: ["#FFCC00", "#2D5BFF", "#FF5A36", "#1DB57A", "#8C5CFF", "#CFCFCB"],
        hlink: "#2D5BFF",
        followed: "#8C5CFF",
    };
    const DIM: &str = "#9A9A9F";

    let mut title = Page::on("accent1");
    title.text((56.0, 120.0, 848.0, 120.0), &["Name"], st(96.0).head().bold().bottom());
    title.text((56.0, 250.0, 700.0, 100.0), &["One line on what you do."], st(36.0).head().bold());
    title.rect((0.0, 500.0, 960.0, 40.0), "tx1");
    title.text((56.0, 500.0, 848.0, 40.0), &["Event or date"], st(14.0).bold().color("accent1").middle());

    let mut statement = Page::new();
    statement.text((56.0, 96.0, 848.0, 24.0), &["THE PROBLEM"], st(14.0).bold().color("tx2"));
    statement.text(
        (56.0, 130.0, 820.0, 210.0),
        &["State the problem in one sentence people will remember."],
        st(44.0).head().bold().spacing(1.0),
    );
    statement.rect((56.0, 356.0, 160.0, 14.0), "accent1");
    statement.text((56.0, 396.0, 700.0, 40.0), &["Where this comes from"], st(18.0).color("tx2"));

    let mut section = Page::on("tx1");
    section.rect((56.0, 196.0, 40.0, 40.0), "accent1");
    section.text((56.0, 250.0, 848.0, 90.0), &["Section title"], st(64.0).head().bold().color("bg1"));

    let mut steps = Page::new();
    steps.text((56.0, 48.0, 848.0, 60.0), &["Three steps"], st(36.0).head().bold().bottom());
    for (i, step) in ["First step", "Second step", "Third step"].iter().enumerate() {
        let x = 56.0 + i as f64 * 288.0;
        steps.oval((x, 160.0, 64.0, 64.0), "accent1");
        steps.text((x, 160.0, 64.0, 64.0), &[&(i + 1).to_string()], st(28.0).bold().center().middle());
        steps.text((x, 248.0, 272.0, 34.0), &[step], st(22.0).bold());
        steps.text((x, 290.0, 256.0, 110.0), &["What happens at this step."], st(18.0).color("tx2").spacing(1.3));
    }

    let mut traction = Page::new();
    traction.text((56.0, 56.0, 400.0, 24.0), &["TRACTION"], st(14.0).bold().color("tx2"));
    traction.text((56.0, 80.0, 420.0, 190.0), &["212"], st(150.0).head().bold().bottom());
    traction.text((56.0, 280.0, 380.0, 70.0), &["What this number counts, and since when"], st(22.0).spacing(1.2));
    let values = [9.0, 14.0, 22.0, 31.0, 45.0, 63.0, 88.0, 120.0, 161.0, 212.0];
    for (i, v) in values.iter().enumerate() {
        let (x, h) = (500.0 + i as f64 * 40.0, v / 212.0 * 300.0);
        traction.rect((x, 440.0 - h, 30.0, h), if i == values.len() - 1 { "tx1" } else { "accent6" });
        traction.text((x, 446.0, 30.0, 16.0), &[&"JFMAMJJASO"[i..i + 1]], st(13.0).color("tx2").center());
    }

    let mut ask = Page::on("tx1");
    ask.text((56.0, 96.0, 848.0, 24.0), &["THE ASK"], st(14.0).bold().color(DIM));
    ask.text((56.0, 140.0, 848.0, 100.0), &["What you're asking for"], st(64.0).head().bold().color("accent1").bottom());
    ask.text((56.0, 256.0, 760.0, 90.0), &["What it will let you do"], st(28.0).color("bg1"));
    ask.text((56.0, 460.0, 848.0, 24.0), &["name@example.com"], st(16.0).color(DIM));

    (look, [title, statement, section, steps, traction, ask].into_iter().map(Page::slide).collect())
}

fn lagoon() -> (Look, Vec<Slide>) {
    let look = Look {
        name: "Lagoon",
        dark: false,
        heading: "Georgia",
        body: "Calibri",
        background: "#FFFFFF",
        text: "#12333A",
        panel: "#E6F2F2",
        muted: "#557277",
        accents: ["#0E6E73", "#FF6F59", "#FFD7A8", "#5FB3B3", "#F2B33D", "#3D5A80"],
        hlink: "#0E6E73",
        followed: "#3D5A80",
    };
    const SOFT: &str = "#CDE7E7";
    let head = |p: &mut Page, title: &str| {
        p.text((56.0, 40.0, 848.0, 64.0), &[title], st(36.0).head().bottom());
    };

    let mut title = Page::on("accent1");
    title.oval((640.0, 70.0, 250.0, 250.0), "accent2");
    title.oval((780.0, 300.0, 150.0, 150.0), "accent3");
    title.text((56.0, 170.0, 560.0, 150.0), &["Presentation title"], st(52.0).head().color("bg1").bottom());
    title.text((56.0, 334.0, 560.0, 30.0), &["Subtitle or date"], st(20.0).color(SOFT));

    let mut week = Page::new();
    head(&mut week, "This week");
    for (i, day) in ["Mon", "Tue", "Wed", "Thu", "Fri"].iter().enumerate() {
        let x = 56.0 + i as f64 * 173.0;
        week.round((x, 136.0, 156.0, 230.0), "bg2", 12.0);
        week.text((x + 16.0, 156.0, 124.0, 24.0), &[day], st(18.0).bold().color("accent1"));
        week.text((x + 16.0, 192.0, 124.0, 160.0), &["Plan for the day"], st(18.0).spacing(1.3));
    }

    let mut section = Page::on("accent2");
    section.oval((790.0, 370.0, 120.0, 120.0), "accent3");
    section.text((56.0, 210.0, 848.0, 80.0), &["Section title"], st(52.0).head().color("bg1").bottom());
    section.text((56.0, 300.0, 848.0, 30.0), &["One line about this section"], st(20.0).color("#FFE6E1"));

    let mut people = Page::new();
    head(&mut people, "The team");
    for (i, (initials, fill, ink)) in
        [("AB", "accent1", "bg1"), ("CD", "accent2", "bg1"), ("EF", "accent3", "tx1"), ("GH", "accent4", "bg1")].iter().enumerate()
    {
        let x = 56.0 + i as f64 * 212.0;
        people.oval((x, 140.0, 96.0, 96.0), fill);
        people.text((x, 140.0, 96.0, 96.0), &[initials], st(28.0).bold().color(ink).center().middle());
        people.text((x, 254.0, 196.0, 28.0), &["Name"], st(20.0).bold());
        people.text((x, 284.0, 196.0, 50.0), &["Role"], st(17.0).color("tx2"));
    }
    people.round((56.0, 380.0, 848.0, 80.0), "bg2", 12.0);
    people.text((84.0, 380.0, 792.0, 80.0), &["A short note, such as who to ask first."], st(18.0).middle());

    let mut columns = Page::new();
    head(&mut columns, "Two columns");
    for (i, heading) in ["First heading", "Second heading"].iter().enumerate() {
        let x = if i == 1 { 488.0 } else { 56.0 };
        columns.round((x, 136.0, 416.0, 236.0), "bg2", 12.0);
        columns.text((x + 28.0, 160.0, 360.0, 30.0), &[heading], st(22.0).bold().color("accent1"));
        columns.text((x + 28.0, 206.0, 360.0, 150.0), &BULLETS[..3], st(20.0).spacing(1.45));
    }

    let mut closing = Page::on("accent1");
    closing.oval((680.0, 250.0, 200.0, 200.0), "accent2");
    closing.oval((820.0, 140.0, 90.0, 90.0), "accent3");
    closing.text((56.0, 170.0, 600.0, 100.0), &["Thank you"], st(52.0).head().color("bg1").bottom());
    closing.text((56.0, 284.0, 600.0, 30.0), &["Where to send questions"], st(20.0).color(SOFT));

    (look, [title, week, section, people, columns, closing].into_iter().map(Page::slide).collect())
}

/// The deck a starter makes, as a .pptx, or None for an id that isn't one.
pub fn starter_bytes(id: &str) -> Option<office_core::Result<Vec<u8>>> {
    let (theme, dark, slides) = match id {
        "blank" => (Theme::office(), false, vec![Slide::default()]),
        _ => {
            let (look, slides) = match id {
                "plain" => plain(),
                "harbor" => harbor(),
                "graphite" => graphite(),
                "signal" => signal(),
                "lagoon" => lagoon(),
                _ => return None,
            };
            (look.theme(), look.dark, slides)
        }
    };
    let mut doc = Document::new(Kind::Slides);
    doc.theme = Some(theme);
    doc.body = Body::Slides { size: SlideSize::default(), slides };
    Some(Compiler::new().with_layer(Decks).write(&doc, Kind::Slides).and_then(|bytes| if dark { darken(&bytes) } else { Ok(bytes) }))
}

/// Maps the background and text roles to the theme's dark and light colors the other way
/// round, as PowerPoint's dark themes do, so `bg1` is the dark color on every slide.
fn darken(bytes: &[u8]) -> office_core::Result<Vec<u8>> {
    let mut pkg = Package::read(bytes)?;
    let main = pkg.main_part()?;
    for (_, master) in pkg.related(&main, office_core::opc::rels::kind::SLIDE_MASTER)? {
        let mut xml = pkg.xml(&master)?;
        if let Some(map) = xml.root.child_mut(ns::P, "clrMap") {
            for (role, slot) in [("bg1", "dk1"), ("tx1", "lt1"), ("bg2", "dk2"), ("tx2", "lt2")] {
                map.set_attr("", role, slot);
            }
        }
        pkg.replace(&master, xml.to_bytes())?;
    }
    pkg.to_bytes()
}

#[cfg(test)]
mod tests {
    use office_core::drawing::Rgb;
    use office_engine::measure::FixedMeasurer;

    use super::*;
    use crate::view::SlidesView;

    fn open(id: &str) -> SlidesView {
        SlidesView::open(&starter_bytes(id).expect("a starter").expect("it writes")).expect("it opens")
    }

    fn bright(v: &SlidesView, slide: usize, name: &str) -> f64 {
        let Rgb(r, g, b) = crate::color(&v.doc, &v.slides()[slide], &Color::scheme(name)).rgb;
        (f64::from(r) + f64::from(g) + f64::from(b)) / 765.0
    }

    #[test]
    fn every_starter_saves_and_reopens_with_its_theme() {
        let mut host = FixedMeasurer::default();
        for id in STARTERS {
            let mut v = open(id);
            let count = if id == "blank" { 1 } else { 6 };
            assert_eq!(v.slides().len(), count, "{id}");
            let theme = v.doc.theme.clone().expect("a theme");
            if id != "blank" {
                assert_eq!(theme.name.to_lowercase(), id);
            }
            let (w, h) = v.size();
            for i in 0..count {
                for e in v.elements(&mut host, i) {
                    assert!(e.x >= -0.5 && e.y >= -0.5 && e.x + e.width <= w + 0.5 && e.y + e.height <= h + 0.5, "{id} slide {i}: {e:?}");
                }
                v.paint(&mut host, i);
            }
            let reopened = SlidesView::open(&v.save().unwrap()).unwrap();
            assert_eq!(reopened.slides().len(), count, "{id}");
            assert_eq!(reopened.doc.theme.as_ref().map(|t| &t.fonts), Some(&theme.fonts), "{id}");
        }
        assert!(starter_bytes("nope").is_none());
    }

    #[test]
    fn light_and_dark_starters_map_background_and_text() {
        let light = open("harbor");
        assert!(bright(&light, 1, "bg1") > 0.9 && bright(&light, 1, "tx1") < 0.2);
        let dark = open("graphite");
        assert!(bright(&dark, 1, "bg1") < 0.2 && bright(&dark, 1, "tx1") > 0.8);
        assert!(bright(&dark, 1, "bg2") < 0.2 && bright(&dark, 1, "tx2") > 0.5);
    }

    #[test]
    fn rounded_corners_keep_their_radius() {
        let bytes = starter_bytes("graphite").unwrap().unwrap();
        let pkg = Package::read(&bytes).unwrap();
        let xml = String::from_utf8(pkg.part("ppt/slides/slide4.xml").unwrap().to_vec()).unwrap();
        // 8 pt corners on a 270 pt tall panel.
        assert!(xml.contains("prst=\"roundRect\"") && xml.contains("val 2963"), "{xml}");
    }

    #[test]
    fn new_slides_follow_the_starter() {
        let mut host = FixedMeasurer::default();
        let mut v = open("harbor");
        v.add_template(0, "title");
        let title = v.elements(&mut host, 1).remove(0);
        assert_eq!(title.text_style.and_then(|s| s.font_family).as_deref(), Some("Calibri Light"));

        // Signal's accent is yellow, so text on it is the dark color, not white.
        let mut v = open("signal");
        v.add_template(0, "comparison");
        let header = v.slides()[1].frames.iter().find(|f| f.name == "Left Header").unwrap();
        let FrameContent::Shape { text: Some(body), .. } = &header.content else { panic!("a text box") };
        assert_eq!(body.paragraphs[0].runs().next().unwrap().props.color, Some(Color::scheme("dk1")));
        let mut v = open("harbor");
        v.add_template(0, "comparison");
        let header = v.slides()[1].frames.iter().find(|f| f.name == "Left Header").unwrap();
        let FrameContent::Shape { text: Some(body), .. } = &header.content else { panic!("a text box") };
        assert_eq!(body.paragraphs[0].runs().next().unwrap().props.color, Some(Color::scheme("lt1")));
    }
}
