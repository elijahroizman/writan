//! The office compiler's Excel layer: what only spreadsheets have, on top of `office-core`'s
//! sheets of values.
//!
//! - **Formats**: each cell's [`CellFormat`] (number format, font, fill, border, alignment),
//!   reading and adding them through the style sheet ([`styles`]), and showing a value the
//!   way Excel does ([`display`], [`numfmt`], [`dates`]).
//! - **Formulas**: parsing ([`formula`]), the formulas of cells that share one, and working
//!   them out again ([`recalculate`], [`eval`]).
//! - **The workbook**: defined names, the date system, column widths and row heights.
//!
//! Add [`Sheets`] to a compiler to have it read and write all this:
//!
//! ```
//! use office_core::model::{Body, CellRef, Value};
//! let compiler = office_core::Compiler::new().with_layer(office_sheets::Sheets);
//! let mut doc = office_core::Document::new(office_core::Kind::Sheets);
//! let Body::Workbook { sheets } = &mut doc.body else { unreachable!() };
//! sheets[0].set(CellRef::new(0, 0), Value::Number(0.256));
//! let mut format = office_sheets::format_of(&doc, 0, CellRef::new(0, 0));
//! format.number_format = "0.0%".into();
//! office_sheets::set_format(&mut doc, 0, CellRef::new(0, 0), &format);
//! assert_eq!(office_sheets::display(&doc, 0, CellRef::new(0, 0)), "25.6%");
//! let saved = compiler.write(&doc, office_core::Kind::Sheets).unwrap();
//! let back = compiler.read(&saved, Some("book.xlsx")).unwrap();
//! assert_eq!(office_sheets::display(&back, 0, CellRef::new(0, 0)), "25.6%");
//! ```

pub mod conditional;
pub mod dates;
pub mod eval;
pub mod formula;
pub mod numfmt;
pub mod styles;
pub mod validation;
pub mod view;

use std::collections::{HashMap, HashSet};

use office_core::model::{Body, CellRef, Document, Kind, Origin, Sheet, SheetCell, Value};
use office_core::opc::Package;
use office_core::units::parse_bool;
use office_core::xml::{ns, Element, Node};
use office_core::{Layer, Result};

pub use dates::{DateSystem, DateTime};
pub use styles::{Alignment, Border, BorderSide, CellFormat, Fill, Font, StyleSheet};

use eval::{Book, V};
use formula::Expr;

/// The layer: add it to a [`office_core::Compiler`].
pub struct Sheets;

/// What the layer keeps on a workbook (in its extensions).
#[derive(Clone, Debug)]
pub struct Excel {
    pub styles: StyleSheet,
    pub date_system: DateSystem,
    pub names: Vec<DefinedName>,
}

/// A defined name: `TaxRate` for `Settings!$B$2`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefinedName {
    pub name: String,
    /// The sheet it belongs to (by position), or `None` for the whole workbook.
    pub sheet: Option<usize>,
    /// What it refers to, as formula text.
    pub formula: String,
    pub hidden: bool,
}

impl Default for Excel {
    fn default() -> Excel {
        Excel { styles: StyleSheet::new_default(), date_system: DateSystem::default(), names: Vec::new() }
    }
}

impl Layer for Sheets {
    fn name(&self) -> &'static str {
        "Excel"
    }

    fn kinds(&self) -> &[Kind] {
        &[Kind::Sheets]
    }

    fn after_read(&self, doc: &mut Document) -> Result<()> {
        let styles = match &doc.package {
            Some(pkg) => StyleSheet::read(pkg)?,
            None => None,
        };
        let workbook = doc.origin.get();
        let date_system = match workbook.and_then(|w| w.child(ns::X, "workbookPr")).and_then(|p| p.attr("date1904")).and_then(parse_bool) {
            Some(true) => DateSystem::From1904,
            _ => DateSystem::From1900,
        };
        let names = workbook
            .and_then(|w| w.child(ns::X, "definedNames"))
            .into_iter()
            .flat_map(|d| d.children_named(ns::X, "definedName"))
            .filter_map(|d| {
                Some(DefinedName {
                    name: d.attr("name")?.to_string(),
                    sheet: d.attr("localSheetId").and_then(|i| i.parse().ok()),
                    formula: d.text(),
                    hidden: d.attr("hidden").and_then(parse_bool).unwrap_or(false),
                })
            })
            .collect::<Vec<_>>();
        if let Body::Workbook { sheets } = &mut doc.body {
            for n in names.iter().filter(|n| n.name.eq_ignore_ascii_case(SIZE_NAME)) {
                if let (Some(sheet), Some(size)) = (n.sheet.and_then(|i| sheets.get_mut(i)), parse_size(&n.formula)) {
                    sheet.size = Some(size);
                }
            }
        }
        doc.extensions.insert(Excel { styles: styles.unwrap_or_else(StyleSheet::new_default), date_system, names });
        Ok(())
    }

    fn before_write(&self, doc: &mut Document, to: Kind) -> Result<()> {
        if to == Kind::Sheets {
            if let Body::Workbook { sheets } = &mut doc.body {
                for sheet in sheets {
                    for row in &mut sheet.rows {
                        for cell in &mut row.cells {
                            if let Some(f) = &mut cell.formula {
                                // Keep imported shared leaders and their markup intact.
                                let original = cell.origin.get().and_then(|c| c.child(ns::X, "f"));
                                if original.is_none_or(|old| old.text() != *f) {
                                    *f = formula::file_text(f);
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn write_parts(&self, doc: &Document, package: &mut Package, _to: Kind) -> Result<()> {
        if let Some(excel) = doc.extensions.get::<Excel>() {
            excel.styles.write(package)?;
        }
        write_sizes(doc, package)
    }
}

/// The hidden, sheet-scoped name that holds a sheet's size (rows and columns) as `{500,52}`.
/// Excel keeps names it doesn't use, and a constant needs nothing changed when the sheet is renamed.
pub const SIZE_NAME: &str = "Writan.SheetSize";
/// The most rows and columns an Excel sheet has.
pub const MAX_ROWS: u32 = 1_048_576;
pub const MAX_COLS: u32 = 16_384;

fn parse_size(formula: &str) -> Option<(u32, u32)> {
    let inner = formula.trim().trim_start_matches('=').trim().strip_prefix('{')?.strip_suffix('}')?;
    let (r, c) = inner.split_once([',', ';'])?;
    let (r, c) = (r.trim().parse::<u32>().ok()?, c.trim().parse::<u32>().ok()?);
    (r >= 1 && c >= 1).then_some((r.min(MAX_ROWS), c.min(MAX_COLS)))
}

/// Puts each sheet's size in the workbook's names, by the sheet's position when written.
fn write_sizes(doc: &Document, pkg: &mut Package) -> Result<()> {
    let Body::Workbook { sheets } = &doc.body else { return Ok(()) };
    let main = pkg.main_part()?;
    let mut workbook = pkg.xml(&main)?;
    let wanted: Vec<Element> = sheets
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let (rows, cols) = s.size?;
            Some(
                Element::new(ns::X, "definedName")
                    .with_attr("", "name", SIZE_NAME)
                    .with_attr("", "localSheetId", i.to_string())
                    .with_attr("", "hidden", "1")
                    .with_text(format!("{{{rows},{cols}}}")),
            )
        })
        .collect();
    let named = |e: &Element| e.attr("name").is_some_and(|v| v.eq_ignore_ascii_case(SIZE_NAME));
    let is_size = |n: &Node| matches!(n, Node::Element(e) if named(e));
    let existing: Vec<Element> =
        workbook.root.child(ns::X, "definedNames").map(|d| d.elements().filter(|e| named(e)).cloned().collect()).unwrap_or_default();
    if existing == wanted {
        return Ok(());
    }
    let mut names = workbook.root.child(ns::X, "definedNames").cloned().unwrap_or_else(|| Element::new(ns::X, "definedNames"));
    names.children.retain(|n| !is_size(n));
    for e in wanted {
        names.push(e);
    }
    if names.elements().next().is_some() {
        workbook.root.place(names, WORKBOOK_ORDER);
    } else {
        workbook.root.remove_children(ns::X, "definedNames");
    }
    let media = pkg.content_type(&main).unwrap_or("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml").to_string();
    pkg.put_xml(&main, &workbook, &media);
    Ok(())
}

/// The children of `workbook`, in schema order.
const WORKBOOK_ORDER: &[&[&str]] = &[
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

/// The layer's data on a workbook, if it has been read with the layer or given formats.
pub fn excel(doc: &Document) -> Option<&Excel> {
    doc.extensions.get::<Excel>()
}

/// The layer's data on a workbook, made (with a new workbook's style sheet) if it has none.
pub fn excel_mut(doc: &mut Document) -> &mut Excel {
    if doc.extensions.get::<Excel>().is_none() {
        doc.extensions.insert(Excel::default());
    }
    doc.extensions.get_mut::<Excel>().unwrap()
}

fn sheet(doc: &Document, index: usize) -> Option<&Sheet> {
    match &doc.body {
        Body::Workbook { sheets } => sheets.get(index),
        _ => None,
    }
}

fn sheet_mut(doc: &mut Document, index: usize) -> Option<&mut Sheet> {
    match &mut doc.body {
        Body::Workbook { sheets } => sheets.get_mut(index),
        _ => None,
    }
}

// ---- formats ----

/// A cell's format (the default for a cell without one).
pub fn format_of(doc: &Document, sheet_index: usize, at: CellRef) -> CellFormat {
    let style = sheet(doc, sheet_index).and_then(|s| s.cell(at)).and_then(|c| c.style).unwrap_or(0);
    match excel(doc) {
        Some(x) => x.styles.format(style),
        None => CellFormat::default(),
    }
}

/// Gives a cell a format, adding it to the style sheet if it isn't there. The cell is made if
/// it's empty, as formatting an empty cell in Excel does.
pub fn set_format(doc: &mut Document, sheet_index: usize, at: CellRef, format: &CellFormat) {
    let index = excel_mut(doc).styles.add(format);
    if let Some(s) = sheet_mut(doc, sheet_index) {
        s.cell_mut(at).style = (index != 0).then_some(index);
    }
}

/// A cell's value as Excel shows it, through its number format.
pub fn display(doc: &Document, sheet_index: usize, at: CellRef) -> String {
    let Some(s) = sheet(doc, sheet_index) else { return String::new() };
    let value = s.value(at);
    let code = format_of(doc, sheet_index, at).number_format;
    numfmt::format(value, &code, excel(doc).map(|x| x.date_system).unwrap_or_default()).text
}

/// The date and time a cell shows, if its format shows it as one.
pub fn date_of(doc: &Document, sheet_index: usize, at: CellRef) -> Option<DateTime> {
    let Value::Number(n) = sheet(doc, sheet_index)?.value(at) else { return None };
    if !numfmt::is_date_format(&format_of(doc, sheet_index, at).number_format) {
        return None;
    }
    dates::from_serial(*n, excel(doc).map(|x| x.date_system).unwrap_or_default())
}

// ---- formulas ----

/// A shared formula's `f`: its group, and the range it covers if this is the group's first cell.
fn shared(origin: &Origin) -> Option<(&str, Option<&str>)> {
    let f = origin.get()?.child(ns::X, "f")?;
    (f.attr("t") == Some("shared")).then(|| (f.attr("si").unwrap_or(""), f.attr("ref")))
}

/// A cell's formula (without its `=`), including a formula it shares with the cells around it:
/// Excel stores a copied formula once, and the others say only that they share it.
pub fn formula(doc: &Document, sheet_index: usize, at: CellRef) -> Option<String> {
    let s = sheet(doc, sheet_index)?;
    let cell = s.cell(at)?;
    if let Some(f) = &cell.formula {
        return Some(f.clone());
    }
    let (group, _) = shared(&cell.origin)?;
    // The group's first cell has the text; this one is that text copied here.
    for row in &s.rows {
        for c in &row.cells {
            if let (Some(text), Some((g, Some(_)))) = (&c.formula, shared(&c.origin)) {
                if g == group {
                    return formula::shift(text, at.row as i64 - row.index as i64, at.col as i64 - c.col as i64).ok();
                }
            }
        }
    }
    None
}

struct SheetFormulas<'a> {
    sheet: &'a Sheet,
    leaders: Option<HashMap<&'a str, (CellRef, &'a str)>>,
}

impl<'a> SheetFormulas<'a> {
    fn new(sheet: &'a Sheet) -> Self {
        Self { sheet, leaders: None }
    }

    fn read(&mut self, at: CellRef, cell: &SheetCell) -> Option<String> {
        if let Some(text) = &cell.formula {
            return Some(text.clone());
        }
        let (group, _) = shared(&cell.origin)?;
        let leaders = self.leaders.get_or_insert_with(|| {
            self.sheet
                .rows
                .iter()
                .flat_map(|row| {
                    row.cells.iter().filter_map(|c| {
                        let text = c.formula.as_deref()?;
                        let (group, _) = shared(&c.origin).filter(|(_, range)| range.is_some())?;
                        Some((group, (CellRef::new(row.index, c.col), text)))
                    })
                })
                .collect()
        });
        let (leader, text) = leaders.get(group)?;
        formula::shift(text, i64::from(at.row) - i64::from(leader.row), i64::from(at.col) - i64::from(leader.col)).ok()
    }
}

/// What [`recalculate_with_report`] found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recalculation {
    /// How many formulas there were.
    pub formulas: usize,
    /// The formulas found in a circular reference (sheet index and cell), in sheet order.
    pub circular: Vec<(usize, CellRef)>,
}

/// Works out every formula in the workbook again and puts the results in the cells, as Excel
/// does when it recalculates. `now` (a serial number) is what NOW() and TODAY() give; without
/// one they're #N/A. A formula in a circular reference comes to 0, as in Excel. Returns how
/// many formulas there were.
pub fn recalculate(doc: &mut Document, now: Option<f64>) -> usize {
    recalculate_with_report(doc, now).formulas
}

/// [`recalculate`], also saying which formulas are in a circular reference.
pub fn recalculate_with_report(doc: &mut Document, now: Option<f64>) -> Recalculation {
    calculate(doc, now, |_| true)
}

/// Works out only the formulas that have no result yet, as a file written by a library
/// without a calculation engine (openpyxl) has them; every other formula keeps the result
/// Excel stored for it. Returns how many formulas were worked out.
pub fn calculate_uncached(doc: &mut Document, now: Option<f64>) -> usize {
    calculate(doc, now, uncached).formulas
}

/// Whether a formula cell has no stored result. An empty `v` under `t="str"` may be a real
/// empty text result, but writers that do not evaluate formulas (report generators such as
/// ClosedXML or POI) write it for a result never worked out, so it is worked out too.
fn uncached(cell: &SheetCell) -> bool {
    cell.value.is_empty()
}

fn calculate(doc: &mut Document, now: Option<f64>, include: impl Fn(&SheetCell) -> bool) -> Recalculation {
    let (results, circular) = evaluate(doc, now, include);
    let formulas = results.len();
    if let Body::Workbook { sheets } = &mut doc.body {
        for ((s, r, c), value) in results {
            let cell = sheets[s].cell_mut(CellRef::new(r, c));
            if cell.value != value {
                cell.value = value;
            }
        }
    }
    Recalculation { formulas, circular }
}

/// The formulas in a circular reference (sheet index and cell), in sheet order, found by
/// working the workbook out without changing it.
pub fn circular_references(doc: &Document, now: Option<f64>) -> Vec<(usize, CellRef)> {
    evaluate(doc, now, |_| true).1
}

type CellKey = (usize, u32, u32);

/// Every formula's result, and the formulas in a circular reference (see [`evaluate`]).
type Evaluated = (Vec<(CellKey, Value)>, Vec<(usize, CellRef)>);

/// Works out the formulas of the cells `include` picks; the rest keep their stored results.
fn evaluate(doc: &Document, now: Option<f64>, include: impl Fn(&SheetCell) -> bool) -> Evaluated {
    let Body::Workbook { sheets } = &doc.body else { return (Vec::new(), Vec::new()) };
    let mut formulas = HashMap::new();
    for (si, s) in sheets.iter().enumerate() {
        let mut reader = SheetFormulas::new(s);
        for row in &s.rows {
            for c in row.cells.iter().filter(|c| include(c)) {
                if let Some(text) = reader.read(CellRef::new(row.index, c.col), c) {
                    let expr = formula::parse(&text).unwrap_or(Expr::Error("#NAME?".into()));
                    formulas.insert((si, row.index, c.col), std::rc::Rc::new(expr));
                }
            }
        }
    }
    let x = excel(doc);
    let mut book = Workbook {
        sheets,
        names: x.map(|x| x.names.clone()).unwrap_or_default(),
        date_system: x.map(|x| x.date_system).unwrap_or_default(),
        now,
        formulas,
        done: HashMap::new(),
        working: HashSet::new(),
        path: Vec::new(),
        reads: HashMap::new(),
        current: vec![0],
        deep: None,
        extents: vec![Default::default(); sheets.len()],
    };
    let keys = book.work_out();
    // A formula is in a circular reference when, through the formulas it reads, it comes back
    // to itself. What the loop got to depends on where it was entered, so the loop's formulas
    // come to 0, as in Excel, and everything is worked out again with that.
    // A loop at 0 can send a formula down another branch (an IF) into a loop of its own, so
    // this goes on until no new one turns up.
    let mut circular = HashSet::new();
    loop {
        let found = loops(&book.reads);
        if found.is_subset(&circular) {
            break;
        }
        circular.extend(found);
        book.done = circular.iter().map(|&k| (k, V::Num(0.0))).collect();
        book.reads.clear();
        book.work_out();
    }
    let results = keys.iter().map(|k| (*k, book.done.get(k).map(V::to_value).unwrap_or(Value::Number(0.0)))).collect();
    let mut circular: Vec<CellKey> = circular.into_iter().collect();
    circular.sort_unstable();
    (results, circular.into_iter().map(|(s, r, c)| (s, CellRef::new(r, c))).collect())
}

/// The formulas that, through the formulas they read, come back to themselves: every
/// strongly connected component of more than one formula, and each formula that reads itself.
fn loops(reads: &HashMap<CellKey, HashSet<CellKey>>) -> HashSet<CellKey> {
    // Tarjan's algorithm, without recursion: a chain of formulas can be thousands long.
    #[derive(Default)]
    struct Search {
        /// Each formula's (index, lowlink).
        index: HashMap<CellKey, (usize, usize)>,
        stack: Vec<CellKey>,
        on_stack: HashSet<CellKey>,
        /// The formulas being searched, each with the reads it has left to follow.
        work: Vec<(CellKey, Vec<CellKey>)>,
    }
    impl Search {
        fn visit(&mut self, k: CellKey, reads: &HashMap<CellKey, HashSet<CellKey>>) {
            let n = self.index.len();
            self.index.insert(k, (n, n));
            self.stack.push(k);
            self.on_stack.insert(k);
            self.work.push((k, reads.get(&k).map(|r| r.iter().copied().collect()).unwrap_or_default()));
        }
    }
    let mut search = Search::default();
    let mut found = HashSet::new();
    let mut roots: Vec<CellKey> = reads.keys().copied().collect();
    roots.sort_unstable();
    for root in roots {
        if search.index.contains_key(&root) {
            continue;
        }
        search.visit(root, reads);
        while let Some((k, next)) = search.work.last_mut() {
            let k = *k;
            if let Some(d) = next.pop() {
                match search.index.get(&d) {
                    None => search.visit(d, reads),
                    Some(&(di, _)) if search.on_stack.contains(&d) => {
                        let e = search.index.get_mut(&k).unwrap();
                        e.1 = e.1.min(di);
                    }
                    Some(_) => {}
                }
                continue;
            }
            search.work.pop();
            let (ki, kl) = search.index[&k];
            if let Some(&(parent, _)) = search.work.last() {
                let e = search.index.get_mut(&parent).unwrap();
                e.1 = e.1.min(kl);
            }
            if kl == ki {
                let mut component = Vec::new();
                while let Some(m) = search.stack.pop() {
                    search.on_stack.remove(&m);
                    component.push(m);
                    if m == k {
                        break;
                    }
                }
                if component.len() > 1 || reads.get(&k).is_some_and(|r| r.contains(&k)) {
                    found.extend(component);
                }
            }
        }
    }
    found
}

impl Workbook<'_> {
    /// Works out every formula, returning their cells in order.
    fn work_out(&mut self) -> Vec<CellKey> {
        let book = self;
        // Top to bottom, so a chain that runs down the sheet (a running total) is worked out a
        // cell at a time instead of all at once from its last cell.
        let mut keys: Vec<CellKey> = book.formulas.keys().copied().collect();
        keys.sort_unstable();
        for &key in &keys {
            // A chain deeper than MAX_DEPTH stops where it gets too deep; that cell is worked out
            // first, then the chain again, which now finds it done.
            let mut stack = vec![key];
            while let Some(&(s, r, c)) = stack.last() {
                book.current = vec![s];
                book.value(s, r, c);
                match book.deep.take() {
                    // A loop that long: it's found in `reads` all the same.
                    Some(d) if stack.contains(&d) => {
                        book.done.insert(d, V::Num(0.0));
                    }
                    Some(d) => stack.push(d),
                    None => {
                        stack.pop();
                    }
                }
            }
        }
        keys
    }
}

/// The workbook as formulas see it while they're worked out.
struct Workbook<'a> {
    sheets: &'a [Sheet],
    names: Vec<DefinedName>,
    date_system: DateSystem,
    now: Option<f64>,
    /// Shared, so working one out doesn't copy its tree: a copy recurses, and a sum of a
    /// hundred thousand terms overflowed the stack.
    formulas: HashMap<CellKey, std::rc::Rc<Expr>>,
    done: HashMap<CellKey, V>,
    working: HashSet<CellKey>,
    /// The formulas being worked out, outermost first.
    path: Vec<CellKey>,
    /// The formulas each formula reads, as found while working them out.
    reads: HashMap<CellKey, HashSet<CellKey>>,
    /// The sheet of the formula being worked out, innermost last.
    current: Vec<usize>,
    /// The cell where a chain of formulas got too deep to follow; nothing worked out on the
    /// way to it is kept.
    deep: Option<CellKey>,
    /// Each sheet's last used row and column, once asked for.
    extents: Vec<std::cell::OnceCell<Option<(u32, u32)>>>,
}

/// How many formulas deep one is worked out before [`Workbook::deep`] takes over. Each level
/// is a few stack frames, and a test thread or a browser has a small stack.
const MAX_DEPTH: usize = 200;

impl Workbook<'_> {
    fn sheet_index(&self, name: Option<&str>) -> Option<usize> {
        match name {
            None => self.current.last().copied(),
            Some(n) => self.sheets.iter().position(|s| s.name.eq_ignore_ascii_case(n)),
        }
    }

    fn value(&mut self, s: usize, r: u32, c: u32) -> V {
        let key = (s, r, c);
        let Some(expr) = self.formulas.get(&key).cloned() else {
            return V::from_value(self.sheets[s].value(CellRef::new(r, c)));
        };
        if let Some(&from) = self.path.last() {
            self.reads.entry(from).or_default().insert(key);
        }
        if let Some(v) = self.done.get(&key) {
            return v.clone();
        }
        if self.deep.is_some() {
            return V::Empty;
        }
        if !self.working.insert(key) {
            // A circular reference, found in `reads` afterwards.
            return V::Num(0.0);
        }
        if self.working.len() > MAX_DEPTH || eval::depth() >= 64 {
            self.working.remove(&key);
            self.deep = Some(key);
            return V::Empty;
        }
        self.path.push(key);
        self.current.push(s);
        let v = eval::eval(&expr, self);
        self.current.pop();
        self.path.pop();
        self.working.remove(&key);
        if self.deep.is_some() {
            return V::Empty;
        }
        // A formula's result is a single value; an array formula shows its first.
        let v = match v {
            V::Range(rows) => rows.into_iter().next().and_then(|r| r.into_iter().next()).unwrap_or(V::Empty),
            v => v,
        };
        self.done.insert(key, v.clone());
        v
    }
}

impl Book for Workbook<'_> {
    fn cell(&mut self, sheet: Option<&str>, row: u32, col: u32) -> V {
        match self.sheet_index(sheet) {
            Some(s) => self.value(s, row, col),
            None => V::Err("#REF!"),
        }
    }

    fn extent(&self, sheet: Option<&str>) -> Option<(u32, u32)> {
        let s = self.sheet_index(sheet)?;
        // The sheets don't change while formulas are worked out, so each is measured once.
        *self.extents.get(s)?.get_or_init(|| self.sheets[s].used_range().map(|u| (u.end.row, u.end.col)))
    }

    fn name(&self, name: &str) -> Option<Expr> {
        let here = self.current.last().copied();
        // A sheet's own name wins over the workbook's.
        let found = self
            .names
            .iter()
            .find(|n| n.name.eq_ignore_ascii_case(name) && n.sheet.is_some() && n.sheet == here)
            .or_else(|| self.names.iter().find(|n| n.name.eq_ignore_ascii_case(name) && n.sheet.is_none()))?;
        formula::parse(&found.formula).ok()
    }

    fn date_system(&self) -> DateSystem {
        self.date_system
    }

    fn now(&self) -> Option<f64> {
        self.now
    }
}

// ---- the workbook ----

/// The workbook's defined names.
pub fn names(doc: &Document) -> &[DefinedName] {
    excel(doc).map(|x| x.names.as_slice()).unwrap_or(&[])
}

/// A column's width in characters of the default font, if the sheet sets one.
pub fn column_width(doc: &Document, sheet_index: usize, col: u32) -> Option<f64> {
    let cols = sheet(doc, sheet_index)?.origin.get()?.child(ns::X, "cols")?;
    cols.children_named(ns::X, "col")
        .find(|c| {
            let n = |k: &str| c.attr(k).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
            (n("min")..=n("max")).contains(&(col + 1))
        })
        .and_then(|c| c.attr("width"))
        .and_then(|w| w.parse().ok())
}

/// The children of `worksheet` before `cols`, for putting a new `cols` in its place.
const BEFORE_COLS: &[&[&str]] = &[&["sheetPr"], &["dimension"], &["sheetViews"], &["sheetFormatPr"], &["cols"], &["sheetData"]];

/// Sets a column's width (in characters), or clears it for `None`.
pub fn set_column_width(doc: &mut Document, sheet_index: usize, col: u32, width: Option<f64>) {
    let Some(s) = sheet_mut(doc, sheet_index) else { return };
    let mut root = s.origin.get().cloned().unwrap_or_else(|| {
        let mut ws = Element::new(ns::X, "worksheet");
        ws.declare(None, ns::X);
        ws.push(Element::new(ns::X, "sheetData"));
        ws
    });
    let n = col + 1;
    let mut cols: Vec<Element> = root.child(ns::X, "cols").map(|c| c.children_named(ns::X, "col").cloned().collect()).unwrap_or_default();
    // Split whatever range covers the column, so it can have its own entry.
    let mut kept = Vec::new();
    let mut covering = None;
    for c in cols.drain(..) {
        let get = |k: &str| c.attr(k).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
        let (min, max) = (get("min"), get("max"));
        if !(min..=max).contains(&n) {
            kept.push(c);
            continue;
        }
        covering = Some(c.clone());
        if min < n {
            kept.push(c.clone().with_attr("", "max", (n - 1).to_string()));
        }
        if max > n {
            kept.push(c.clone().with_attr("", "min", (n + 1).to_string()));
        }
    }
    let mut entry = covering.unwrap_or_else(|| Element::new(ns::X, "col"));
    entry.set_attr("", "min", n.to_string());
    entry.set_attr("", "max", n.to_string());
    if let Some(w) = width {
        entry.set_attr("", "width", office_core::model::format_number(w));
        entry.set_attr("", "customWidth", "1");
    } else {
        entry.remove_attr("", "width");
        entry.remove_attr("", "customWidth");
    }
    if width.is_some() || entry.attrs.iter().any(|a| !matches!(a.local.as_str(), "min" | "max")) {
        kept.push(entry);
    }
    kept.sort_by_key(|c| c.attr("min").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0));
    root.remove_children(ns::X, "cols");
    if !kept.is_empty() {
        let mut el = Element::new(ns::X, "cols");
        el.children = kept.into_iter().map(Node::Element).collect();
        root.place(el, BEFORE_COLS);
    }
    s.origin = Origin::new(root);
}

/// A row's height in points, if it has one of its own.
pub fn row_height(doc: &Document, sheet_index: usize, row: u32) -> Option<f64> {
    let s = sheet(doc, sheet_index)?;
    let r = s.rows.iter().find(|r| r.index == row)?;
    r.origin.get()?.attr("ht")?.parse().ok()
}

/// Sets a row's height in points, or clears it for `None`. The row is made if it's empty.
pub fn set_row_height(doc: &mut Document, sheet_index: usize, row: u32, height: Option<f64>) {
    let Some(s) = sheet_mut(doc, sheet_index) else { return };
    let i = match s.rows.binary_search_by_key(&row, |r| r.index) {
        Ok(i) => i,
        Err(i) => {
            s.rows.insert(i, office_core::model::SheetRow { index: row, ..Default::default() });
            i
        }
    };
    let r = &mut s.rows[i];
    let mut el = r.origin.get().cloned().unwrap_or_else(|| Element::new(ns::X, "row"));
    match height {
        Some(h) => {
            el.set_attr("", "ht", office_core::model::format_number(h));
            el.set_attr("", "customHeight", "1");
        }
        None => {
            el.remove_attr("", "ht");
            el.remove_attr("", "customHeight");
        }
    }
    r.origin = Origin::new(el);
}

#[cfg(test)]
mod tests;
