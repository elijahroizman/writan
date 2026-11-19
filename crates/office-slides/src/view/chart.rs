//! Read-only previews of chart parts kept opaque by the PowerPoint writer. The caches in the
//! chart part are what PowerPoint last displayed; we don't open the embedded workbook or links.

use std::f64::consts::{PI, TAU};
use std::fmt::Write;

use office_core::drawing::{Color, Rgba};
use office_core::formats::xlsx::chart::read_chart;
use office_core::model::{Chart, ChartGrouping, ChartKind, ChartSeries, Document, LegendPosition, RunProps, Slide};
use office_core::xml::{ns, Element};
use office_engine::display::{r, Item};
use office_engine::para::{Ctx, StyleId};
use office_engine::style::TextStyle;

use super::draw::Boxed;
use super::text::Paint;

const MAX_POINTS: usize = 4096;

pub(super) fn draw(ctx: &mut Ctx, doc: &Document, slide: &Slide, part: &str, frame: &Element, b: Boxed) -> Option<Item> {
    if !frame.is(ns::P, "graphicFrame") || b.w < 16.0 || b.h < 16.0 {
        return None;
    }
    let data = frame.path(ns::A, &["graphic", "graphicData"])?;
    let rid = data.child(ns::C, "chart")?.attr_ns(ns::R, "id")?;
    let package = doc.package.as_ref()?;
    let rels = package.rels(part).ok()?;
    let rel = rels.get(rid).filter(|r| r.is("chart"))?;
    let xml = package.xml(&package.target(part, rel)?).ok()?;
    let root = &xml.root;
    let plot = root.path(ns::C, &["chart", "plotArea"])?;
    let mut groups = plot.elements().filter(|e| e.in_ns(ns::C) && e.local.ends_with("Chart"));
    let group = groups.next()?;
    // Don't pass off a combination, 3D or a chart with unhandled grouping as a different chart.
    if groups.next().is_some()
        || !matches!(group.local.as_str(), "pieChart" | "doughnutChart" | "barChart" | "lineChart" | "areaChart" | "scatterChart")
        || group.children_named(ns::C, "ser").count() > 64
        || !bounded_cache(root)
    {
        return None;
    }
    let chart = read_chart(root)?;
    let radial = !chart.kind.has_axes();
    if chart.grouping != ChartGrouping::Standard
        || chart.series.iter().map(|s| s.cache.values.len()).sum::<usize>() > MAX_POINTS
        || !chart.series.iter().flat_map(|s| &s.cache.values).flatten().any(|v| v.is_finite())
        || (chart.kind == ChartKind::Pie && chart.series.len() != 1)
    {
        return None;
    }

    let map = root.child(ns::C, "clrMapOvr");
    let color = |c: &Color| crate::color_with_override(doc, slide, c, map);
    let series = SeriesColors::new(group, &color, slide_theme(doc, slide), radial);
    let mut items = Vec::new();
    if let Some(fill) = solid(root.child(ns::C, "spPr")).map(|c| color(&c).css()) {
        items.push(Item::Rect { x: 0.0, y: 0.0, w: r(b.w), h: r(b.h), fill: Some(fill) });
    }

    let font = Paint { doc, slide }.body_font();
    let padding = (b.w.min(b.h) * 0.045).clamp(5.0, 18.0);
    let label_size = (b.w.min(b.h) * 0.045).clamp(8.0, 13.0);
    let title_size = label_size * 1.3;
    let text_color = Color::scheme("tx1");
    let mut bounds = Boxed { x: padding, y: padding, w: b.w - 2.0 * padding, h: b.h - 2.0 * padding, ..Default::default() };
    let props = RunProps { font: Some(font), color: Some(text_color.clone()), ..Default::default() };
    let labels = ctx.styles.add(ctx.measure, TextStyle::from_props(&props, label_size, &color));

    if let Some(title) = chart.title.as_deref().filter(|t| !t.is_empty()).filter(|_| bounds.h > title_size * 3.0) {
        let style = ctx.styles.add(ctx.measure, TextStyle::from_props(&props, title_size, &color));
        if let Some(item) = label(ctx, style, title, bounds.w) {
            items.push(place(item, b.w / 2.0, bounds.y + title_size, 0.5));
            bounds.y += title_size * 1.5;
            bounds.h -= title_size * 1.5;
        }
    }
    if let Some(pos) = chart.legend {
        legend(ctx, labels, &chart, &series, pos, &mut bounds, &mut items);
    }
    if radial {
        rings(&chart, group, &series, bounds, &mut items)?;
    } else {
        axes(ctx, labels, &chart, &series, &color(&text_color).css(), bounds, &mut items)?;
    }

    Some(Item::Group { x: 0.0, y: 0.0, w: r(b.w), h: r(b.h), rot: 0.0, fh: false, fv: false, clip: true, id: None, items })
}

fn bounded_cache(el: &Element) -> bool {
    let index = if el.is(ns::C, "pt") {
        el.attr("idx").map(|v| (v, MAX_POINTS - 1))
    } else if el.is(ns::C, "ptCount") {
        el.attr("val").map(|v| (v, MAX_POINTS))
    } else {
        None
    };
    index.is_none_or(|(v, max)| v.parse::<usize>().is_ok_and(|i| i <= max)) && el.elements().all(bounded_cache)
}

fn val<'a>(el: &'a Element, name: &str) -> Option<&'a str> {
    el.child(ns::C, name)?.attr("val")
}

fn solid(props: Option<&Element>) -> Option<Color> {
    Color::in_drawingml(props?.child(ns::A, "solidFill")?)
}

fn slide_theme<'a>(doc: &'a Document, slide: &Slide) -> Option<&'a office_core::drawing::Theme> {
    let layout = crate::template(doc, slide.layout.as_deref());
    crate::template(doc, layout.and_then(|l| l.master.as_deref())).and_then(|m| m.theme.as_ref()).or(doc.theme.as_ref())
}

struct SeriesColors {
    series: Vec<Option<String>>,
    points: Vec<std::collections::HashMap<usize, String>>,
    palette: [String; 6],
    vary: bool,
}

impl SeriesColors {
    fn new(group: &Element, color: &impl Fn(&Color) -> Rgba, theme: Option<&office_core::drawing::Theme>, radial: bool) -> Self {
        let fallback = [0x4472C4, 0xED7D31, 0xA5A5A5, 0xFFC000, 0x5B9BD5, 0x70AD47];
        let palette = std::array::from_fn(|i| {
            let key = format!("accent{}", i + 1);
            if theme.is_some_and(|t| t.scheme_color(&key).is_some()) {
                color(&Color::scheme(&key)).css()
            } else {
                format!("#{:06X}", fallback[i])
            }
        });
        let mut style: Vec<_> = group.children_named(ns::C, "ser").enumerate().collect();
        style.sort_by_key(|&(i, s)| val(s, "order").and_then(|v| v.parse::<u32>().ok()).unwrap_or(i as u32));
        let (series, points) = style
            .into_iter()
            .map(|(_, s)| {
                let props = s.child(ns::C, "spPr");
                let series = solid(props).or_else(|| solid(props?.child(ns::A, "ln"))).map(|c| color(&c).css());
                let points = s
                    .children_named(ns::C, "dPt")
                    .filter_map(|p| Some((val(p, "idx")?.parse().ok()?, color(&solid(p.child(ns::C, "spPr"))?).css())))
                    .collect();
                (series, points)
            })
            .unzip();
        let vary = val(group, "varyColors").and_then(office_core::units::parse_bool).unwrap_or(radial);
        Self { series, points, palette, vary }
    }

    fn at(&self, series: usize, point: Option<usize>) -> String {
        point
            .and_then(|p| self.points[series].get(&p))
            .or(self.series[series].as_ref())
            .unwrap_or(&self.palette[if self.vary { point.unwrap_or(series) } else { series } % self.palette.len()])
            .clone()
    }
}

fn label(ctx: &mut Ctx, style: StyleId, text: &str, limit: f64) -> Option<Item> {
    let text = text.trim();
    if text.is_empty() || text.len() > 512 {
        return None;
    }
    let w = ctx.measure.width(ctx.host, ctx.styles.font(style), text);
    (w <= limit).then(|| Item::Text { x: 0.0, y: 0.0, w: r(w), s: style.0, text: text.into(), ws: 0.0, link: None, rtl: false })
}

fn place(mut item: Item, x0: f64, y0: f64, align: f64) -> Item {
    if let Item::Text { x, y, w, .. } = &mut item {
        *x = r(x0 - *w * align);
        *y = r(y0);
    }
    item
}

fn legend(ctx: &mut Ctx, style: StyleId, chart: &Chart, colors: &SeriesColors, pos: LegendPosition, b: &mut Boxed, out: &mut Vec<Item>) {
    let swatch = ctx.styles.get(style).full_px * 0.77;
    let step = swatch * 2.0;
    let vertical = matches!(pos, LegendPosition::Left | LegendPosition::Right);
    let max_w = if vertical { b.w * 0.35 } else { b.w };
    let entries: Vec<(String, String)> = if chart.kind.has_axes() {
        chart
            .series
            .iter()
            .enumerate()
            .map(|(i, s)| {
                (s.cache.name.clone().or_else(|| s.name.clone()).unwrap_or_else(|| format!("Series {}", i + 1)), colors.at(i, None))
            })
            .collect()
    } else {
        chart.series[0].cache.categories.iter().enumerate().map(|(i, s)| (s.clone(), colors.at(0, Some(i)))).collect()
    };
    let mut x = 0.0;
    let mut y = swatch;
    let mut max = 0.0_f64;
    let mut items = Vec::new();
    for (name, fill) in entries {
        let Some(item @ Item::Text { w, .. }) = label(ctx, style, &name, max_w - step) else { continue };
        if x + w + step > max_w || y + swatch > b.h * if vertical { 1.0 } else { 0.25 } {
            break;
        }
        items.push(Item::Rect { x: r(x), y: r(y - swatch), w: r(swatch), h: r(swatch), fill: Some(fill) });
        items.push(place(item, x + step, y, 0.0));
        max = max.max(x + w + step);
        if vertical {
            y += step;
        } else {
            x += w + step + swatch;
        }
    }
    if items.is_empty() {
        return;
    }
    let w = max + swatch;
    let h = if vertical { y } else { step };
    let (x, y) = match pos {
        LegendPosition::Left => (b.x, b.y + (b.h - h) / 2.0),
        LegendPosition::Right => (b.x + b.w - w, b.y + (b.h - h) / 2.0),
        LegendPosition::Top => (b.x + (b.w - w) / 2.0, b.y),
        LegendPosition::Bottom => (b.x + (b.w - w) / 2.0, b.y + b.h - h),
    };
    out.push(Item::Group { x: r(x), y: r(y), w: r(w), h: r(h), rot: 0.0, fh: false, fv: false, clip: false, id: None, items });
    if vertical {
        b.w -= w;
        if pos == LegendPosition::Left {
            b.x += w;
        }
    } else {
        b.h -= h;
        if pos == LegendPosition::Top {
            b.y += h;
        }
    }
}

fn rings(chart: &Chart, group: &Element, colors: &SeriesColors, b: Boxed, out: &mut Vec<Item>) -> Option<()> {
    let radius = b.w.min(b.h) / 2.0;
    let hole = if chart.kind == ChartKind::Doughnut {
        f64::from(val(group, "holeSize").and_then(|v| v.parse::<u32>().ok()).unwrap_or(50).clamp(10, 90)) / 100.0
    } else {
        0.0
    };
    let thick = radius * (1.0 - hole) / chart.series.len() as f64;
    let start = f64::from(val(group, "firstSliceAng").and_then(|v| v.parse::<i32>().ok()).unwrap_or(0) % 360).to_radians() - PI / 2.0;
    let previous = out.len();
    for (i, s) in chart.series.iter().enumerate() {
        let max = s.cache.values.iter().flatten().filter(|v| v.is_finite()).map(|v| v.abs()).fold(0.0, f64::max);
        if max == 0.0 {
            continue;
        }
        let shares = s.cache.values.iter().map(|v| v.filter(|v| v.is_finite()).map_or(0.0, |v| v.abs() / max));
        let sum = shares.clone().sum::<f64>();
        let inner = radius * hole + thick * i as f64;
        let mut a = start;
        for (j, share) in shares.enumerate().filter(|(_, v)| *v > 0.0) {
            let end = a + share / sum * TAU;
            let mut d = String::new();
            arc(&mut d, 'M', inner + thick, a, end);
            if inner == 0.0 {
                d.push_str("L0 0");
            } else {
                arc(&mut d, 'L', inner, end, a);
            }
            d.push('Z');
            out.push(Item::Path {
                x: r(b.x + b.w / 2.0),
                y: r(b.y + b.h / 2.0),
                d,
                fill: Some(colors.at(i, Some(j))),
                stroke: None,
                sw: 0.0,
                dash: "solid",
            });
            a = end;
        }
    }
    (out.len() > previous).then_some(())
}

fn arc(d: &mut String, command: char, radius: f64, start: f64, end: f64) {
    let xy = |a: f64| (r(radius * a.cos()), r(radius * a.sin()));
    let (x, y) = xy(start);
    let _ = write!(d, "{command}{x} {y}");
    // An SVG arc cannot start and end at the same point. Split a full circle at its midpoint.
    let delta = end - start;
    let sweep = u8::from(delta > 0.0);
    let radius = r(radius);
    if delta.abs() >= TAU - 1e-9 {
        let (x, y) = xy(start + delta / 2.0);
        let _ = write!(d, "A{radius} {radius} 0 0 {sweep} {x} {y}");
    }
    let (x, y) = xy(end);
    let large = u8::from(delta.abs() > PI && delta.abs() < TAU - 1e-9);
    let _ = write!(d, "A{radius} {radius} 0 {large} {sweep} {x} {y}");
}

fn scatter_x(series: &ChartSeries, i: usize) -> Option<f64> {
    if series.categories.is_none() && series.cache.categories.is_empty() {
        return Some((i + 1) as f64);
    }
    series.cache.categories.get(i)?.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

fn axes(ctx: &mut Ctx, style: StyleId, chart: &Chart, colors: &SeriesColors, ink: &str, mut b: Boxed, out: &mut Vec<Item>) -> Option<()> {
    let values = chart.series.iter().flat_map(|s| &s.cache.values).flatten().copied().filter(|v| v.is_finite());
    let scale = values.clone().map(f64::abs).fold(0.0, f64::max);
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let (low, high) = values.fold((0.0_f64, 0.0_f64), |(lo, hi), v| (lo.min(v / scale), hi.max(v / scale)));
    let high = if low == high { 1.0 } else { high };
    let px = ctx.styles.get(style).full_px;
    let bar = chart.kind == ChartKind::Bar;
    let scatter = chart.kind == ChartKind::Scatter;
    let n = chart.series.iter().map(|s| s.cache.values.len()).max()?;
    let categories = &chart.series.iter().max_by_key(|s| s.cache.categories.len())?.cache.categories;
    let ticks: Vec<f64> = (0..=4).map(|i| low + (high - low) * f64::from(i) / 4.0).collect();
    let tick_labels: Vec<_> = ticks.iter().map(|t| label(ctx, style, &tick_text(t * scale), b.w / 4.0)).collect();
    let margin = if bar {
        px * 4.0
    } else {
        tick_labels
            .iter()
            .filter_map(|item| match item {
                Some(Item::Text { w, .. }) => Some(*w),
                _ => None,
            })
            .fold(0.0, f64::max)
            + px / 2.0
    }
    .min(b.w / 4.0);
    b.x += margin;
    b.w -= margin;
    b.h -= px * 1.4;
    if b.w <= 0.0 || b.h <= 0.0 {
        return None;
    }
    let scaled = |v: f64| (v - low) / (high - low);
    let xy = |category: f64, value: f64| {
        if bar {
            (b.x + b.w * scaled(value), b.y + b.h * category)
        } else {
            (b.x + b.w * category, b.y + b.h * (1.0 - scaled(value)))
        }
    };
    let xs = chart.series.iter().flat_map(|s| {
        s.cache.values.iter().enumerate().filter_map(|(i, y)| if scatter && y.is_some_and(f64::is_finite) { scatter_x(s, i) } else { None })
    });
    let x_scale = xs.clone().map(f64::abs).fold(0.0, f64::max);
    let x_scale = if x_scale > 0.0 { x_scale } else { 1.0 };
    let x_bounds = xs.fold(None, |a: Option<(f64, f64)>, v| {
        let v = v / x_scale;
        Some(a.map_or((v, v), |(lo, hi)| (lo.min(v), hi.max(v))))
    });
    let scatter_position = |s: &ChartSeries, i: usize| {
        let (lo, hi) = x_bounds?;
        let x = scatter_x(s, i)? / x_scale;
        Some(if lo == hi { 0.5 } else { (x - lo) / (hi - lo) })
    };
    let before = out.len();
    let columns = matches!(chart.kind, ChartKind::Column | ChartKind::Bar);
    let bands = (if columns { 0.76 } else { 0.0 }) / n as f64 / chart.series.len() as f64;
    for (i, s) in chart.series.iter().enumerate() {
        let mut run = Vec::new();
        let c = colors.at(i, None);
        for (j, value) in s.cache.values.iter().chain(std::iter::once(&None)).enumerate() {
            let category = if scatter { scatter_position(s, j) } else { Some((j as f64 + 0.5) / n as f64) };
            let Some((cat, v)) = category.zip(value.filter(|v| v.is_finite()).map(|v| v / scale)) else {
                if !run.is_empty() {
                    plot_run(chart.kind, &c, xy(0.0, 0.0).1, &run, out);
                    run.clear();
                }
                continue;
            };
            let (x, y) = xy(cat, v);
            if columns {
                let offset = (i as f64 - chart.series.len() as f64 / 2.0) * bands;
                let a = xy(cat + offset, v);
                let z = xy(cat + offset + bands, 0.0);
                out.push(Item::Rect {
                    x: r(a.0.min(z.0)),
                    y: r(a.1.min(z.1)),
                    w: r((a.0 - z.0).abs()),
                    h: r((a.1 - z.1).abs()),
                    fill: Some(colors.at(i, Some(j))),
                });
            } else if scatter {
                dot(x, y, colors.at(i, Some(j)), out);
            } else {
                run.push((x, y));
            }
        }
    }
    if out.len() == before {
        return None;
    }
    for (t, text) in ticks.into_iter().zip(tick_labels) {
        if let Some(item) = text {
            let (x, y) = xy(0.0, t);
            out.push(if bar { place(item, x, b.y + b.h + px, 0.5) } else { place(item, x - px / 3.0, y + px * 0.3, 1.0) });
        }
    }
    for (a, z) in [(xy(0.0, low), xy(0.0, high)), (xy(0.0, 0.0), xy(1.0, 0.0))] {
        out.push(Item::Line { x1: r(a.0), y1: r(a.1), x2: r(z.0), y2: r(z.1), color: ink.into(), w: 0.65, dash: "solid" });
    }
    if scatter {
        if let Some((lo, hi)) = x_bounds {
            for (x, t) in [(b.x, lo), (b.x + b.w, hi)] {
                if let Some(item) = label(ctx, style, &tick_text(t * x_scale), b.w / 4.0) {
                    out.push(place(item, x, b.y + b.h + px, if x == b.x { 0.0 } else { 1.0 }));
                }
            }
        }
    } else {
        let space = (if bar { b.h } else { b.w }) / n as f64;
        if space >= px * 1.2 {
            for (i, cat) in categories.iter().take(n).enumerate() {
                if let Some(item) = label(ctx, style, cat, if bar { margin - px / 3.0 } else { space - 2.0 }) {
                    let (x, y) = xy((i as f64 + 0.5) / n as f64, low);
                    out.push(if bar { place(item, x - px / 3.0, y + px * 0.3, 1.0) } else { place(item, x, y + px, 0.5) });
                }
            }
        }
    }
    Some(())
}

fn tick_text(v: f64) -> String {
    if v != 0.0 && !(0.01..100_000.0).contains(&v.abs()) {
        format!("{v:.1e}")
    } else {
        format!("{v:.2}").trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn dot(x: f64, y: f64, color: String, out: &mut Vec<Item>) {
    out.push(Item::Path {
        x: r(x),
        y: r(y),
        d: "M-2.5 0a2.5 2.5 0 1 0 5 0a2.5 2.5 0 1 0 -5 0Z".into(),
        fill: Some(color),
        stroke: None,
        sw: 0.0,
        dash: "solid",
    });
}

fn plot_run(kind: ChartKind, color: &str, baseline: f64, points: &[(f64, f64)], out: &mut Vec<Item>) {
    let (first_x, first_y) = points[0];
    if points.len() == 1 {
        dot(first_x, first_y, color.into(), out);
        return;
    }
    let mut d = format!("M{} {}", r(first_x), r(first_y));
    for &(x, y) in &points[1..] {
        let _ = write!(d, "L{} {}", r(x), r(y));
    }
    let area = kind == ChartKind::Area;
    if area {
        let _ = write!(d, "V{}H{}Z", r(baseline), r(first_x));
    }
    out.push(Item::Path {
        x: 0.0,
        y: 0.0,
        d,
        fill: area.then(|| color.into()),
        stroke: (!area).then(|| color.into()),
        sw: 1.5,
        dash: "solid",
    });
}
