use std::collections::HashMap;

use pdf_writer::Content;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use svgtypes::{SimplePathSegment, SimplifyingPathParser, ViewBox};

use crate::color::Color;
use crate::fonts::FontSpec;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Matrix(pub [f64; 6]);

impl Matrix {
    pub const IDENTITY: Self = Self([1., 0., 0., 1., 0., 0.]);

    pub fn translate(x: f64, y: f64) -> Self {
        Self([1., 0., 0., 1., x, y])
    }

    pub fn scale(x: f64, y: f64) -> Self {
        Self([x, 0., 0., y, 0., 0.])
    }

    pub fn rotate(degrees: f64) -> Self {
        let (sin, cos) = degrees.to_radians().sin_cos();
        Self([cos, sin, -sin, cos, 0., 0.])
    }

    // Apply the child transform in local space, then this transform in parent space.
    pub fn then_local(self, child: Self) -> Self {
        let [a, b, c, d, e, f] = self.0;
        let [g, h, i, j, k, l] = child.0;
        Self([a * g + c * h, b * g + d * h, a * i + c * j, b * i + d * j, a * k + c * l + e, b * k + d * l + f])
    }

    pub fn point(self, x: f64, y: f64) -> [f64; 2] {
        let [a, b, c, d, e, f] = self.0;
        [a * x + c * y + e, b * x + d * y + f]
    }

    pub fn apply(self, content: &mut Content) {
        content.transform(self.0.map(|v| v as f32));
    }

    fn parse(value: &str) -> Result<Self, String> {
        let parsed = value.parse::<svgtypes::Transform>().map_err(|e| e.to_string())?;
        Ok(Self([parsed.a, parsed.b, parsed.c, parsed.d, parsed.e, parsed.f]))
    }

    fn checked(self) -> Result<Self, String> {
        if self.0.iter().any(|v| !v.is_finite() || v.abs() > 1e12) {
            return Err("Invalid SVG transform".into());
        }
        Ok(self)
    }
}

fn checked_coordinates<const N: usize>(coordinates: [f64; N]) -> Result<[f32; N], String> {
    if coordinates.iter().any(|v| !v.is_finite() || v.abs() > 1e12) {
        return Err("Invalid SVG path coordinate".into());
    }
    Ok(coordinates.map(|v| v as f32))
}

pub(crate) fn path(content: &mut Content, d: &str) -> Result<(), String> {
    use SimplePathSegment::*;
    let (mut prev, mut start) = ([0., 0.], [0., 0.]);
    let segments = SimplifyingPathParser::from(d).collect::<Result<Vec<_>, _>>().map_err(|e| format!("Invalid SVG path: {e}"))?;
    for item in segments {
        match item {
            MoveTo { x, y } => {
                let [fx, fy] = checked_coordinates([x, y])?;
                content.move_to(fx, fy);
                start = [x, y];
                prev = start;
            }
            LineTo { x, y } => {
                let [fx, fy] = checked_coordinates([x, y])?;
                content.line_to(fx, fy);
                prev = [x, y];
            }
            CurveTo { x1, y1, x2, y2, x, y } => {
                let [x1, y1, x2, y2, fx, fy] = checked_coordinates([x1, y1, x2, y2, x, y])?;
                content.cubic_to(x1, y1, x2, y2, fx, fy);
                prev = [x, y];
            }
            Quadratic { x1, y1, x, y } => {
                let [px, py] = prev;
                let [x1, y1, x2, y2, fx, fy] =
                    checked_coordinates([(px + 2. * x1) / 3., (py + 2. * y1) / 3., (x + 2. * x1) / 3., (y + 2. * y1) / 3., x, y])?;
                content.cubic_to(x1, y1, x2, y2, fx, fy);
                prev = [x, y];
            }
            ClosePath => {
                content.close_path();
                prev = start;
            }
        }
    }
    Ok(())
}

type Attrs = HashMap<String, String>;

#[derive(Default)]
struct Node {
    name: String,
    attrs: Attrs,
    text: String,
    children: Vec<usize>,
}

fn element(event: &BytesStart<'_>) -> Result<Node, String> {
    let name = event.local_name().as_ref().to_owned();
    let mut attrs = Attrs::new();
    for attr in event.attributes() {
        let attr = attr.map_err(|e| e.to_string())?;
        let key = attr.key.as_ref().to_owned();
        let value = attr.normalized_value(quick_xml::XmlVersion::Implicit1_0).map_err(|e| e.to_string())?.into_owned();
        attrs.insert(key, value);
    }
    if let Some(style) = attrs.get("style").cloned() {
        for (key, value) in style.split(';').filter_map(|d| d.split_once(':')) {
            attrs.insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }
    Ok(Node { name, attrs, ..Node::default() })
}

fn parse(svg: &str) -> Result<Vec<Node>, String> {
    let mut reader = Reader::from_str(svg);
    let mut nodes = vec![Node::default()];
    let mut parents = vec![0];
    loop {
        let event = reader.read_event().map_err(|e| format!("Invalid SVG: {e}"))?;
        let parent = *parents.last().ok_or("Invalid SVG nesting")?;
        match event {
            Event::Empty(ref e) | Event::Start(ref e) => {
                let index = nodes.len();
                nodes.push(element(e)?);
                nodes[parent].children.push(index);
                if matches!(event, Event::Start(_)) {
                    if parents.len() >= 128 {
                        return Err("SVG nesting is too deep".into());
                    }
                    parents.push(index);
                }
            }
            Event::End(_) => {
                parents.pop();
            }
            Event::Text(value) => nodes[parent].text.push_str(&value.xml_content(quick_xml::XmlVersion::Implicit1_0)),
            Event::CData(value) => nodes[parent].text.push_str(&value.xml_content(quick_xml::XmlVersion::Implicit1_0)),
            Event::GeneralRef(value) => {
                if let Some(ch) = value.resolve_char_ref().map_err(|e| e.to_string())? {
                    nodes[parent].text.push(ch);
                } else if let Some(decoded) = quick_xml::escape::resolve_xml_entity(value.as_ref()) {
                    nodes[parent].text.push_str(decoded);
                } else {
                    return Err("Unknown SVG entity".into());
                }
            }
            Event::DocType(_) => return Err("SVG document types are unsupported".into()),
            Event::Eof if parents.len() == 1 => return Ok(nodes),
            Event::Eof => return Err("Unclosed SVG element".into()),
            _ => {}
        }
    }
}

fn number(value: &str) -> Option<f64> {
    let parsed: f64 = value.trim().trim_end_matches("px").parse().ok()?;
    (parsed.is_finite() && parsed.abs() <= 1e12).then_some(parsed)
}

impl Node {
    fn get(&self, key: &str) -> Option<&str> {
        self.attrs.get(key).map(String::as_str)
    }

    fn num(&self, key: &str) -> f64 {
        self.get(key).and_then(number).unwrap_or(0.)
    }
}

#[derive(Clone)]
pub(crate) struct SvgStyle {
    pub fill: Option<Color>,
    pub stroke: Option<Color>,
    color: Color,
    opacity: f32,
    fill_opacity: f32,
    stroke_opacity: f32,
    pub stroke_width: f64,
    pub dash: Option<String>,
    pub line_cap: Option<String>,
    pub line_join: Option<String>,
    pub even_odd: bool,
    font_family: String,
    font_size: f64,
    bold: bool,
    italic: bool,
    anchor: f64,
}

impl Default for SvgStyle {
    fn default() -> Self {
        Self {
            fill: Some(Color::BLACK),
            stroke: None,
            color: Color::BLACK,
            opacity: 1.,
            fill_opacity: 1.,
            stroke_opacity: 1.,
            stroke_width: 1.,
            dash: None,
            line_cap: None,
            line_join: None,
            even_odd: false,
            font_family: "Calibri".into(),
            font_size: 16.,
            bold: false,
            italic: false,
            anchor: 0.,
        }
    }
}

impl SvgStyle {
    fn apply(&mut self, node: &Node) {
        fn opacity(value: &str) -> Option<f32> {
            if let Some(percent) = value.strip_suffix('%') {
                Some((number(percent)? / 100.).clamp(0., 1.) as f32)
            } else {
                Some(number(value)?.clamp(0., 1.) as f32)
            }
        }
        if let Some(color) = node.get("color").and_then(Color::parse) {
            self.color = color;
        }
        for (key, target) in [("fill", &mut self.fill), ("stroke", &mut self.stroke)] {
            match node.get(key) {
                None | Some("inherit") => {}
                Some("currentColor") => *target = Some(self.color),
                Some(value) => *target = Color::parse(value),
            }
        }
        self.opacity *= node.get("opacity").and_then(opacity).unwrap_or(1.);
        if let Some(value) = node.get("fill-opacity").and_then(opacity) {
            self.fill_opacity = value;
        }
        if let Some(value) = node.get("stroke-opacity").and_then(opacity) {
            self.stroke_opacity = value;
        }
        if let Some(value) = node.get("stroke-width").and_then(number) {
            self.stroke_width = value;
        }
        if let Some(value) = node.get("stroke-dasharray") {
            self.dash = Some(value.to_owned());
        }
        if let Some(value) = node.get("stroke-linecap") {
            self.line_cap = Some(value.to_owned());
        }
        if let Some(value) = node.get("stroke-linejoin") {
            self.line_join = Some(value.to_owned());
        }
        if let Some(value) = node.get("fill-rule") {
            self.even_odd = value == "evenodd";
        }
        if let Some(value) = node.get("font-family") {
            self.font_family = value.to_owned();
        }
        if let Some(value) = node.get("font-size").and_then(number).filter(|v| *v > 0.) {
            self.font_size = value;
        }
        if let Some(value) = node.get("font-weight") {
            self.bold = value == "bold" || value.parse::<u16>().is_ok_and(|weight| weight >= 600);
        }
        if let Some(value) = node.get("font-style") {
            self.italic = matches!(value, "italic" | "oblique");
        }
        if let Some(value) = node.get("text-anchor") {
            self.anchor = match value {
                "middle" => 0.5,
                "end" => 1.,
                _ => 0.,
            };
        }
    }

    pub fn fill_color(&self) -> Option<Color> {
        self.fill.map(|c| c.with_opacity(self.opacity * self.fill_opacity))
    }

    pub fn stroke_color(&self) -> Option<Color> {
        self.stroke.map(|c| c.with_opacity(self.opacity * self.stroke_opacity))
    }
}

pub(crate) enum Geometry {
    Path(String),
    Rect([f64; 4], f64, f64),
    Line([f64; 4]),
    Ellipse(f64, f64, f64, f64),
    Text { text: String, x: f64, y: f64, font: FontSpec, anchor: f64 },
}

impl Geometry {
    pub fn write_path(&self, content: &mut Content) -> Result<(), String> {
        match self {
            Self::Path(d) => path(content, d)?,
            Self::Rect([x, y, w, h], rx, ry) => {
                let [x, y, w, h] = [*x, *y, *w, *h];
                let (rx, ry) = (rx.max(0.).min(w / 2.), ry.max(0.).min(h / 2.));
                if w <= 0. || h <= 0. {
                    return Ok(());
                }
                if rx == 0. || ry == 0. {
                    content.rect(x as f32, y as f32, w as f32, h as f32);
                } else {
                    let d = format!("M {} {y} H {} A {rx} {ry} 0 0 1 {} {} V {} A {rx} {ry} 0 0 1 {} {} H {} A {rx} {ry} 0 0 1 {x} {} V {} A {rx} {ry} 0 0 1 {} {y} Z",
                        x+rx, x+w-rx, x+w, y+ry, y+h-ry, x+w-rx, y+h, x+rx, y+h-ry, y+ry, x+rx);
                    path(content, &d)?;
                }
            }
            Self::Line([x1, y1, x2, y2]) => {
                content.move_to(*x1 as f32, *y1 as f32).line_to(*x2 as f32, *y2 as f32);
            }
            Self::Ellipse(x, y, rx, ry) if *rx > 0. && *ry > 0. => {
                let d = format!("M {} {y} A {rx} {ry} 0 1 0 {} {y} A {rx} {ry} 0 1 0 {} {y} Z", x + rx, x - rx, x + rx);
                path(content, &d)?;
            }
            _ => {}
        }
        Ok(())
    }
}

pub(crate) struct SvgDraw {
    pub transform: Matrix,
    pub style: SvgStyle,
    pub geometry: Geometry,
}

struct Svg<'a> {
    nodes: &'a [Node],
    defs: HashMap<&'a str, usize>,
    visited: usize,
}

impl Svg<'_> {
    fn collect(
        &mut self,
        index: usize,
        transform: Matrix,
        mut style: SvgStyle,
        output: &mut Vec<SvgDraw>,
        depth: usize,
    ) -> Result<(), String> {
        self.visited += 1;
        if depth >= 128 || self.visited > 250_000 {
            return Err("SVG expansion is too large or cyclic".into());
        }
        let node = &self.nodes[index];
        if node.get("display") == Some("none") || node.get("visibility") == Some("hidden") {
            return Ok(());
        }
        let mut transform =
            transform.then_local(node.get("transform").map(Matrix::parse).transpose()?.unwrap_or(Matrix::IDENTITY)).checked()?;
        style.apply(node);
        let geometry = match node.name.as_str() {
            "svg" | "g" | "symbol" | "a" => {
                for &child in &node.children {
                    self.collect(child, transform, style.clone(), output, depth + 1)?;
                }
                None
            }
            "use" => {
                if let Some(&target) = node
                    .get("href")
                    .or_else(|| node.get("xlink:href"))
                    .and_then(|href| href.strip_prefix('#'))
                    .and_then(|id| self.defs.get(id))
                {
                    transform = transform.then_local(Matrix::translate(node.num("x"), node.num("y")));
                    self.collect(target, transform, style, output, depth + 1)?;
                }
                return Ok(());
            }
            "path" => node.get("d").map(|d| Geometry::Path(d.to_owned())),
            "rect" => {
                let rx = node.get("rx").and_then(number);
                let ry = node.get("ry").and_then(number);
                Some(Geometry::Rect(
                    [node.num("x"), node.num("y"), node.num("width"), node.num("height")],
                    rx.or(ry).unwrap_or(0.),
                    ry.or(rx).unwrap_or(0.),
                ))
            }
            "line" => Some(Geometry::Line([node.num("x1"), node.num("y1"), node.num("x2"), node.num("y2")])),
            "circle" => Some(Geometry::Ellipse(node.num("cx"), node.num("cy"), node.num("r"), node.num("r"))),
            "ellipse" => Some(Geometry::Ellipse(node.num("cx"), node.num("cy"), node.num("rx"), node.num("ry"))),
            "text" => {
                let font = FontSpec::from_family(&style.font_family, style.font_size, style.bold, style.italic);
                Some(Geometry::Text {
                    text: node.text.split_whitespace().collect::<Vec<_>>().join(" "),
                    x: node.num("x"),
                    y: node.num("y"),
                    font,
                    anchor: style.anchor,
                })
            }
            _ => None,
        };
        if let Some(geometry) = geometry {
            output.push(SvgDraw { transform, style, geometry });
        }
        Ok(())
    }
}

pub(crate) fn translate(svg: &str, width: f64, height: f64) -> Result<Vec<SvgDraw>, String> {
    let nodes = parse(svg)?;
    let [root] = nodes[0].children.as_slice() else {
        return Err("SVG must have one root".into());
    };
    let node = &nodes[*root];
    if node.name != "svg" {
        return Err("Missing SVG root".into());
    }
    let transform = match node.get("viewBox") {
        Some(value) => {
            let vb = value.parse::<ViewBox>().map_err(|e| e.to_string())?;
            if ![vb.x, vb.y, vb.w, vb.h].iter().all(|v| v.is_finite()) || vb.w <= 0. || vb.h <= 0. {
                return Err("Invalid SVG viewBox".into());
            }
            let (sx, sy) = (width / vb.w, height / vb.h);
            let aspect = node.get("preserveAspectRatio").unwrap_or("xMidYMid meet");
            if aspect == "none" {
                Matrix::scale(sx, sy).then_local(Matrix::translate(-vb.x, -vb.y))
            } else {
                let s = if aspect.ends_with("slice") { sx.max(sy) } else { sx.min(sy) };
                let align = |axis: &str| {
                    if aspect.contains(&format!("{axis}Min")) {
                        0.
                    } else if aspect.contains(&format!("{axis}Max")) {
                        1.
                    } else {
                        0.5
                    }
                };
                Matrix::translate((width - vb.w * s) * align("x"), (height - vb.h * s) * align("Y"))
                    .then_local(Matrix::scale(s, s))
                    .then_local(Matrix::translate(-vb.x, -vb.y))
            }
        }
        None => Matrix::scale(
            width / node.get("width").and_then(number).filter(|w| *w > 0.).unwrap_or(width),
            height / node.get("height").and_then(number).filter(|h| *h > 0.).unwrap_or(height),
        ),
    };
    let defs = nodes.iter().enumerate().filter_map(|(i, node)| Some((node.get("id")?, i))).collect();
    let mut output = Vec::new();
    Svg { nodes: &nodes, defs, visited: 0 }.collect(*root, transform, SvgStyle::default(), &mut output, 0)?;
    Ok(output)
}
