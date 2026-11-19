//! ECMA-376 DrawingML geometry, evaluated in EMU and emitted as SVG in CSS pixels.
//!
//! The 187 preset definitions are in `geometry/presets.dat`. Its header records
//! the source URL and checksum. The table preserves formulas, paths, paint modes,
//! extrusion flags and text rectangles; handles and connection sites are omitted.
//! `@` starts a preset, `a`/`g` define adjustment/ordinary guides, `r` defines the
//! text rectangle, and `p` starts a path (w, h, fill, stroke, extrusionOk; `-` means
//! an omitted size). Uppercase SVG command letters encode the DrawingML commands;
//! `A` retains DrawingML's wR, hR, stAng, swAng rather than SVG arc operands.
//! A `OnceLock` compiles it into indexed operands, so preset rendering allocates
//! only the returned paths and their strings. Custom XML uses the same compiler.

use std::{fmt::Write, sync::OnceLock};

use office_core::xml::{ns, Element};

const EMU_PER_PX: f64 = 9525.0;
const ANGLE_RAD: f64 = std::f64::consts::PI / 10_800_000.0;
const FULL_TURN: f64 = 21_600_000.0;

/// One SVG subpath collection, relative to the shape's top-left corner, in pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapePath {
    pub d: String,
    pub fill: PathFill,
    pub stroke: bool,
}

/// DrawingML's per-path paint modifier, applied to the shape's fill by the painter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathFill {
    Norm,
    None,
    Lighten,
    LightenLess,
    Darken,
    DarkenLess,
}

/// Shape outlines and the text rectangle `(left, top, right, bottom)` in pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Geometry {
    pub paths: Vec<ShapePath>,
    pub text_rect: (f64, f64, f64, f64),
}

/// Evaluate an ECMA-376 preset at the given CSS pixel size.
///
/// `adjust` is the shape's own avLst overrides as `(name, formula-string)`,
/// e.g. `("adj", "val 25000")`. Unknown presets, malformed overrides of known
/// adjustments, and negative or non-finite dimensions return `None`.
pub fn preset(name: &str, w: f64, h: f64, adjust: &[(String, String)]) -> Option<Geometry> {
    let presets = definitions();
    let index = presets.binary_search_by_key(&name, |(name, _)| *name).ok()?;
    presets[index].1.evaluate(w, h, adjust)
}

/// Compile and evaluate an `a:custGeom`, including its adjustments and text box.
/// Invalid formulas, unresolved guides and malformed paths return `None`.
pub fn custom(cust_geom: &Element, w: f64, h: f64) -> Option<Geometry> {
    if !cust_geom.is(ns::A, "custGeom") {
        return None;
    }
    Definition::from_xml(cust_geom)?.evaluate(w, h, &[])
}

/// Every supported ECMA-376 preset name, in lexical order.
pub fn preset_names() -> impl Iterator<Item = &'static str> {
    definitions().iter().map(|(name, _)| *name)
}

#[derive(Clone, Copy, Debug)]
enum Builtin {
    Width(f64),
    Height(f64),
    Short(f64),
    Long,
}

impl Builtin {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "w" | "r" => Some(Self::Width(1.0)),
            "h" | "b" => Some(Self::Height(1.0)),
            "hc" => Some(Self::Width(2.0)),
            "vc" => Some(Self::Height(2.0)),
            "ss" => Some(Self::Short(1.0)),
            "ls" => Some(Self::Long),
            _ => {
                let (make, divisor): (fn(f64) -> Self, _) = if let Some(d) = name.strip_prefix("wd") {
                    (Self::Width, d)
                } else if let Some(d) = name.strip_prefix("hd") {
                    (Self::Height, d)
                } else {
                    (Self::Short, name.strip_prefix("ssd")?)
                };
                let divisor = divisor.parse::<f64>().ok()?;
                (divisor > 0.0 && divisor.is_finite()).then(|| make(divisor))
            }
        }
    }

    fn value(self, w: f64, h: f64) -> f64 {
        match self {
            Self::Width(d) => w / d,
            Self::Height(d) => h / d,
            Self::Short(d) => w.min(h) / d,
            Self::Long => w.max(h),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Operand {
    Constant(f64),
    Builtin(Builtin),
    Guide(usize),
}

impl Operand {
    fn compile(token: &str, guides: &[Guide]) -> Option<Self> {
        if let Ok(n) = token.parse::<f64>() {
            return n.is_finite().then_some(Self::Constant(n));
        }
        if let Some(i) = guides.iter().rposition(|g| g.name == token) {
            return Some(Self::Guide(i));
        }
        if matches!(token, "l" | "t") {
            return Some(Self::Constant(0.0));
        }
        if let Some((n, d)) = token.split_once("cd") {
            let numerator = if n.is_empty() { 1.0 } else { n.parse::<f64>().ok()? };
            let denominator = d.parse::<f64>().ok()?;
            let angle = FULL_TURN * numerator / denominator;
            return (denominator > 0.0 && angle.is_finite()).then_some(Self::Constant(angle));
        }
        Builtin::parse(token).map(Self::Builtin)
    }

    fn value(self, values: &[f64], w: f64, h: f64) -> f64 {
        match self {
            Self::Constant(n) => n,
            Self::Builtin(b) => b.value(w, h),
            Self::Guide(i) => values[i],
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Op {
    MulDiv,
    AddSub,
    AddDiv,
    If,
    Abs,
    Atan2,
    CosAtan2,
    Cos,
    Max,
    Min,
    Mod,
    Pin,
    SinAtan2,
    Sin,
    Sqrt,
    Tan,
    Val,
}

impl Op {
    fn parse(token: &str) -> Option<(Self, usize)> {
        Some(match token {
            "*/" => (Self::MulDiv, 3),
            "+-" => (Self::AddSub, 3),
            "+/" => (Self::AddDiv, 3),
            "?:" => (Self::If, 3),
            "abs" => (Self::Abs, 1),
            "at2" => (Self::Atan2, 2),
            "cat2" => (Self::CosAtan2, 3),
            "cos" => (Self::Cos, 2),
            "max" => (Self::Max, 2),
            "min" => (Self::Min, 2),
            "mod" => (Self::Mod, 3),
            "pin" => (Self::Pin, 3),
            "sat2" => (Self::SinAtan2, 3),
            "sin" => (Self::Sin, 2),
            "sqrt" => (Self::Sqrt, 1),
            "tan" => (Self::Tan, 2),
            "val" => (Self::Val, 1),
            _ => return None,
        })
    }

    fn evaluate(self, [x, y, z]: [f64; 3]) -> f64 {
        let div = |a, b| if b == 0.0 { 0.0 } else { a / b };
        let result = match self {
            Self::MulDiv => div(x * y, z),
            Self::AddSub => x + y - z,
            Self::AddDiv => div(x + y, z),
            Self::If => {
                if x > 0.0 {
                    y
                } else {
                    z
                }
            }
            Self::Abs => x.abs(),
            Self::Atan2 => y.atan2(x) / ANGLE_RAD,
            Self::CosAtan2 => x * z.atan2(y).cos(),
            Self::Cos => x * (y * ANGLE_RAD).cos(),
            Self::Max => x.max(y),
            Self::Min => x.min(y),
            Self::Mod => x.hypot(y).hypot(z),
            Self::Pin => {
                if y < x {
                    x
                } else if y > z {
                    z
                } else {
                    y
                }
            }
            Self::SinAtan2 => x * z.atan2(y).sin(),
            Self::Sin => x * (y * ANGLE_RAD).sin(),
            Self::Sqrt => x.max(0.0).sqrt(),
            Self::Tan => x * (y * ANGLE_RAD).tan(),
            Self::Val => x,
        };
        // Degenerate dimensions can cause division by zero in otherwise valid
        // Office formulas. Keep those shapes drawable, with collapsed guides.
        if result.is_finite() {
            result
        } else {
            0.0
        }
    }
}

#[derive(Debug)]
struct Formula {
    op: Op,
    args: [Operand; 3],
}

impl Formula {
    fn compile(source: &str, guides: &[Guide]) -> Option<Self> {
        let mut tokens = source.split_ascii_whitespace();
        let (op, arity) = Op::parse(tokens.next()?)?;
        let mut args = [Operand::Constant(0.0); 3];
        for arg in &mut args[..arity] {
            *arg = Operand::compile(tokens.next()?, guides)?;
        }
        // Eight formulas in the ECMA circular-arrow definitions have a fourth,
        // redundant zero after `+-`. Office readers ignore that operand.
        match tokens.next() {
            None => (),
            Some("0") if matches!(op, Op::AddSub) && tokens.next().is_none() => (),
            _ => return None,
        }
        Some(Self { op, args })
    }

    fn evaluate(&self, values: &[f64], w: f64, h: f64) -> f64 {
        self.op.evaluate(self.args.map(|a| a.value(values, w, h)))
    }
}

#[derive(Debug)]
struct Guide {
    name: String,
    formula: Formula,
}

#[derive(Debug)]
enum Command {
    Move([Operand; 2]),
    Line([Operand; 2]),
    Arc([Operand; 4]),
    Quad([Operand; 4]),
    Cubic([Operand; 6]),
    Close,
}

#[derive(Debug)]
struct Path {
    width: Option<f64>,
    height: Option<f64>,
    fill: PathFill,
    stroke: bool,
    // Preserved for consumers extending the module to 3D. SVG has no extrusion.
    _extrusion_ok: bool,
    commands: Vec<Command>,
}

#[derive(Debug, Default)]
struct Definition {
    guides: Vec<Guide>,
    adjustments: usize,
    rect: Option<[Operand; 4]>,
    paths: Vec<Path>,
}

fn operands<const N: usize>(tokens: &str, guides: &[Guide]) -> Option<[Operand; N]> {
    let mut tokens = tokens.split_ascii_whitespace();
    let mut out = [Operand::Constant(0.0); N];
    for item in &mut out {
        *item = Operand::compile(tokens.next()?, guides)?;
    }
    if tokens.next().is_some() {
        return None;
    }
    Some(out)
}

fn fill(value: &str) -> Option<PathFill> {
    Some(match value {
        "norm" => PathFill::Norm,
        "none" => PathFill::None,
        "lighten" => PathFill::Lighten,
        "lightenLess" => PathFill::LightenLess,
        "darken" => PathFill::Darken,
        "darkenLess" => PathFill::DarkenLess,
        _ => return None,
    })
}

fn boolean(value: &str) -> Option<bool> {
    match value {
        "1" | "true" => Some(true),
        "0" | "false" => Some(false),
        _ => None,
    }
}

fn coordinate_size(value: Option<&str>) -> Option<Option<f64>> {
    value.map(|s| s.parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0)).map_or(Some(None), |v| v.map(Some))
}

impl Definition {
    fn add_guide(&mut self, name: &str, formula: &str, adjustment: bool) -> Option<()> {
        let formula = Formula::compile(formula, &self.guides)?;
        self.guides.push(Guide { name: name.to_owned(), formula });
        self.adjustments += usize::from(adjustment);
        Some(())
    }

    fn from_table(source: &str) -> Option<Self> {
        let mut def = Self::default();
        for line in source.lines() {
            let (kind, rest) = line.split_once(' ').unwrap_or((line, ""));
            match kind {
                "a" | "g" => {
                    let (name, formula) = rest.split_once(' ')?;
                    def.add_guide(name, formula, kind == "a")?;
                }
                "r" => def.rect = Some(operands(rest, &def.guides)?),
                "p" => {
                    let mut tokens = rest.split_ascii_whitespace();
                    let width = tokens.next()?;
                    let height = tokens.next()?;
                    def.paths.push(Path {
                        width: coordinate_size((width != "-").then_some(width))?,
                        height: coordinate_size((height != "-").then_some(height))?,
                        fill: fill(tokens.next()?)?,
                        stroke: boolean(tokens.next()?)?,
                        _extrusion_ok: boolean(tokens.next()?)?,
                        commands: Vec::new(),
                    });
                }
                "" => (),
                _ => {
                    let cmd = match kind {
                        "M" => Command::Move(operands(rest, &def.guides)?),
                        "L" => Command::Line(operands(rest, &def.guides)?),
                        "A" => Command::Arc(operands(rest, &def.guides)?),
                        "Q" => Command::Quad(operands(rest, &def.guides)?),
                        "C" => Command::Cubic(operands(rest, &def.guides)?),
                        "Z" => Command::Close,
                        _ => return None,
                    };
                    def.paths.last_mut()?.commands.push(cmd);
                }
            }
        }
        Some(def)
    }

    fn from_xml(root: &Element) -> Option<Self> {
        let mut def = Self::default();
        for (list, adjustment) in [("avLst", true), ("gdLst", false)] {
            if let Some(list) = root.child(ns::A, list) {
                for gd in list.children_named(ns::A, "gd") {
                    def.add_guide(gd.attr("name")?, gd.attr("fmla")?, adjustment)?;
                }
            }
        }
        if let Some(rect) = root.child(ns::A, "rect") {
            let mut coords = [Operand::Constant(0.0); 4];
            for (out, attr) in coords.iter_mut().zip(["l", "t", "r", "b"]) {
                *out = Operand::compile(rect.attr(attr)?, &def.guides)?;
            }
            def.rect = Some(coords);
        }
        for path in root.child(ns::A, "pathLst")?.children_named(ns::A, "path") {
            let mut compiled = Path {
                width: coordinate_size(path.attr("w"))?,
                height: coordinate_size(path.attr("h"))?,
                fill: fill(path.attr("fill").unwrap_or("norm"))?,
                stroke: boolean(path.attr("stroke").unwrap_or("1"))?,
                _extrusion_ok: boolean(path.attr("extrusionOk").unwrap_or("1"))?,
                commands: Vec::new(),
            };
            for cmd in path.elements() {
                if !cmd.in_ns(ns::A) {
                    return None;
                }
                let command = match cmd.local.as_str() {
                    "close" => Command::Close,
                    "arcTo" => {
                        let mut args = [Operand::Constant(0.0); 4];
                        for (out, attr) in args.iter_mut().zip(["wR", "hR", "stAng", "swAng"]) {
                            *out = Operand::compile(cmd.attr(attr)?, &def.guides)?;
                        }
                        Command::Arc(args)
                    }
                    "moveTo" => Command::Move(xml_points(cmd, &def.guides)?),
                    "lnTo" => Command::Line(xml_points(cmd, &def.guides)?),
                    "quadBezTo" => Command::Quad(xml_points(cmd, &def.guides)?),
                    "cubicBezTo" => Command::Cubic(xml_points(cmd, &def.guides)?),
                    _ => return None,
                };
                if compiled.commands.is_empty() && !matches!(command, Command::Move(_)) {
                    return None;
                }
                compiled.commands.push(command);
            }
            def.paths.push(compiled);
        }
        Some(def)
    }

    fn evaluate(&self, w: f64, h: f64, adjust: &[(String, String)]) -> Option<Geometry> {
        if w < 0.0 || h < 0.0 || !(w * EMU_PER_PX).is_finite() || !(h * EMU_PER_PX).is_finite() {
            return None;
        }
        let (w, h) = (w * EMU_PER_PX, h * EMU_PER_PX);
        // The largest preset has 244 guides. Custom geometry may have more;
        // only that case needs a temporary heap allocation.
        let mut stack = [0.0; 256];
        let mut heap;
        let values = if self.guides.len() <= stack.len() {
            &mut stack[..self.guides.len()]
        } else {
            heap = vec![0.0; self.guides.len()];
            &mut heap[..]
        };
        for (i, guide) in self.guides.iter().enumerate() {
            let override_formula =
                if i < self.adjustments { adjust.iter().rev().find(|(name, _)| *name == guide.name).map(|(_, f)| f) } else { None };
            values[i] = if let Some(source) = override_formula {
                Formula::compile(source, &self.guides[..i])?.evaluate(values, w, h)
            } else {
                guide.formula.evaluate(values, w, h)
            };
        }
        let [l, t, r, b] = self.rect.map_or([0.0, 0.0, w, h], |rect| rect.map(|a| a.value(values, w, h)));
        let mut paths = Vec::with_capacity(self.paths.len());
        for path in &self.paths {
            paths.push(path.render(values, w, h)?);
        }
        Some(Geometry { paths, text_rect: (l / EMU_PER_PX, t / EMU_PER_PX, r / EMU_PER_PX, b / EMU_PER_PX) })
    }
}

fn xml_points<const N: usize>(cmd: &Element, guides: &[Guide]) -> Option<[Operand; N]> {
    let mut out = [Operand::Constant(0.0); N];
    let mut points = cmd.children_named(ns::A, "pt");
    for pair in out.as_chunks_mut::<2>().0 {
        let pt = points.next()?;
        pair[0] = Operand::compile(pt.attr("x")?, guides)?;
        pair[1] = Operand::compile(pt.attr("y")?, guides)?;
    }
    if points.next().is_some() {
        return None;
    }
    Some(out)
}

fn definitions() -> &'static [(&'static str, Definition)] {
    static DEFINITIONS: OnceLock<Vec<(&'static str, Definition)>> = OnceLock::new();
    DEFINITIONS.get_or_init(|| parse_definitions(include_str!("geometry/presets.dat")))
}

/// The table's entries by name. A checkout with CRLF line endings (Git's default on Windows)
/// must still give `rect`, not `rect\r`.
fn parse_definitions(table: &'static str) -> Vec<(&'static str, Definition)> {
    let mut definitions: Vec<_> = table
        .split('@')
        .skip(1)
        .map(|entry| {
            let (name, source) = entry.split_once('\n').expect("preset name");
            let name = name.trim_end_matches('\r');
            (name, Definition::from_table(source).unwrap_or_else(|| panic!("invalid embedded preset {name}")))
        })
        .collect();
    definitions.sort_unstable_by_key(|(name, _)| *name);
    definitions
}

/// Append a rounded decimal without allocating a temporary string.
fn number(out: &mut String, n: f64) {
    let start = out.len();
    // Avoid negative zero, including values which round to zero.
    let n = if n.abs() < 0.005 { 0.0 } else { n };
    write!(out, "{n:.2}").expect("writing to String");
    if out.as_bytes().last() == Some(&b'0') {
        out.pop();
        if out.as_bytes().last() == Some(&b'0') {
            out.pop();
        }
        if out.as_bytes().last() == Some(&b'.') {
            out.pop();
        }
    }
    debug_assert!(out.len() > start);
}

fn command(out: &mut String, letter: char, coords: &[f64]) {
    out.push(letter);
    for (i, n) in coords.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        number(out, *n);
    }
}

impl Path {
    fn render(&self, values: &[f64], w: f64, h: f64) -> Option<ShapePath> {
        let scale = |size: Option<f64>, actual| match size {
            Some(0.0) => 0.0,
            Some(size) => actual / size / EMU_PER_PX,
            None => 1.0 / EMU_PER_PX,
        };
        let (sx, sy) = (scale(self.width, w), scale(self.height, h));
        let mut out = String::with_capacity(self.commands.len() * 24);
        let mut current = [0.0; 2];
        let mut start = current;
        for cmd in &self.commands {
            match cmd {
                Command::Move(args) | Command::Line(args) => {
                    current = args.map(|a| a.value(values, w, h));
                    let letter = if matches!(cmd, Command::Move(_)) {
                        start = current;
                        'M'
                    } else {
                        'L'
                    };
                    command(&mut out, letter, &[current[0] * sx, current[1] * sy]);
                }
                Command::Quad(args) => {
                    let mut coords = args.map(|a| a.value(values, w, h));
                    current = [coords[2], coords[3]];
                    for pair in coords.as_chunks_mut::<2>().0 {
                        pair[0] *= sx;
                        pair[1] *= sy;
                    }
                    command(&mut out, 'Q', &coords);
                }
                Command::Cubic(args) => {
                    let mut coords = args.map(|a| a.value(values, w, h));
                    current = [coords[4], coords[5]];
                    for pair in coords.as_chunks_mut::<2>().0 {
                        pair[0] *= sx;
                        pair[1] *= sy;
                    }
                    command(&mut out, 'C', &coords);
                }
                Command::Arc(args) => {
                    let [rx, ry, angle, sweep] = args.map(|a| a.value(values, w, h));
                    arc(&mut out, &mut current, [rx.abs(), ry.abs()], angle, sweep, [sx, sy])?;
                }
                Command::Close => {
                    out.push('Z');
                    current = start;
                }
            }
        }
        // This also catches overflow from hostile custom coordinates or sizes.
        if out.contains("inf") || out.contains("NaN") {
            return None;
        }
        Some(ShapePath { d: out, fill: self.fill, stroke: self.stroke })
    }
}

/// The ray at a DrawingML angle intersects the ellipse at this offset. DrawingML
/// angles are polar, whereas `(rx*cos(a), ry*sin(a))` uses a parametric angle.
fn ellipse_point(rx: f64, ry: f64, angle: f64) -> [f64; 2] {
    let angle = angle.rem_euclid(FULL_TURN);
    // Exact axis intersections also define the limit of a collapsed ellipse.
    // Evaluating sin(PI) numerically would give the wrong point when rx = 0.
    match angle {
        0.0 => return [rx, 0.0],
        5_400_000.0 => return [0.0, ry],
        10_800_000.0 => return [-rx, 0.0],
        16_200_000.0 => return [0.0, -ry],
        _ => (),
    }
    let (sin, cos) = (angle * ANGLE_RAD).sin_cos();
    let (x, y) = (ry * cos, rx * sin);
    let length = x.hypot(y);
    if length == 0.0 {
        [0.0, 0.0]
    } else {
        [rx * (x / length), ry * (y / length)]
    }
}

fn arc(out: &mut String, current: &mut [f64; 2], [rx, ry]: [f64; 2], angle: f64, sweep: f64, [sx, sy]: [f64; 2]) -> Option<()> {
    if sweep == 0.0 {
        return Some(());
    }
    let start = ellipse_point(rx, ry, angle);
    let center = [current[0] - start[0], current[1] - start[1]];
    // A full ellipse needs multiple SVG arcs: coincident endpoints draw nothing.
    // At most half a polar turn per arc also avoids large-arc ambiguity after
    // rounding. Preserve negative sweeps and sweeps longer than one turn.
    let count = (sweep.abs() / (FULL_TURN / 2.0)).ceil();
    // Limit output work for malformed custom input, never for spec presets.
    if count > 4096.0 {
        return None;
    }
    let count = count as usize;
    for i in 1..=count {
        let point = ellipse_point(rx, ry, angle + sweep * i as f64 / count as f64);
        *current = [center[0] + point[0], center[1] + point[1]];
        if rx * sx == 0.0 || ry * sy == 0.0 {
            command(out, 'L', &[current[0] * sx, current[1] * sy]);
        } else {
            command(out, 'A', &[rx * sx, ry * sy, 0.0, 0.0, f64::from(sweep > 0.0), current[0] * sx, current[1] * sy]);
        }
    }
    Some(())
}

#[cfg(test)]
mod tests;
