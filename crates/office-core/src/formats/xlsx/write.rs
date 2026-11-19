use std::collections::{HashMap, HashSet};

use super::props::escape;
use crate::drawing::Theme;
use crate::formats::common::{names_parts, origin_as, put_if_changed, rel_attr, Out, PartCtx};
use crate::model::{format_number, Baseline, Body, CellRef, Document, Kind, Range, Run, Sheet, SheetCell, Value};
use crate::opc::content_types::types;
use crate::opc::{rels::kind, Package};
use crate::xml::{ns, Element, Node, XmlDoc};
use crate::{Error, Result};

/// The children of `worksheet`, in schema order.
const WORKSHEET: &[&[&str]] = &[
    &["sheetPr"],
    &["dimension"],
    &["sheetViews"],
    &["sheetFormatPr"],
    &["cols"],
    &["sheetData"],
    &["sheetCalcPr"],
    &["sheetProtection"],
    &["protectedRanges"],
    &["scenarios"],
    &["autoFilter"],
    &["sortState"],
    &["dataConsolidate"],
    &["customSheetViews"],
    &["mergeCells"],
    &["phoneticPr"],
    &["conditionalFormatting"],
    &["dataValidations"],
    &["hyperlinks"],
    &["printOptions"],
    &["pageMargins"],
    &["pageSetup"],
    &["headerFooter"],
    &["rowBreaks"],
    &["colBreaks"],
    &["customProperties"],
    &["cellWatches"],
    &["ignoredErrors"],
    &["smartTags"],
    &["drawing"],
    &["legacyDrawing"],
    &["legacyDrawingHF"],
    &["drawingHF"],
    &["picture"],
    &["oleObjects"],
    &["controls"],
    &["webPublishItems"],
    &["tableParts"],
    &["extLst"],
];

/// The shared string table being built: each distinct string once, in first-use order.
#[derive(Default)]
struct Strings {
    items: Vec<Element>,
    index: HashMap<String, usize>,
    uses: usize,
    /// How many strings the workbook's own table had.
    original: usize,
}

/// What makes two strings the same string: their text and, for rich text, each run's formatting.
pub(super) fn string_key(value: &Value) -> String {
    match value {
        Value::Rich(runs) => format!("{:?}", runs.iter().map(|r| (&r.text, &r.props)).collect::<Vec<_>>()),
        v => format!("t:{}", v.display()),
    }
}

impl Strings {
    /// Starts from the table the workbook has, as it is, so strings keep their numbers and
    /// cells that didn't change don't either.
    fn from_table(items: Vec<Element>) -> Strings {
        let mut s = Strings::default();
        for si in items {
            s.index.entry(string_key(&super::props::read_string(&si))).or_insert(s.items.len());
            s.items.push(si);
        }
        s.original = s.items.len();
        s
    }

    fn add(&mut self, value: &Value) -> usize {
        self.uses += 1;
        let key = string_key(value);
        if let Some(&i) = self.index.get(&key) {
            return i;
        }
        self.items.push(string_item("si", value));
        self.index.insert(key, self.items.len() - 1);
        self.items.len() - 1
    }
}

/// `si` (shared) or `is` (inline): `t` for plain text, `r` runs for rich text.
fn string_item(local: &str, value: &Value) -> Element {
    let t = |text: &str| {
        let mut t = Element::new(ns::X, "t").with_text(escape(text));
        if text.starts_with(char::is_whitespace) || text.ends_with(char::is_whitespace) {
            t.set_attr(ns::XML, "space", "preserve");
        }
        t
    };
    let mut el = Element::new(ns::X, local);
    match value {
        Value::Rich(runs) => {
            for r in runs {
                let mut run = Element::new(ns::X, "r");
                if let Some(pr) = run_props(r) {
                    run.push(pr);
                }
                el.push(run.with_child(t(&r.text)));
            }
        }
        v => {
            el.push(t(&v.display()));
        }
    }
    el
}

/// A rich-text run's `rPr`, in the order SpreadsheetML's font elements go.
fn run_props(run: &Run) -> Option<Element> {
    let p = &run.props;
    if p.is_empty() {
        return None;
    }
    let val = |local: &str, v: String| Element::new(ns::X, local).with_attr("", "val", v);
    let flag = |local: &str, on: Option<bool>| on.map(|on| if on { Element::new(ns::X, local) } else { val(local, "0".into()) });
    let mut el = Element::new(ns::X, "rPr");
    let children = [
        p.font.as_ref().filter(|f| !f.starts_with('+')).map(|f| val("rFont", f.clone())),
        flag("b", p.bold),
        flag("i", p.italic),
        p.strike.map(|s| if s == crate::model::Strike::None { val("strike", "0".into()) } else { Element::new(ns::X, "strike") }),
        p.color.as_ref().map(color),
        p.size.map(|s| val("sz", format_number(s.points()))),
        // Excel's underlines; Word's others (dotted, wavy…) come out single.
        p.underline.as_ref().map(|u| match u.as_str() {
            "single" => Element::new(ns::X, "u"),
            "double" | "singleAccounting" | "doubleAccounting" | "none" => val("u", u.clone()),
            _ => val("u", "single".into()),
        }),
        p.baseline.map(|b| val("vertAlign", if b == Baseline::Superscript { "superscript" } else { "subscript" }.into())),
    ];
    for c in children.into_iter().flatten() {
        el.push(c);
    }
    Some(el)
}

/// A SpreadsheetML color: Excel's numbered theme colors and tint where it can say the color
/// that way, else the value.
pub fn color(c: &crate::drawing::Color) -> Element {
    use crate::drawing::ColorBase;
    let mut el = Element::new(ns::X, "color");
    let slot = match &c.base {
        ColorBase::Scheme { name } => {
            let name = match name.as_str() {
                "tx1" => "dk1",
                "bg1" => "lt1",
                "tx2" => "dk2",
                "bg2" => "lt2",
                n => n,
            };
            super::props::THEME_INDEX.iter().position(|s| *s == name)
        }
        _ => None,
    };
    let tint = match c.transforms.as_slice() {
        [] => Some(0.0),
        [m] if m.kind == "lumMod" => Some(m.value as f64 / 100_000.0 - 1.0),
        [m, o] if m.kind == "lumMod" && o.kind == "lumOff" => Some(o.value as f64 / 100_000.0),
        _ => None,
    };
    match (slot, tint) {
        (Some(i), Some(t)) => {
            el.set_attr("", "theme", i.to_string());
            if t != 0.0 {
                el.set_attr("", "tint", format_number(t));
            }
        }
        _ => {
            let rgb = c.resolve(&|n| Theme::office().scheme_color(n)).rgb;
            el.set_attr("", "rgb", format!("FF{}", rgb.hex()));
        }
    }
    el
}

pub fn write(doc: &Document) -> Result<Package> {
    let Body::Workbook { sheets } = &doc.body else {
        return Err(Error::Invalid("only a workbook can be saved as .xlsx".into()));
    };
    let same = doc.kind == Kind::Sheets && doc.package.is_some();
    let pkg = if same { doc.package.clone().unwrap() } else { scaffold(doc.theme.as_ref().unwrap_or(&Theme::office()))? };
    let names = sheet_names(sheets, doc.kind != Kind::Sheets)?;
    let mut out = Out::new(doc, pkg, same, "xl/media");
    let main = out.pkg.main_part()?;
    let mut workbook = out.pkg.xml(&main)?;
    if let Some(origin) = origin_as(&doc.origin, ns::X, "workbook").filter(|_| same) {
        workbook.root = origin;
    }
    let mut ctx = PartCtx::load(&out.pkg, &main)?;
    let sst_part = out.pkg.related(&main, kind::SHARED_STRINGS)?.into_iter().next().map(|(_, p)| p).filter(|p| out.pkg.has(p));
    // Sheets nobody touched are left as they are, numbers into the string table and all.
    let untouched: Vec<bool> = sheets.iter().map(|s| same && s.untouched() && s.part.as_deref().is_some_and(|p| out.pkg.has(p))).collect();
    let any_written = untouched.contains(&false);
    // Remove stale calculation dependencies through the context that will write the rels.
    if any_written {
        for rel in ctx.rels.of_kind("calcChain").cloned().collect::<Vec<_>>() {
            if let Some(part) = out.pkg.target(&main, &rel) {
                out.pkg.remove(&part);
            }
            ctx.rels.remove(&rel.id);
        }
    }
    // Loaded even when every sheet's cells are untouched: a sheet that gains or loses a chart is
    // still written, and a table rebuilt from it alone would lose the other sheets' strings.
    let mut strings = match &sst_part {
        Some(p) if same => Strings::from_table(super::read::shared_strings(&out.pkg, p)?),
        _ => Strings::default(),
    };

    // Sheets that are gone.
    let kept: Vec<&str> = sheets.iter().filter_map(|s| s.part.as_deref()).filter(|_| same).collect();
    for rel in ctx.rels.of_kind(kind::WORKSHEET).cloned().collect::<Vec<_>>() {
        let Some(part) = out.pkg.target(&main, &rel) else { continue };
        if !kept.contains(&part.as_str()) {
            out.pkg.remove(&part);
            ctx.rels.remove(&rel.id);
        }
    }

    let old_list = workbook.root.child(ns::X, "sheets").cloned();
    let old_sheets: Vec<&Element> = old_list.iter().flat_map(|l| l.children_named(ns::X, "sheet")).collect();
    let worksheet_names: HashSet<_> = names.iter().map(|n| n.to_lowercase()).collect();
    for chart in old_sheets.iter().filter(|s| rel_attr(s, "id").and_then(|id| ctx.rels.get(id)).is_some_and(|r| !r.is(kind::WORKSHEET))) {
        if chart.attr("name").is_some_and(|name| worksheet_names.contains(&name.to_lowercase())) {
            return Err(Error::Invalid("worksheet name duplicates a chart sheet".into()));
        }
    }
    let mut used_ids: HashSet<u32> = old_sheets.iter().filter_map(|s| s.attr("sheetId")?.parse().ok()).collect();
    let mut next_id = 1u32;
    let mut written = HashSet::new();
    let mut list = old_list.clone().unwrap_or_else(|| Element::new(ns::X, "sheets"));
    let mut entries = Vec::new();
    for ((sheet, untouched), name) in sheets.iter().zip(untouched).zip(names) {
        let original = sheet.part.as_deref().filter(|p| same && out.pkg.has(p));
        let copy_of = original.filter(|p| written.contains(*p));
        let part = match original.filter(|_| copy_of.is_none()) {
            Some(p) => p.to_string(),
            None => {
                let part = out.pkg.unused_name("xl/worksheets/sheet{}.xml");
                if let Some(source) = copy_of {
                    let source_pkg = doc.package.as_ref().unwrap();
                    out.pkg.put(&part, source_pkg.part(source)?.to_vec(), types::XLSX_SHEET);
                    let rels = source_pkg.rels(source)?;
                    out.pkg.put_rels(&part, &rels);
                }
                part
            }
        };
        // Charts first: a sheet that gains or loses its drawing must say so itself.
        let link = super::chart::write_charts(sheet, &part, sheets, same && copy_of.is_none(), &mut out.pkg)?;
        let shown = origin_as(&sheet.origin, ns::X, "worksheet")
            .filter(|_| same && copy_of.is_none())
            .and_then(|o| o.child(ns::X, "drawing").and_then(|d| rel_attr(d, "id")).map(str::to_string));
        let drawing = match link {
            super::chart::DrawingLink::Same => None,
            super::chart::DrawingLink::Linked(rid) => (shown.as_deref() != Some(rid.as_str())).then_some(Some(rid)),
            super::chart::DrawingLink::Gone => shown.is_some().then_some(None),
        };
        if !untouched || drawing.is_some() {
            write_sheet(sheet, &part, &mut strings, same, drawing, &mut out)?;
        }
        written.insert(part.clone());
        let rid = ctx.relate(kind::WORKSHEET, &part);
        let existing = old_sheets.iter().find(|s| rel_attr(s, "id") == Some(rid.as_str())).map(|e| (*e).clone());
        let mut el = if let Some(existing) = existing {
            existing
        } else {
            while used_ids.contains(&next_id) {
                next_id = next_id.checked_add(1).ok_or_else(|| Error::Invalid("no worksheet ids remain".into()))?;
            }
            let id = next_id;
            used_ids.insert(id);
            Element::new(ns::X, "sheet").with_attr("", "name", "").with_attr("", "sheetId", id.to_string()).with_attr(
                ns::R,
                "id",
                rid.clone(),
            )
        };
        if el.attr("name") != Some(name.as_str()) {
            el.set_attr("", "name", &name);
        }
        let hidden_now = matches!(el.attr("state"), Some("hidden" | "veryHidden"));
        if hidden_now != sheet.hidden {
            crate::formats::dml::set_opt(&mut el, "state", sheet.hidden.then(|| "hidden".to_string()));
        }
        entries.push(el);
    }
    // Chart sheets aren't in the model; they keep their places, and the worksheets fill the
    // worksheets' places in order (any more go at the end).
    let rids = |list: &Element| list.children_named(ns::X, "sheet").map(|e| rel_attr(e, "id").map(str::to_string)).collect::<Vec<_>>();
    let old_rids = rids(&list);
    let mut entries = entries.into_iter();
    let mut children = Vec::new();
    for n in std::mem::take(&mut list.children) {
        match n {
            Node::Element(e) if e.is(ns::X, "sheet") => {
                let chart = rel_attr(&e, "id").and_then(|id| ctx.rels.get(id)).is_some_and(|r| !r.is(kind::WORKSHEET));
                if chart {
                    children.push(Node::Element(e));
                } else if let Some(w) = entries.next() {
                    children.push(Node::Element(w));
                }
            }
            other => children.push(other),
        }
    }
    children.extend(entries.map(Node::Element));
    list.children = children;
    let new_rids = rids(&list);
    if old_rids != new_rids {
        follow_sheet_positions(&mut workbook.root, &old_rids, &new_rids);
    }
    const WORKBOOK: &[&[&str]] = &[
        &["fileVersion"],
        &["fileSharing"],
        &["workbookPr"],
        &["workbookProtection"],
        &["bookViews"],
        &["sheets"],
        &["functionGroups"],
        &["externalReferences"],
        &["definedNames"],
        &["calcPr"],
        &["oleSize"],
        &["customWorkbookViews"],
        &["pivotCaches"],
        &["smartTagPr"],
        &["smartTagTypes"],
        &["webPublishing"],
        &["fileRecoveryPr"],
        &["webPublishObjects"],
        &["extLst"],
    ];
    workbook.root.place(list, WORKBOOK);

    // The shared strings, if anything uses them or the workbook had a table.
    // Strings are only ever added (the table keeps unused ones, as Excel's does), so a table
    // with nothing new is left alone.
    let had_table = sst_part.is_some();
    let sst_part = sst_part.or_else(|| (!strings.items.is_empty()).then(|| "xl/sharedStrings.xml".to_string()));
    if let Some(part) = sst_part.filter(|_| !(had_table && strings.items.len() == strings.original) || !same) {
        let mut sst = Element::new(ns::X, "sst");
        sst.declare(None, ns::X);
        sst.set_attr("", "count", strings.uses.max(strings.items.len()).to_string());
        sst.set_attr("", "uniqueCount", strings.items.len().to_string());
        let scope = sst.attrs.clone();
        let body: String = strings.items.iter().map(|si| si.to_xml_string_in(&scope)).collect();
        out.pkg.put(&part, splice(sst, &body), types::XLSX_SHARED_STRINGS);
        ctx.relate(kind::SHARED_STRINGS, &part);
    }
    // A same-package save of a macro-enabled workbook (.xlsm) must keep that main-part type.
    // Replacing it with the plain xlsx type makes Excel drop the macros. A new workbook is xlsx.
    let main_type = if same { out.pkg.content_type(&main).unwrap_or(types::XLSX_MAIN).to_string() } else { types::XLSX_MAIN.to_string() };
    put_if_changed(&mut out.pkg, &main, workbook, &main_type)?;
    ctx.finish(&mut out.pkg);
    doc.properties.write(&mut out.pkg)?;
    Ok(std::mem::take(&mut out.pkg))
}

/// Excel sheet names, either validated or made from generated labels such as CSV filenames.
pub(super) fn sheet_names(sheets: &[Sheet], generated: bool) -> Result<Vec<String>> {
    let mut used = HashSet::new();
    let mut names = Vec::with_capacity(sheets.len());
    for sheet in sheets {
        let mut name = if generated {
            sheet
                .name
                .chars()
                .map(|c| if ":\\/?*[]".contains(c) { '_' } else { c })
                .collect::<String>()
                .trim_matches('\'')
                .chars()
                .take(31)
                .collect::<String>()
                .trim_end_matches('\'')
                .to_string()
        } else {
            sheet.name.clone()
        };
        if generated {
            if name.is_empty() || name.eq_ignore_ascii_case("History") {
                name = "Sheet".into();
            }
            let base = name.clone();
            let mut suffix = 2usize;
            while used.contains(&name.to_lowercase()) {
                let tail = format!(" ({suffix})");
                name = base.chars().take(31usize.saturating_sub(tail.len())).collect::<String>() + &tail;
                suffix += 1;
            }
        }
        if name.is_empty()
            // Excel counts a character it writes as `_x000A_` once.
            || super::props::unescape(&name).chars().count() > 31
            || name.chars().any(|c| ":\\/?*[]".contains(c))
            || name.starts_with('\'')
            || name.ends_with('\'')
            || name.eq_ignore_ascii_case("History")
            || !used.insert(name.to_lowercase())
        {
            return Err(Error::Invalid(format!("invalid or duplicate worksheet name {:?}", sheet.name)));
        }
        names.push(name);
    }
    Ok(names)
}

/// The ISO calendar date and optional time accepted by SpreadsheetML's date cells.
fn valid_iso_date(text: &str) -> bool {
    let b = text.as_bytes();
    let number = |s: &[u8]| s.iter().try_fold(0u32, |n, &c| c.is_ascii_digit().then(|| n * 10 + (c - b'0') as u32));
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let (Some(year), Some(month), Some(day)) = (number(&b[..4]), number(&b[5..7]), number(&b[8..10])) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        _ => return false,
    };
    if year == 0 || day == 0 || day > days {
        return false;
    }
    if b.len() == 10 {
        return true;
    }
    if b.len() < 19 || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return false;
    }
    let (Some(h), Some(m), Some(s)) = (number(&b[11..13]), number(&b[14..16]), number(&b[17..19])) else {
        return false;
    };
    if h > 23 || m > 59 || s > 59 {
        return false;
    }
    let mut tail = &b[19..];
    if tail.first() == Some(&b'.') {
        tail = &tail[1..];
        let digits = tail.iter().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 {
            return false;
        }
        tail = &tail[digits..];
    }
    match tail {
        [] | [b'Z'] => true,
        [b'+' | b'-', h1, h2, b':', m1, m2] => {
            matches!(number(&[*h1, *h2]), Some(0..=14))
                && matches!(number(&[*m1, *m2]), Some(0..=59))
                && (&tail[1..3] != b"14" || &tail[4..] == b"00")
        }
        _ => false,
    }
}

/// Points what refers to sheets by position (sheet-scoped names, the active and first tabs)
/// at where those sheets are now. A name scoped to a sheet that's gone goes with it.
fn follow_sheet_positions(workbook: &mut Element, old: &[Option<String>], new: &[Option<String>]) {
    let moved = |i: &str| -> Option<usize> {
        let rid = old.get(i.parse::<usize>().ok()?)?.as_ref()?;
        new.iter().position(|r| r.as_ref() == Some(rid))
    };
    if let Some(names) = workbook.child_mut(ns::X, "definedNames") {
        names.children.retain_mut(|n| {
            let Node::Element(e) = n else { return true };
            let Some(i) = e.attr("localSheetId").map(str::to_string) else { return true };
            match moved(&i) {
                Some(to) => {
                    e.set_attr("", "localSheetId", to.to_string());
                    true
                }
                None => false,
            }
        });
    }
    if let Some(views) = workbook.child_mut(ns::X, "bookViews") {
        for view in views.elements_mut() {
            for attr in ["activeTab", "firstSheet"] {
                if let Some(i) = view.attr(attr).map(str::to_string) {
                    let to = moved(&i).unwrap_or(0);
                    if to == 0 {
                        view.remove_attr("", attr);
                    } else {
                        view.set_attr("", attr, to.to_string());
                    }
                }
            }
        }
    }
}

/// Where the rows go in a sheet written as text; see [`splice`].
const ROWS: &str = "office-core:rows";

/// `root` as a part, with `content` (already XML) in place of the [`ROWS`] comment, or as the
/// root's content if there's no such comment. Big sheets and string tables are written this
/// way, a row or a string at a time, never as one tree.
fn splice(mut root: Element, content: &str) -> Vec<u8> {
    let has_marker = crate::xml::stream::contains_comment(&root, ROWS);
    if !has_marker {
        root.children = vec![Node::Comment(ROWS.into())];
    }
    let text = String::from_utf8(XmlDoc::new(root).to_bytes()).expect("the writer writes UTF-8");
    text.replacen(&format!("<!--{ROWS}-->"), content, 1).into_bytes()
}

/// `drawing`: the sheet's drawing changed, to this relationship id or to none.
fn write_sheet(sheet: &Sheet, part: &str, strings: &mut Strings, same: bool, drawing: Option<Option<String>>, out: &mut Out) -> Result<()> {
    // Another package's sheet can't bring its drawings and comments (the parts its r:ids name)
    // along; anything else about it can come.
    let mut root = origin_as(&sheet.origin, ns::X, "worksheet").filter(|o| same || !names_parts(o)).unwrap_or_else(|| {
        let mut ws = Element::new(ns::X, "worksheet");
        ws.declare(None, ns::X);
        ws.declare(Some("r"), ns::R);
        ws
    });
    let mut data = root.child(ns::X, "sheetData").cloned().unwrap_or_else(|| Element::new(ns::X, "sheetData"));
    data.children = vec![Node::Comment(ROWS.into())];
    let mut scope: Vec<_> = root.attrs.iter().filter(|a| a.is_ns_decl()).cloned().collect();
    scope.extend(data.attrs.iter().filter(|a| a.is_ns_decl()).cloned());
    root.place(data, WORKSHEET);

    let mut rows = String::new();
    for row in &sheet.rows {
        let mut r = origin_as(&row.origin, ns::X, "row").unwrap_or_else(|| Element::new(ns::X, "row"));
        r.children.retain(|n| !matches!(n, Node::Element(e) if e.is(ns::X, "c")));
        r.set_attr("", "r", (row.index + 1).to_string());
        // `spans` is an optional hint of the row's first and last columns; keep it right.
        if r.attr("spans").is_some() {
            match (row.cells.first(), row.cells.last()) {
                (Some(f), Some(l)) => r.set_attr("", "spans", format!("{}:{}", f.col + 1, l.col + 1)),
                _ => {
                    r.remove_attr("", "spans");
                }
            }
        }
        let others = std::mem::take(&mut r.children);
        for cell in &row.cells {
            r.push(write_cell(cell, CellRef::new(row.index, cell.col), strings));
        }
        // A row's extensions come after its cells.
        r.children.extend(others);
        rows.push_str(&r.to_xml_string_in(&scope));
    }

    let dimension = sheet.used_range().unwrap_or(Range { start: CellRef::new(0, 0), end: CellRef::new(0, 0) });
    let dim = root.child(ns::X, "dimension").and_then(|d| d.attr("ref")).and_then(Range::parse);
    if dim != Some(dimension) {
        root.place(Element::new(ns::X, "dimension").with_attr("", "ref", dimension.to_string()), WORKSHEET);
    }
    root.remove_children(ns::X, "mergeCells");
    if !sheet.merges.is_empty() {
        let merges =
            sheet.merges.iter().fold(Element::new(ns::X, "mergeCells").with_attr("", "count", sheet.merges.len().to_string()), |m, r| {
                m.with_child(Element::new(ns::X, "mergeCell").with_attr("", "ref", r.to_string()))
            });
        root.place(merges, WORKSHEET);
    }
    match drawing {
        Some(Some(rid)) => root.place(Element::new(ns::X, "drawing").with_attr(ns::R, "id", rid), WORKSHEET),
        Some(None) => {
            root.remove_children(ns::X, "drawing");
        }
        None => {}
    }
    // A sheet needs sheetData even if it's empty, and Excel asks for page margins.
    if root.child(ns::X, "pageMargins").is_none() && !same {
        let margins = [("left", "0.7"), ("right", "0.7"), ("top", "0.75"), ("bottom", "0.75"), ("header", "0.3"), ("footer", "0.3")];
        root.place(margins.iter().fold(Element::new(ns::X, "pageMargins"), |el, (k, v)| el.with_attr("", k, *v)), WORKSHEET);
    }
    out.pkg.put(part, splice(root, &rows), types::XLSX_SHEET);
    Ok(())
}

fn write_cell(cell: &SheetCell, at: CellRef, strings: &mut Strings) -> Element {
    let origin = origin_as(&cell.origin, ns::X, "c");
    let old_formula = origin.as_ref().and_then(|o| o.child(ns::X, "f").cloned());
    let inline = origin.as_ref().and_then(|o| o.attr("t")) == Some("inlineStr");
    // Strict files' ISO dates, which the model holds as their text.
    let iso_date = origin.as_ref().and_then(|o| o.attr("t")) == Some("d");
    let old_inline = origin.as_ref().and_then(|o| o.child(ns::X, "is")).filter(|is| super::props::read_string(is) == cell.value).cloned();
    let old_index = origin
        .as_ref()
        .filter(|o| o.attr("t") == Some("s"))
        .and_then(|o| o.child(ns::X, "v"))
        .and_then(|v| v.text().trim().parse::<usize>().ok())
        .filter(|&i| strings.items.get(i).is_some_and(|si| super::props::read_string(si) == cell.value));
    let empty_v = origin.as_ref().and_then(|o| o.child(ns::X, "v")).is_some_and(|v| v.text().is_empty());
    // A formula's empty text result, which an empty `v` without the type would not say.
    let empty_text = empty_v && origin.as_ref().and_then(|o| o.attr("t")) == Some("str");
    let mut c = origin.unwrap_or_else(|| Element::new(ns::X, "c"));
    c.children.retain(|n| !matches!(n, Node::Element(e) if e.is(ns::X, "f") || e.is(ns::X, "v") || e.is(ns::X, "is")));
    c.set_attr("", "r", at.to_string());
    match cell.style {
        Some(s) => c.set_attr("", "s", s.to_string()),
        None => {
            c.remove_attr("", "s");
        }
    }
    // The formula: kept as it was if it hasn't changed (or if the cell is part of a shared
    // formula, which has no text of its own), else written plain.
    let formula = match (&cell.formula, old_formula) {
        (Some(f), Some(old)) if old.text() == *f => Some(old),
        (None, Some(old)) if old.text().is_empty() => Some(old),
        (Some(f), _) => Some(Element::new(ns::X, "f").with_text(f.clone())),
        (None, _) => None,
    };
    let has_formula = formula.is_some();
    if let Some(f) = formula {
        c.children.insert(0, Node::Element(f));
    }
    let v = |text: String| Element::new(ns::X, "v").with_text(text);
    let (t, value) = match &cell.value {
        Value::Empty => ((empty_text && has_formula).then_some("str"), empty_v.then(|| Element::new(ns::X, "v"))),
        Value::Number(n) => (None, Some(v(xlsx_number(*n)))),
        Value::Bool(b) => (Some("b"), Some(v(if *b { "1" } else { "0" }.into()))),
        Value::Error(e) => (Some("e"), Some(v(e.clone()))),
        // A formula's text result is a plain string, not a shared one.
        Value::Text(s) if has_formula => (Some("str"), Some(v(escape(s)))),
        Value::Text(s) if iso_date && valid_iso_date(s) => (Some("d"), Some(v(s.clone()))),
        value @ (Value::Text(_) | Value::Rich(_)) if inline => {
            (Some("inlineStr"), Some(old_inline.unwrap_or_else(|| string_item("is", value))))
        }
        value @ (Value::Text(_) | Value::Rich(_)) => (
            Some("s"),
            Some(v(match old_index {
                Some(i) => {
                    strings.uses += 1;
                    i
                }
                None => strings.add(value),
            }
            .to_string())),
        ),
    };
    // No type is a number; some writers say so with t="n".
    if t != c.attr("t").filter(|t| *t != "n") {
        crate::formats::dml::set_opt(&mut c, "t", t.map(str::to_string));
    }
    if let Some(value) = value {
        // v (or is) goes after f, before extLst.
        let at = c.children.iter().position(|n| matches!(n, Node::Element(e) if e.is(ns::X, "extLst"))).unwrap_or(c.children.len());
        c.children.insert(at, Node::Element(value));
    }
    c
}

/// A number the way Excel writes one: the shortest form that reads back the same, with an
/// exponent for the very large and very small.
fn xlsx_number(n: f64) -> String {
    if n != 0.0 && (n.abs() >= 1e15 || n.abs() < 1e-6) {
        format!("{n:E}")
    } else {
        format_number(n)
    }
}

/// A new package with what Excel needs around sheets: the workbook, a theme, and the smallest
/// style sheet Excel opens (one font, the two fills it insists on, one border, one format).
fn scaffold(theme: &Theme) -> Result<Package> {
    let mut pkg = Package::new();
    let main = "xl/workbook.xml";
    let mut wb = Element::new(ns::X, "workbook");
    wb.declare(None, ns::X);
    wb.declare(Some("r"), ns::R);
    wb.push(Element::new(ns::X, "bookViews").with_child(Element::new(ns::X, "workbookView")));
    wb.push(Element::new(ns::X, "sheets"));
    pkg.put_xml(main, &XmlDoc::new(wb), types::XLSX_MAIN);
    pkg.relate("", kind::OFFICE_DOCUMENT, main)?;

    let font = theme.fonts.minor.latin.clone();
    let el = |local: &str| Element::new(ns::X, local);
    let val = |local: &str, v: &str| el(local).with_attr("", "val", v);
    let list = |local: &str, items: Vec<Element>| items.into_iter().fold(el(local).with_attr("", "count", "0"), |l, i| l.with_child(i));
    let mut styles = el("styleSheet");
    styles.declare(None, ns::X);
    let mut parts = vec![
        list(
            "fonts",
            vec![el("font")
                .with_child(val("sz", "11"))
                .with_child(el("color").with_attr("", "theme", "1"))
                .with_child(val("name", if font.is_empty() { "Calibri" } else { &font }))
                .with_child(val("family", "2"))
                .with_child(val("scheme", "minor"))],
        ),
        list(
            "fills",
            vec![
                el("fill").with_child(el("patternFill").with_attr("", "patternType", "none")),
                el("fill").with_child(el("patternFill").with_attr("", "patternType", "gray125")),
            ],
        ),
        list(
            "borders",
            vec![el("border")
                .with_child(el("left"))
                .with_child(el("right"))
                .with_child(el("top"))
                .with_child(el("bottom"))
                .with_child(el("diagonal"))],
        ),
        list(
            "cellStyleXfs",
            vec![el("xf")
                .with_attr("", "numFmtId", "0")
                .with_attr("", "fontId", "0")
                .with_attr("", "fillId", "0")
                .with_attr("", "borderId", "0")],
        ),
        list(
            "cellXfs",
            vec![el("xf")
                .with_attr("", "numFmtId", "0")
                .with_attr("", "fontId", "0")
                .with_attr("", "fillId", "0")
                .with_attr("", "borderId", "0")
                .with_attr("", "xfId", "0")],
        ),
        list(
            "cellStyles",
            vec![el("cellStyle").with_attr("", "name", "Normal").with_attr("", "xfId", "0").with_attr("", "builtinId", "0")],
        ),
    ];
    for p in &mut parts {
        let n = p.elements().count();
        p.set_attr("", "count", n.to_string());
    }
    styles.children.extend(parts.into_iter().map(Node::Element));
    pkg.put_xml("xl/styles.xml", &XmlDoc::new(styles), types::XLSX_STYLES);
    pkg.relate(main, kind::STYLES, "xl/styles.xml")?;
    pkg.put_xml("xl/theme/theme1.xml", &theme.to_xml(), types::THEME);
    pkg.relate(main, kind::THEME, "xl/theme/theme1.xml")?;
    Ok(pkg)
}
