//! Conditional formatting: a worksheet's rules, worked out against the values its cells hold.
//!
//! Rules come from `<conditionalFormatting>` and from the 2010 extension list
//! (`x14:conditionalFormatting`), which Excel uses for rules that refer to other sheets. All of
//! them are ordered by priority: where two rules set the same thing, the higher one wins, and a
//! rule with `stopIfTrue` ends the search when it applies. Nothing here changes the file; the
//! rules stay in the worksheet's markup and are saved as read.

use std::cell::OnceCell;
use std::collections::HashMap;

use office_core::drawing::{Color, Rgb};
use office_core::model::{format_number, Body, CellRef, Document, Range, Sheet, Value};
use office_core::xml::{ns, Element};

use crate::dates::DateSystem;
use crate::eval::{self, Book, V};
use crate::formula;
use crate::styles::{read_dxf, CellFormat, Dxf};
use crate::DefinedName;

/// What the rules that apply to a cell change about how it's drawn.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Look {
    /// Laid over the cell's own format.
    pub dxf: Dxf,
    pub bar: Option<Bar>,
    pub icon: Option<Icon>,
    /// A data bar or icon set that shows instead of the value.
    pub hide_value: bool,
}

/// A data bar: how far across the cell it reaches, from 0 to 1.
#[derive(Clone, Debug, PartialEq)]
pub struct Bar {
    pub len: f64,
    pub color: Color,
    pub gradient: bool,
}

/// An icon from an icon set: `index` counts up from the lowest band.
#[derive(Clone, Debug, PartialEq)]
pub struct Icon {
    pub set: String,
    pub index: usize,
    pub count: usize,
}

#[derive(Clone, Debug)]
struct Cfvo {
    kind: String,
    val: Option<String>,
    gte: bool,
}

#[derive(Clone, Debug)]
enum Scale {
    Colors(Vec<Cfvo>, Vec<Color>),
    Bar { lo: Cfvo, hi: Cfvo, color: Option<Color>, min_len: f64, max_len: f64, show_value: bool, gradient: bool },
    Icons { set: String, cfvos: Vec<Cfvo>, reverse: bool, show_value: bool },
}

#[derive(Debug)]
struct Rule {
    kind: String,
    attrs: HashMap<String, String>,
    formulas: Vec<String>,
    ranges: Vec<Range>,
    priority: i64,
    stop: bool,
    dxf: Option<Dxf>,
    scale: Option<Scale>,
    /// The numbers in the rule's ranges, sorted, for rules that look at the whole range.
    numbers: OnceCell<Vec<f64>>,
    /// How often each value appears in the rule's ranges, for duplicate and unique rules.
    counts: OnceCell<HashMap<String, usize>>,
}

impl Rule {
    fn attr(&self, k: &str) -> Option<&str> {
        self.attrs.get(k).map(String::as_str)
    }
    fn flag(&self, k: &str, default: bool) -> bool {
        self.attr(k).map_or(default, |v| v == "1" || v == "true")
    }
    fn top_left(&self) -> CellRef {
        self.ranges.first().map(|r| r.start).unwrap_or_default()
    }
}

/// A worksheet's conditional formatting rules, ready to work out cell by cell.
pub struct Rules {
    rules: Vec<Rule>,
    sheet: usize,
    now: Option<f64>,
}

fn children<'a>(el: &'a Element, local: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
    el.elements().filter(move |e| e.local == local)
}

fn child<'a>(el: &'a Element, local: &str) -> Option<&'a Element> {
    el.elements().find(|e| e.local == local)
}

fn read_cfvo(el: &Element) -> Cfvo {
    Cfvo {
        kind: el.attr("type").unwrap_or("num").to_string(),
        val: el.attr("val").map(str::to_string).or_else(|| child(el, "f").map(Element::text)),
        gte: el.attr("gte").is_none_or(|v| v != "0" && v != "false"),
    }
}

fn read_scale(el: &Element, kind: &str) -> Option<Scale> {
    let body = child(el, kind)?;
    let cfvos: Vec<Cfvo> = children(body, "cfvo").map(read_cfvo).collect();
    let on = |k: &str, default: bool| body.attr(k).map_or(default, |v| v == "1" || v == "true");
    let num = |k: &str, default: f64| body.attr(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(default);
    match kind {
        "colorScale" => {
            let colors: Vec<Color> = children(body, "color").filter_map(office_core::formats::xlsx::read_color).collect();
            (cfvos.len() >= 2 && colors.len() == cfvos.len()).then_some(Scale::Colors(cfvos, colors))
        }
        "dataBar" => {
            let color = children(body, "color").find_map(office_core::formats::xlsx::read_color);
            let mut it = cfvos.into_iter();
            let lo = it.next().unwrap_or(Cfvo { kind: "min".into(), val: None, gte: true });
            let hi = it.next().unwrap_or(Cfvo { kind: "max".into(), val: None, gte: true });
            Some(Scale::Bar {
                lo,
                hi,
                color,
                min_len: num("minLength", 10.0),
                max_len: num("maxLength", 90.0),
                show_value: on("showValue", true),
                gradient: on("gradient", true),
            })
        }
        "iconSet" => Some(Scale::Icons {
            set: body.attr("iconSet").unwrap_or("3TrafficLights1").to_string(),
            cfvos,
            reverse: on("reverse", false),
            show_value: on("showValue", true),
        }),
        _ => None,
    }
}

fn read_rule(el: &Element, sqref: &str, doc: &Document) -> Option<Rule> {
    let kind = el.attr("type")?.to_string();
    let attrs = el.attrs.iter().map(|a| (a.local.clone(), a.value.clone())).collect::<HashMap<_, _>>();
    let dxf = match el.attr("dxfId").and_then(|v| v.parse::<usize>().ok()) {
        Some(i) => crate::excel(doc).and_then(|x| x.styles.dxf(i)).cloned(),
        None => child(el, "dxf").map(read_dxf),
    };
    let formulas = el.elements().filter(|e| e.local == "formula" || e.local == "f").map(Element::text).collect();
    let ranges: Vec<Range> = sqref.split_whitespace().filter_map(|r| Range::parse(r.trim_start_matches('$'))).collect();
    if ranges.is_empty() {
        return None;
    }
    Some(Rule {
        scale: read_scale(el, &kind),
        priority: el.attr("priority").and_then(|v| v.parse().ok()).unwrap_or(i64::MAX),
        stop: el.attr("stopIfTrue").is_some_and(|v| v == "1" || v == "true"),
        kind,
        attrs,
        formulas,
        ranges,
        dxf,
        numbers: OnceCell::new(),
        counts: OnceCell::new(),
    })
}

pub(crate) fn sheets(doc: &Document) -> &[Sheet] {
    match &doc.body {
        Body::Workbook { sheets } => sheets,
        _ => &[],
    }
}

impl Rules {
    pub fn read(doc: &Document, sheet: usize, now: Option<f64>) -> Rules {
        let mut rules = Vec::new();
        let root = sheets(doc).get(sheet).and_then(|s| s.origin.get());
        if let Some(root) = root {
            // The 2010 extension can say more about a data bar the main rule already draws.
            let mut bars: HashMap<String, &Element> = HashMap::new();
            let mut ext_rules = Vec::new();
            for ext in root.child(ns::X, "extLst").into_iter().flat_map(Element::elements) {
                for list in children(ext, "conditionalFormattings") {
                    for cf in children(list, "conditionalFormatting") {
                        let sqref = child(cf, "sqref").map(Element::text).unwrap_or_default();
                        for el in children(cf, "cfRule") {
                            if let (Some("dataBar"), Some(id)) = (el.attr("type"), el.attr("id")) {
                                bars.insert(id.to_string(), el);
                            } else {
                                ext_rules.push((el, sqref.clone()));
                            }
                        }
                    }
                }
            }
            for cf in root.children_named(ns::X, "conditionalFormatting") {
                let sqref = cf.attr("sqref").unwrap_or_default();
                for el in cf.children_named(ns::X, "cfRule") {
                    let Some(mut rule) = read_rule(el, sqref, doc) else { continue };
                    let ext_id = el.descendant("http://schemas.microsoft.com/office/spreadsheetml/2009/9/main", "id").map(Element::text);
                    if let (Some(Scale::Bar { min_len, max_len, gradient, .. }), Some(ext)) =
                        (&mut rule.scale, ext_id.and_then(|id| bars.get(&id)).and_then(|e| child(e, "dataBar")))
                    {
                        let num = |k: &str, d: f64| ext.attr(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(d);
                        *min_len = num("minLength", 0.0);
                        *max_len = num("maxLength", 100.0);
                        *gradient = ext.attr("gradient").is_none_or(|v| v != "0" && v != "false");
                    }
                    rules.push(rule);
                }
            }
            rules.extend(ext_rules.into_iter().filter_map(|(el, sqref)| read_rule(el, &sqref, doc)));
        }
        rules.sort_by_key(|r| r.priority);
        Rules { rules, sheet, now }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// What the rules do to the cell at `at`, if any apply.
    pub fn look(&self, doc: &Document, at: CellRef) -> Option<Look> {
        let mut out: Option<Look> = None;
        for rule in self.rules.iter().filter(|r| r.ranges.iter().any(|g| g.contains(at))) {
            let mut book = self.book(doc);
            let applied = match &rule.scale {
                Some(scale) => self.scaled(doc, &mut book, rule, scale, at, &mut out),
                None => {
                    let hit = self.applies(doc, &mut book, rule, at);
                    if hit {
                        if let Some(d) = &rule.dxf {
                            layer(&mut out.get_or_insert_with(Look::default).dxf, d);
                        }
                    }
                    hit
                }
            };
            if applied && rule.stop {
                break;
            }
        }
        out
    }

    fn book<'a>(&self, doc: &'a Document) -> Stored<'a> {
        Stored::new(doc, self.sheet, self.now)
    }

    fn formula(&self, book: &mut Stored, rule: &Rule, i: usize, at: CellRef) -> V {
        let Some(src) = rule.formulas.get(i) else { return V::Err("#N/A") };
        let tl = rule.top_left();
        let Ok(text) = formula::shift(src, i64::from(at.row) - i64::from(tl.row), i64::from(at.col) - i64::from(tl.col)) else {
            return V::Err("#REF!");
        };
        match formula::parse(&text) {
            Ok(e) => scalar(eval::eval(&e, book)),
            Err(_) => V::Err("#NAME?"),
        }
    }

    fn applies(&self, doc: &Document, book: &mut Stored, rule: &Rule, at: CellRef) -> bool {
        let Some(sheet) = sheets(doc).get(self.sheet) else { return false };
        let value = V::from_value(sheet.value(at));
        let text = || text_of(&value).to_lowercase();
        let needle = || rule.attr("text").unwrap_or_default().to_lowercase();
        match rule.kind.as_str() {
            "expression" => truthy(&self.formula(book, rule, 0, at)),
            "cellIs" => {
                let a = self.formula(book, rule, 0, at);
                let cmp = |b: &V| compare(&value, b);
                use std::cmp::Ordering::*;
                match rule.attr("operator").unwrap_or("equal") {
                    "equal" => cmp(&a) == Some(Equal),
                    "notEqual" => matches!(cmp(&a), Some(Less | Greater)),
                    "greaterThan" => cmp(&a) == Some(Greater),
                    "lessThan" => cmp(&a) == Some(Less),
                    "greaterThanOrEqual" => matches!(cmp(&a), Some(Greater | Equal)),
                    "lessThanOrEqual" => matches!(cmp(&a), Some(Less | Equal)),
                    op @ ("between" | "notBetween") => {
                        let b = self.formula(book, rule, 1, at);
                        let (lo, hi) = if compare(&a, &b) == Some(Greater) { (b, a) } else { (a, b) };
                        let (Some(from), Some(to)) = (cmp(&lo), cmp(&hi)) else { return false };
                        let inside = from != Less && to != Greater;
                        inside == (op == "between")
                    }
                    _ => false,
                }
            }
            "containsText" => !matches!(value, V::Err(_)) && text().contains(&needle()),
            "notContainsText" => !text().contains(&needle()),
            "beginsWith" => !matches!(value, V::Err(_)) && text().starts_with(&needle()),
            "endsWith" => !matches!(value, V::Err(_)) && text().ends_with(&needle()),
            "containsBlanks" => text_of(&value).trim().is_empty(),
            "notContainsBlanks" => !text_of(&value).trim().is_empty(),
            "containsErrors" => matches!(value, V::Err(_)),
            "notContainsErrors" => !matches!(value, V::Err(_)),
            "timePeriod" => truthy(&self.formula(book, rule, 0, at)),
            "duplicateValues" | "uniqueValues" => {
                let Some(key) = key_of(sheet.value(at)) else { return false };
                let counts = rule.counts.get_or_init(|| {
                    let mut m = HashMap::new();
                    each_value(sheet, rule, |v| {
                        if let Some(k) = key_of(v) {
                            *m.entry(k).or_insert(0) += 1;
                        }
                    });
                    m
                });
                let n = counts.get(&key).copied().unwrap_or(0);
                (n > 1) == (rule.kind == "duplicateValues")
            }
            "top10" => {
                let V::Num(v) = value else { return false };
                let nums = numbers(sheet, rule);
                if nums.is_empty() {
                    return false;
                }
                let rank = rule.attr("rank").and_then(|r| r.parse::<f64>().ok()).unwrap_or(10.0);
                let n = if rule.flag("percent", false) { (nums.len() as f64 * rank / 100.0).floor().max(1.0) } else { rank.max(1.0) };
                let n = (n as usize).min(nums.len());
                if rule.flag("bottom", false) {
                    v <= nums[n - 1]
                } else {
                    v >= nums[nums.len() - n]
                }
            }
            "aboveAverage" => {
                let V::Num(v) = value else { return false };
                let nums = numbers(sheet, rule);
                if nums.is_empty() {
                    return false;
                }
                let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                let sd = (nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / nums.len() as f64).sqrt();
                let devs = rule.attr("stdDev").and_then(|d| d.parse::<f64>().ok()).unwrap_or(0.0);
                let equal = rule.flag("equalAverage", false);
                if rule.flag("aboveAverage", true) {
                    let edge = mean + devs * sd;
                    v > edge || (equal && v == edge)
                } else {
                    let edge = mean - devs * sd;
                    v < edge || (equal && v == edge)
                }
            }
            _ => false,
        }
    }

    /// Color scales, data bars and icon sets: placed by where the cell's number falls in its range.
    fn scaled(&self, doc: &Document, book: &mut Stored, rule: &Rule, scale: &Scale, at: CellRef, out: &mut Option<Look>) -> bool {
        let Some(sheet) = sheets(doc).get(self.sheet) else { return false };
        let Value::Number(v) = *sheet.value(at) else { return false };
        let nums = numbers(sheet, rule);
        let paint = |c: &Color| c.resolve(&|n| doc.theme.as_ref()?.scheme_color(n)).rgb;
        match scale {
            Scale::Colors(cfvos, colors) => {
                let ts: Option<Vec<f64>> = cfvos.iter().map(|c| self.threshold(book, rule, c, nums, at)).collect();
                let Some(ts) = ts else { return false };
                let look = out.get_or_insert_with(Look::default);
                if look.dxf.fill.is_none() {
                    let mut rgb = paint(&colors[0]);
                    if v >= ts[ts.len() - 1] {
                        rgb = paint(&colors[colors.len() - 1]);
                    } else {
                        for i in 1..ts.len() {
                            if v < ts[i] {
                                let span = ts[i] - ts[i - 1];
                                let t = if span > 0.0 { ((v - ts[i - 1]) / span).clamp(0.0, 1.0) } else { 0.0 };
                                rgb = mix(paint(&colors[i - 1]), paint(&colors[i]), t);
                                break;
                            }
                        }
                    }
                    look.dxf.fill = Some(Color::rgb(rgb));
                }
                true
            }
            Scale::Bar { lo, hi, color, min_len, max_len, show_value, gradient } => {
                let lo_t = self.threshold(book, rule, lo, nums, at);
                let hi_t = self.threshold(book, rule, hi, nums, at);
                let (Some(a), Some(b)) = (lo_t, hi_t) else { return false };
                let t = if b > a { ((v - a) / (b - a)).clamp(0.0, 1.0) } else { 1.0 };
                let look = out.get_or_insert_with(Look::default);
                if look.bar.is_none() {
                    let len = (min_len + t * (max_len - min_len)) / 100.0;
                    let color = color.clone().unwrap_or_else(|| Color::rgb(Rgb(0x63, 0x8e, 0xc6)));
                    look.bar = Some(Bar { len: len.clamp(0.0, 1.0), color, gradient: *gradient });
                    look.hide_value |= !show_value;
                }
                true
            }
            Scale::Icons { set, cfvos, reverse, show_value } => {
                if cfvos.len() < 2 {
                    return false;
                }
                let mut index = 0;
                for (i, c) in cfvos.iter().enumerate().skip(1) {
                    let Some(t) = self.threshold(book, rule, c, nums, at) else { return false };
                    if v > t || (c.gte && v == t) {
                        index = i;
                    }
                }
                let count = cfvos.len();
                if *reverse {
                    index = count - 1 - index;
                }
                let look = out.get_or_insert_with(Look::default);
                if look.icon.is_none() {
                    look.icon = Some(Icon { set: set.clone(), index, count });
                    look.hide_value |= !show_value;
                }
                true
            }
        }
    }

    fn threshold(&self, book: &mut Stored, rule: &Rule, c: &Cfvo, nums: &[f64], at: CellRef) -> Option<f64> {
        let (min, max) = (nums.first().copied()?, nums.last().copied()?);
        let mut val = || -> Option<f64> {
            let src = c.val.as_deref()?;
            if let Ok(n) = src.trim().parse::<f64>() {
                return Some(n);
            }
            let tl = rule.top_left();
            let text = formula::shift(src, i64::from(at.row) - i64::from(tl.row), i64::from(at.col) - i64::from(tl.col)).ok()?;
            match scalar(eval::eval(&formula::parse(&text).ok()?, book)) {
                V::Num(n) => Some(n),
                _ => None,
            }
        };
        Some(match c.kind.as_str() {
            "min" => min,
            "max" => max,
            "autoMin" => min.min(0.0),
            "autoMax" => max.max(0.0),
            "percent" => min + (max - min) * val()? / 100.0,
            "percentile" => percentile(nums, val()?),
            _ => val()?,
        })
    }
}

/// A cell's format with its conditional formatting laid over it.
pub fn format_at(doc: &Document, sheet: usize, at: CellRef, now: Option<f64>) -> CellFormat {
    let mut f = crate::format_of(doc, sheet, at);
    if let Some(look) = Rules::read(doc, sheet, now).look(doc, at) {
        look.dxf.apply(&mut f);
    }
    f
}

/// Sets what `d` sets and `to` doesn't yet: a higher-priority rule has already had its say.
fn layer(to: &mut Dxf, d: &Dxf) {
    macro_rules! keep {
        ($($f:ident),*) => {$( if to.$f.is_none() { to.$f = d.$f.clone(); } )*};
    }
    keep!(number_format, bold, italic, strike, underline, color, fill);
    for (to, from) in [
        (&mut to.border.left, &d.border.left),
        (&mut to.border.right, &d.border.right),
        (&mut to.border.top, &d.border.top),
        (&mut to.border.bottom, &d.border.bottom),
    ] {
        if to.style.is_none() {
            *to = from.clone();
        }
    }
}

fn each_value(sheet: &Sheet, rule: &Rule, mut f: impl FnMut(&Value)) {
    for range in &rule.ranges {
        let start = sheet.rows.partition_point(|r| r.index < range.start.row);
        for row in sheet.rows[start..].iter().take_while(|r| r.index <= range.end.row) {
            let lo = row.cells.partition_point(|c| c.col < range.start.col);
            for c in row.cells[lo..].iter().take_while(|c| c.col <= range.end.col) {
                f(&c.value);
            }
        }
    }
}

fn numbers<'a>(sheet: &Sheet, rule: &'a Rule) -> &'a [f64] {
    rule.numbers.get_or_init(|| {
        let mut out = Vec::new();
        each_value(sheet, rule, |v| {
            if let Value::Number(n) = v {
                out.push(*n);
            }
        });
        out.sort_by(f64::total_cmp);
        out
    })
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    let rank = (p / 100.0).clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let (i, frac) = (rank.floor() as usize, rank.fract());
    match sorted.get(i + 1) {
        Some(next) => sorted[i] + (next - sorted[i]) * frac,
        None => sorted[i],
    }
}

fn mix(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let m = |x: u8, y: u8| (f64::from(x) + (f64::from(y) - f64::from(x)) * t).round() as u8;
    Rgb(m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

fn key_of(v: &Value) -> Option<String> {
    match v {
        Value::Empty => None,
        Value::Number(n) => Some(format!("n{n}")),
        Value::Bool(b) => Some(format!("b{b}")),
        Value::Error(e) => Some(format!("e{e}")),
        _ => Some(format!("t{}", text_of(&V::from_value(v)).to_lowercase())),
    }
}

pub(crate) fn scalar(v: V) -> V {
    match v {
        V::Range(rows) => rows.into_iter().next().and_then(|r| r.into_iter().next()).unwrap_or(V::Empty),
        v => v,
    }
}

pub(crate) fn truthy(v: &V) -> bool {
    match v {
        V::Num(n) => *n != 0.0,
        V::Bool(b) => *b,
        _ => false,
    }
}

pub(crate) fn text_of(v: &V) -> String {
    match v {
        V::Num(n) => format_number(*n),
        V::Text(s) => s.clone(),
        V::Bool(b) => if *b { "TRUE" } else { "FALSE" }.into(),
        V::Err(e) => (*e).into(),
        _ => String::new(),
    }
}

/// Excel's comparison for cell rules: an empty cell is 0 next to a number and "" next to text,
/// text compares without case, and numbers come before text.
pub(crate) fn compare(a: &V, b: &V) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering::*;
    match (a, b) {
        (V::Err(_), _) | (_, V::Err(_)) => None,
        (V::Empty, V::Empty) => Some(Equal),
        (V::Empty, V::Num(n)) => 0.0f64.partial_cmp(n),
        (V::Num(n), V::Empty) => n.partial_cmp(&0.0),
        (V::Empty, V::Text(s)) => "".cmp(&s.to_lowercase()).into(),
        (V::Text(s), V::Empty) => s.to_lowercase().as_str().cmp("").into(),
        (V::Empty, V::Bool(b)) => false.cmp(b).into(),
        (V::Bool(b), V::Empty) => b.cmp(&false).into(),
        (V::Num(x), V::Num(y)) => x.partial_cmp(y),
        (V::Text(x), V::Text(y)) => x.to_lowercase().cmp(&y.to_lowercase()).into(),
        (V::Bool(x), V::Bool(y)) => x.cmp(y).into(),
        (V::Num(_), _) => Some(Less),
        (_, V::Num(_)) => Some(Greater),
        (V::Text(_), V::Bool(_)) => Some(Less),
        (V::Bool(_), V::Text(_)) => Some(Greater),
        _ => None,
    }
}

/// The workbook as last calculated: rules read stored results rather than working formulas out again.
pub(crate) struct Stored<'a> {
    sheets: &'a [Sheet],
    names: &'a [DefinedName],
    here: usize,
    dates: DateSystem,
    now: Option<f64>,
    /// A value to read in place of what a cell on the formula's own sheet holds.
    pub(crate) instead: Option<(CellRef, Value)>,
}

impl<'a> Stored<'a> {
    pub(crate) fn new(doc: &'a Document, sheet: usize, now: Option<f64>) -> Stored<'a> {
        let x = crate::excel(doc);
        Stored {
            sheets: sheets(doc),
            names: x.map(|x| x.names.as_slice()).unwrap_or(&[]),
            here: sheet,
            dates: x.map(|x| x.date_system).unwrap_or_default(),
            now,
            instead: None,
        }
    }

    fn index(&self, sheet: Option<&str>) -> Option<usize> {
        match sheet {
            None => Some(self.here),
            Some(n) => self.sheets.iter().position(|s| s.name.eq_ignore_ascii_case(n)),
        }
    }
}

impl Book for Stored<'_> {
    fn cell(&mut self, sheet: Option<&str>, row: u32, col: u32) -> V {
        if let Some((at, v)) = &self.instead {
            if self.index(sheet) == Some(self.here) && *at == CellRef::new(row, col) {
                return V::from_value(v);
            }
        }
        match self.index(sheet).and_then(|i| self.sheets.get(i)) {
            Some(s) => V::from_value(s.value(CellRef::new(row, col))),
            None => V::Err("#REF!"),
        }
    }
    fn extent(&self, sheet: Option<&str>) -> Option<(u32, u32)> {
        self.sheets.get(self.index(sheet)?)?.used_range().map(|u| (u.end.row, u.end.col))
    }
    fn name(&self, name: &str) -> Option<formula::Expr> {
        let found = self
            .names
            .iter()
            .find(|n| n.name.eq_ignore_ascii_case(name) && n.sheet == Some(self.here))
            .or_else(|| self.names.iter().find(|n| n.name.eq_ignore_ascii_case(name) && n.sheet.is_none()))?;
        formula::parse(&found.formula).ok()
    }
    fn date_system(&self) -> DateSystem {
        self.dates
    }
    fn now(&self) -> Option<f64> {
        self.now
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use office_core::model::{CellRef, Document, Kind};
    use office_core::opc::Package;
    use office_core::Compiler;

    use super::*;

    const X14: &str = r#"xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main""#;

    /// A two-sheet workbook with `cells` and `rules` in the first sheet, `other` in the second
    /// and `dxfs` in the style sheet.
    pub(crate) fn book(cells: &str, rules: &str, other: &str, dxfs: &str) -> Document {
        let mut doc = Document::new(Kind::Sheets);
        let Body::Workbook { sheets } = &mut doc.body else { unreachable!() };
        sheets[0].name = "Data".into();
        sheets.push(office_core::model::Sheet::new("Other"));
        let bytes = office_core::write(&doc, Kind::Sheets).unwrap();
        let mut pkg = Package::read(&bytes).unwrap();
        let edit = |pkg: &mut Package, part: &str, f: &dyn Fn(String) -> String| {
            let text = String::from_utf8(pkg.part(part).unwrap().to_vec()).unwrap();
            pkg.replace(part, f(text).into_bytes()).unwrap();
        };
        let data = |cells: &str| {
            let cells = cells.to_string();
            move |t: String| {
                t.replace("<sheetData/>", "<sheetData></sheetData>")
                    .replace("<sheetData></sheetData>", &format!("<sheetData>{cells}</sheetData>"))
            }
        };
        let first = data(cells);
        edit(&mut pkg, "xl/worksheets/sheet1.xml", &|t| {
            let t = first(t);
            let at = t.find("</sheetData>").unwrap() + "</sheetData>".len();
            format!("{}{}{}", &t[..at], rules, &t[at..])
        });
        edit(&mut pkg, "xl/worksheets/sheet2.xml", &data(other));
        edit(&mut pkg, "xl/styles.xml", &|t| t.replace("</styleSheet>", &format!("<dxfs count=\"9\">{dxfs}</dxfs></styleSheet>")));
        Compiler::new().with_layer(crate::Sheets).read(&pkg.to_bytes().unwrap(), Some("book.xlsx")).unwrap()
    }

    pub(crate) fn s(at: &str, text: &str) -> String {
        format!(r#"<c r="{at}" t="inlineStr"><is><t>{text}</t></is></c>"#)
    }

    pub(crate) fn n(at: &str, v: f64) -> String {
        format!(r#"<c r="{at}"><v>{v}</v></c>"#)
    }

    pub(crate) fn row(i: u32, cells: &[String]) -> String {
        format!(r#"<row r="{i}">{}</row>"#, cells.concat())
    }

    fn fill(doc: &Document, at: &str) -> Option<String> {
        let f = format_at(doc, 0, CellRef::parse(at).unwrap(), None);
        f.fill.fg.map(|c| c.resolve(&|_| None).css())
    }

    const GREEN: &str = r#"<dxf><fill><patternFill><bgColor rgb="FF00B050"/></patternFill></fill></dxf>"#;
    const RED: &str = r#"<dxf><fill><patternFill><bgColor rgb="FFFF0000"/></patternFill></fill></dxf>"#;

    /// CORPUS-010, LibreOffice's conditional_fmt_checkpriority.xlsx: the rules of the main list
    /// and the 2010 extension list are ordered together by priority.
    #[test]
    fn rules_from_both_lists_apply_by_priority() {
        let rules = format!(
            r#"<conditionalFormatting sqref="A1"><cfRule type="containsText" dxfId="5" priority="4" operator="containsText" text="ABC"><formula>NOT(ISERROR(SEARCH("ABC",A1)))</formula></cfRule></conditionalFormatting><conditionalFormatting sqref="A3"><cfRule type="containsText" dxfId="2" priority="1" operator="containsText" text="BAC"><formula>NOT(ISERROR(SEARCH("BAC",A3)))</formula></cfRule></conditionalFormatting><extLst><ext uri="{{78C0D931-6437-407d-A8EE-F0AAD7539E65}}" {X14}><x14:conditionalFormattings><x14:conditionalFormatting><x14:cfRule type="cellIs" priority="3" operator="equal" id="{{A}}"><xm:f>Other!$A$1</xm:f><x14:dxf><fill><patternFill><bgColor rgb="FF00B050"/></patternFill></fill></x14:dxf></x14:cfRule><xm:sqref>A1</xm:sqref></x14:conditionalFormatting><x14:conditionalFormatting><x14:cfRule type="cellIs" priority="2" operator="equal" id="{{B}}"><xm:f>Other!$A$2</xm:f><x14:dxf><fill><patternFill><bgColor rgb="FF00B050"/></patternFill></fill></x14:dxf></x14:cfRule><xm:sqref>A3</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext></extLst>"#
        );
        let cells = row(1, &[s("A1", "ABC")]) + &row(3, &[s("A3", "BAC")]);
        let other = row(1, &[s("A1", "ABC")]) + &row(2, &[s("A2", "BAC")]);
        let dxfs = [GREEN, RED, RED, GREEN, GREEN, RED].concat();
        let doc = book(&cells, &rules, &other, &dxfs);
        assert_eq!(fill(&doc, "A1").as_deref(), Some("#00b050"));
        assert_eq!(fill(&doc, "A3").as_deref(), Some("#ff0000"));
        assert_eq!(fill(&doc, "A2"), None);
    }

    #[test]
    fn cell_and_expression_rules_follow_relative_references() {
        let rules = r#"<conditionalFormatting sqref="A1:A4"><cfRule type="cellIs" dxfId="0" priority="2" operator="between"><formula>2</formula><formula>3</formula></cfRule><cfRule type="expression" dxfId="1" priority="1" stopIfTrue="1"><formula>B1="x"</formula></cfRule></conditionalFormatting>"#;
        let cells = row(1, &[n("A1", 1.0)]) + &row(2, &[n("A2", 2.0)]) + &row(3, &[n("A3", 3.0), s("B3", "x")]) + &row(4, &[n("A4", 4.0)]);
        let bold = r#"<dxf><font><b/><color rgb="FF9C0006"/></font></dxf>"#;
        let doc = book(&cells, rules, "", &[GREEN, bold].concat());
        assert_eq!(fill(&doc, "A1"), None);
        assert_eq!(fill(&doc, "A2").as_deref(), Some("#00b050"));
        // The expression rule wins on A3 and stops the search, so the fill doesn't apply.
        let a3 = format_at(&doc, 0, CellRef::parse("A3").unwrap(), None);
        assert!(a3.font.bold);
        assert_eq!(fill(&doc, "A3"), None);
        assert_eq!(fill(&doc, "A4"), None);
    }

    #[test]
    fn scales_bars_and_icons_place_values_in_their_range() {
        let rules = r#"<conditionalFormatting sqref="A1:A3"><cfRule type="colorScale" priority="1"><colorScale><cfvo type="min"/><cfvo type="max"/><color rgb="FF000000"/><color rgb="FFFFFFFF"/></colorScale></cfRule></conditionalFormatting><conditionalFormatting sqref="B1:B3"><cfRule type="dataBar" priority="2"><dataBar showValue="0"><cfvo type="min"/><cfvo type="max"/><color rgb="FF638EC6"/></dataBar></cfRule></conditionalFormatting><conditionalFormatting sqref="C1:C3"><cfRule type="iconSet" priority="3"><iconSet iconSet="3Arrows"><cfvo type="percent" val="0"/><cfvo type="percent" val="33"/><cfvo type="percent" val="67"/></iconSet></cfRule></conditionalFormatting><conditionalFormatting sqref="D1:D4"><cfRule type="duplicateValues" dxfId="0" priority="4"/><cfRule type="top10" dxfId="1" priority="5" rank="1"/></conditionalFormatting>"#;
        let cells = (1..=3)
            .map(|i| {
                row(
                    i,
                    &[
                        n(&format!("A{i}"), i as f64),
                        n(&format!("B{i}"), i as f64),
                        n(&format!("C{i}"), i as f64),
                        n(&format!("D{i}"), if i == 3 { 1.0 } else { i as f64 }),
                    ],
                )
            })
            .collect::<String>()
            + &row(4, &[n("D4", 9.0)]);
        let doc = book(&cells, rules, "", &[GREEN, RED].concat());
        assert_eq!(fill(&doc, "A1").as_deref(), Some("#000000"));
        assert_eq!(fill(&doc, "A2").as_deref(), Some("#808080"));
        assert_eq!(fill(&doc, "A3").as_deref(), Some("#ffffff"));
        let rules_of = Rules::read(&doc, 0, None);
        let look = |at: &str| rules_of.look(&doc, CellRef::parse(at).unwrap()).unwrap();
        let b1 = look("B1");
        assert_eq!(b1.bar.as_ref().map(|b| (b.len * 100.0).round()), Some(10.0));
        assert_eq!(look("B3").bar.map(|b| (b.len * 100.0).round()), Some(90.0));
        assert!(b1.hide_value);
        assert_eq!(look("C1").icon.map(|i| i.index), Some(0));
        assert_eq!(look("C2").icon.map(|i| i.index), Some(1));
        assert_eq!(look("C3").icon.map(|i| (i.index, i.count)), Some((2, 3)));
        assert_eq!(fill(&doc, "D1").as_deref(), Some("#00b050"));
        assert_eq!(fill(&doc, "D2"), None);
        assert_eq!(fill(&doc, "D4").as_deref(), Some("#ff0000"));
    }
}
