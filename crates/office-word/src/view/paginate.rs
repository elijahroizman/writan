//! Cutting each section's flow into pages: lines and table rows fill a page's body, and a page
//! ends early for a page break, to keep a heading with what follows it, or to avoid a
//! paragraph's first or last line alone on a page (widow and orphan control).

use office_core::model::Document;
use office_engine::flow::{BlockBox, FlowLayout, Keep};
use office_engine::measure::Measurer;
use office_engine::para::LineEnd;

use super::resolve::emu_px;
use super::{Page, WordView};
use crate::sections::Section;
use crate::{word, StoryKind};

/// Something a page can't be cut inside: a line of a paragraph, or a table row.
#[derive(Clone, Debug)]
struct Unit {
    top: f64,
    bottom: f64,
    /// Its line in the paragraph (rows count as line 0 of a one-line paragraph).
    line: usize,
    lines: usize,
    keep: Keep,
    /// A page break ends this line.
    break_after: bool,
}

fn flow_units(flow: &FlowLayout, y: f64, out: &mut Vec<Unit>) {
    for block in &flow.blocks {
        match block {
            BlockBox::Para(p) => {
                let n = p.layout.lines.len();
                for (i, l) in p.layout.lines.iter().enumerate() {
                    out.push(Unit {
                        top: y + p.y + l.top,
                        bottom: y + p.y + l.top + l.height,
                        line: i,
                        lines: n,
                        keep: p.keep,
                        break_after: l.ends == LineEnd::PageBreak,
                    });
                }
            }
            BlockBox::Table(t) => {
                for r in &t.rows {
                    out.push(Unit {
                        top: y + t.y + r.y,
                        bottom: y + t.y + r.y + r.height,
                        line: 0,
                        lines: 1,
                        keep: Keep::default(),
                        break_after: false,
                    });
                }
            }
            BlockBox::Preview { y: top, flow, .. } => flow_units(flow, y + top, out),
            BlockBox::Empty { .. } => {}
        }
    }
}

/// Where to start a new page, wanting to start it at unit `i` with the page starting at unit
/// `first`: moved back to keep lines and headings together, never back to `first` itself.
fn adjust(units: &[Unit], i: usize, first: usize) -> usize {
    let mut cut = i;
    let u = &units[cut];
    if u.keep.together && u.line > 0 {
        cut -= u.line;
    } else if u.keep.widows && u.lines >= 2 {
        if u.line == 1 {
            // An orphan: the paragraph's first line alone at the bottom of the page.
            cut -= 1;
        } else if u.line + 1 == u.lines && u.lines >= 3 && u.line >= 2 {
            // A widow: its last line alone at the top of the next.
            cut -= 1;
        }
    }
    // Headings stay with the paragraph after them.
    let mut guard = 0;
    while cut > first && units[cut].line == 0 && guard < 16 {
        let prev = &units[cut - 1];
        if !prev.keep.with_next {
            break;
        }
        cut -= prev.line + 1;
        guard += 1;
    }
    if cut <= first {
        i
    } else {
        cut
    }
}

/// Header and footer story indexes for each kind a section has: (`default`, `first`, `even`).
fn section_stories(doc: &Document, section: &Section, kind: StoryKind) -> Vec<(String, usize)> {
    let Some(w) = word(doc) else { return Vec::new() };
    let Some(pkg) = &doc.package else { return Vec::new() };
    let Ok(main) = pkg.main_part() else { return Vec::new() };
    let Ok(rels) = pkg.rels(&main) else { return Vec::new() };
    let refs = if kind == StoryKind::Header { &section.headers } else { &section.footers };
    refs.iter()
        .filter_map(|(ty, rid)| {
            let rel = rels.get(rid)?;
            let part = pkg.target(&main, rel)?;
            let i = w.headers.iter().position(|s| s.part == part && s.kind == kind)?;
            Some((ty.clone(), i))
        })
        .collect()
}

/// The height a header or footer takes, laid out at width `w`.
fn story_height(view: &mut WordView, host: &mut dyn Measurer, story: Option<usize>, w: f64) -> f64 {
    let Some(i) = story else { return 0.0 };
    let blocks = word(&view.doc).and_then(|wd| wd.headers.get(i)).map(|s| s.blocks.clone()).unwrap_or_default();
    let labels = Default::default();
    let mut ctx = office_engine::para::Ctx { host, measure: &mut view.measure, styles: &mut view.styles };
    let mut r = super::resolve::WordResolver {
        doc: &view.doc,
        extras: &view.extras,
        story: &blocks,
        labels: &labels,
        fields: Some(super::resolve::PageFields { page: 1, pages: 1 }),
        specs: &mut view.specs,
    };
    office_engine::flow::layout(&mut ctx, &mut view.cache, &mut r, &blocks, &[], w).height
}

pub(super) fn paginate(view: &mut WordView, host: &mut dyn Measurer) -> Vec<Page> {
    let mut heights: std::collections::HashMap<(Option<usize>, u64), f64> = Default::default();
    let mut story_height = |view: &mut WordView, host: &mut dyn Measurer, story: Option<usize>, w: f64| -> f64 {
        *heights.entry((story, w.to_bits())).or_insert_with(|| story_height(view, host, story, w))
    };
    let mut pages: Vec<Page> = Vec::new();
    let mut inherited: [Vec<(String, usize)>; 2] = [Vec::new(), Vec::new()];
    for si in 0..view.sections.len() {
        let section = view.sections[si].section.clone();
        let (w, h) = (emu_px(section.page_width), emu_px(section.page_height));
        // A section without its own headers uses the previous section's (Word's "link to previous").
        let mut stories =
            [section_stories(&view.doc, &section, StoryKind::Header), section_stories(&view.doc, &section, StoryKind::Footer)];
        for k in 0..2 {
            for (ty, i) in inherited[k].clone() {
                if !stories[k].iter().any(|(t, _)| *t == ty) {
                    stories[k].push((ty, i));
                }
            }
            inherited[k] = stories[k].clone();
        }
        let pick = |list: &[(String, usize)], first: bool, even: bool| -> Option<usize> {
            let find = |t: &str| list.iter().find(|(ty, _)| ty == t).map(|(_, i)| *i);
            if first && section.title_page {
                return find("first");
            }
            if even {
                if let Some(i) = find("even") {
                    return Some(i);
                }
            }
            find("default")
        };
        let text_w = emu_px(section.text_width()).max(24.0);
        let mut units = Vec::new();
        flow_units(&view.sections[si].flow, 0.0, &mut units);
        let flow_height = view.sections[si].flow.height;
        let mut page_top = units.first().map(|u| u.top).unwrap_or(0.0);
        let mut first_unit = 0;
        let mut i = 0;
        let mut index_in_section = 0;
        // The body's height on a page of this section (headers and footers measured on the
        // first page; they're the same height on nearly every page).
        let first_header = pick(&stories[0], true, false);
        let first_footer = pick(&stories[1], true, false);
        let hh = story_height(view, host, first_header, text_w);
        let fh = story_height(view, host, first_footer, text_w);
        let top_margin = emu_px(section.margin_top).max(if hh > 0.0 { emu_px(section.header) + hh } else { 0.0 });
        let bottom_margin = emu_px(section.margin_bottom).max(if fh > 0.0 { emu_px(section.footer) + fh } else { 0.0 });
        let avail = (h - top_margin - bottom_margin).max(24.0);
        let mut make_page = |view: &mut WordView, host: &mut dyn Measurer, top: f64, bottom: f64, n: usize, pages: &mut Vec<Page>| {
            let number = pages.len() as u32 + 1;
            let header = pick(&stories[0], n == 0, number.is_multiple_of(2));
            let footer = pick(&stories[1], n == 0, number.is_multiple_of(2));
            let hh = story_height(view, host, header, text_w);
            let body_y = emu_px(section.margin_top).max(if hh > 0.0 { emu_px(section.header) + hh } else { 0.0 });
            pages.push(Page {
                section: si,
                number,
                width: w,
                height: h,
                top,
                bottom,
                body_x: emu_px(section.margin_left),
                body_y,
                header,
                footer,
            });
        };
        while i < units.len() {
            let u = &units[i];
            let starts_page = i == first_unit;
            if !starts_page && u.line == 0 && u.keep.page_break_before {
                make_page(view, host, page_top, u.top, index_in_section, &mut pages);
                index_in_section += 1;
                page_top = u.top;
                first_unit = i;
                continue;
            }
            if !starts_page && u.bottom - page_top > avail + 0.01 {
                let cut = adjust(&units, i, first_unit);
                make_page(view, host, page_top, units[cut].top, index_in_section, &mut pages);
                index_in_section += 1;
                page_top = units[cut].top;
                first_unit = cut;
                i = cut;
                continue;
            }
            if u.break_after {
                let next_top = units.get(i + 1).map(|n| n.top).unwrap_or(u.bottom);
                make_page(view, host, page_top, next_top, index_in_section, &mut pages);
                index_in_section += 1;
                page_top = next_top;
                first_unit = i + 1;
            }
            i += 1;
        }
        if index_in_section == 0 || page_top < flow_height - 0.01 || units.last().is_some_and(|u| u.break_after) {
            make_page(view, host, page_top, flow_height.max(page_top), index_in_section, &mut pages);
        }
    }
    if pages.is_empty() {
        let s = Section::default();
        pages.push(Page {
            section: 0,
            number: 1,
            width: emu_px(s.page_width),
            height: emu_px(s.page_height),
            top: 0.0,
            bottom: 0.0,
            body_x: emu_px(s.margin_left),
            body_y: emu_px(s.margin_top),
            header: None,
            footer: None,
        });
    }
    pages
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(block: usize, line: usize, lines: usize, keep: Keep) -> Unit {
        let top = (block * 10 + line) as f64 * 10.0;
        Unit { top, bottom: top + 10.0, line, lines, keep, break_after: false }
    }

    #[test]
    fn widows_and_orphans() {
        let keep = Keep { widows: true, ..Default::default() };
        let units: Vec<Unit> = (0..5).map(|l| unit(0, l, 5, keep)).collect();
        // Cutting before line 1 would orphan line 0: the whole paragraph moves (unless it starts
        // the page, where nothing can be done).
        assert_eq!(adjust(&units, 1, 0), 1);
        let mut with_prev = vec![unit(0, 0, 1, Keep::default())];
        with_prev.extend((0..5).map(|l| unit(1, l, 5, keep)));
        assert_eq!(adjust(&with_prev, 2, 0), 1);
        // Cutting before the last line would widow it: one more line goes over.
        assert_eq!(adjust(&with_prev, 5, 0), 4);
    }

    #[test]
    fn headings_stay_with_the_next_paragraph() {
        let heading = Keep { with_next: true, ..Default::default() };
        let units =
            vec![unit(0, 0, 1, Keep::default()), unit(1, 0, 1, heading), unit(2, 0, 3, Keep::default()), unit(2, 1, 3, Keep::default())];
        assert_eq!(adjust(&units, 2, 0), 1);
    }
}
