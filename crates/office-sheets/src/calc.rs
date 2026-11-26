//! Persistent formula AST cache and reverse dependency graph for incremental recalculation.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use office_core::model::{Body, CellRef, Document};

use crate::formula::{self, Expr, Op, Reference};
use crate::tables::Table;
use crate::{excel, CellKey, SheetFormulas};

#[derive(Clone, Debug, Default)]
struct FormulaDeps {
    cells: Vec<CellKey>,
    /// (sheet, r0, r1, c0, c1)
    ranges: Vec<(usize, u32, u32, u32, u32)>,
    open_sheets: Vec<usize>,
    volatile: bool,
    dynamic_book: bool,
}

pub(crate) type ColRangeEntry = (u32, u32, CellKey);
pub(crate) type WideRangeEntry = (u32, u32, u32, u32, CellKey);
pub(crate) type NameAstMap = HashMap<(Option<usize>, String), Rc<Expr>>;

#[derive(Default)]
pub(crate) struct CalcGraph {
    pub formulas: HashMap<CellKey, Rc<Expr>>,
    pub formula_keys: Vec<CellKey>,
    sorted_dirty: bool,
    pub tables: Vec<Table>,
    pub name_asts: NameAstMap,
    deps: HashMap<CellKey, FormulaDeps>,
    rev_cells: HashMap<CellKey, HashSet<CellKey>>,
    /// Narrow range index keyed by (sheet, col) for <= 16 columns.
    rev_col_ranges: HashMap<(usize, u32), Vec<ColRangeEntry>>,
    rev_wide_ranges: Vec<Vec<WideRangeEntry>>,
    rev_open_sheets: Vec<HashSet<CellKey>>,
    pub volatiles: HashSet<CellKey>,
    pub dynamic_count: usize,
}

impl CalcGraph {
    pub fn build(doc: &Document) -> Self {
        let Body::Workbook { sheets } = &doc.body else {
            return Self::default();
        };
        let sheet_count = sheets.len();
        let sheet_names: Vec<&str> = sheets.iter().map(|s| s.name.as_str()).collect();
        let tables = doc.package.as_ref().map(|pkg| Table::read(pkg, sheets)).unwrap_or_default();
        let mut name_asts = HashMap::new();
        if let Some(x) = excel(doc) {
            for dn in &x.names {
                if let Ok(expr) = formula::parse(&dn.formula) {
                    name_asts.insert((dn.sheet, dn.name.to_ascii_lowercase()), Rc::new(expr));
                }
            }
        }
        let mut formulas = HashMap::new();
        let mut formula_keys = Vec::new();
        for (si, s) in sheets.iter().enumerate() {
            let mut reader = SheetFormulas::new(s);
            for row in &s.rows {
                for c in &row.cells {
                    if let Some(text) = reader.read(CellRef::new(row.index, c.col), c) {
                        let expr = Rc::new(formula::parse(&text).unwrap_or(Expr::Error("#NAME?".into())));
                        let key = (si, row.index, c.col);
                        formulas.insert(key, expr);
                        formula_keys.push(key);
                    }
                }
            }
        }
        formula_keys.sort_unstable();
        let mut graph = Self {
            formulas,
            formula_keys,
            sorted_dirty: false,
            tables,
            name_asts,
            deps: HashMap::new(),
            rev_cells: HashMap::new(),
            rev_col_ranges: HashMap::new(),
            rev_wide_ranges: vec![Vec::new(); sheet_count],
            rev_open_sheets: vec![HashSet::new(); sheet_count],
            volatiles: HashSet::new(),
            dynamic_count: 0,
        };
        let keys: Vec<CellKey> = graph.formula_keys.clone();
        for key in keys {
            if let Some(expr) = graph.formulas.get(&key).cloned() {
                let d = extract_deps(&expr, key, &sheet_names, &graph.tables, &graph.name_asts);
                graph.insert_deps(key, d);
            }
        }
        graph
    }

    pub fn update_cells(&mut self, doc: &Document, sheet: usize, cells: &[CellRef]) {
        let Body::Workbook { sheets } = &doc.body else { return };
        let Some(s) = sheets.get(sheet) else { return };
        let sheet_names: Vec<&str> = sheets.iter().map(|sh| sh.name.as_str()).collect();
        let mut reader = SheetFormulas::new(s);
        for &at in cells {
            let key = (sheet, at.row, at.col);
            self.remove_deps(key);
            if let Some(cell) = s.cell(at) {
                if let Some(text) = reader.read(at, cell) {
                    let expr = Rc::new(formula::parse(&text).unwrap_or(Expr::Error("#NAME?".into())));
                    if self.formulas.insert(key, expr.clone()).is_none() {
                        self.formula_keys.push(key);
                        self.sorted_dirty = true;
                    }
                    let d = extract_deps(&expr, key, &sheet_names, &self.tables, &self.name_asts);
                    self.insert_deps(key, d);
                    continue;
                }
            }
            if self.formulas.remove(&key).is_some() {
                self.sorted_dirty = true;
            }
        }
        if self.sorted_dirty {
            self.formula_keys.retain(|k| self.formulas.contains_key(k));
            self.formula_keys.sort_unstable();
            self.sorted_dirty = false;
        }
    }

    pub fn dirty_closure(&self, doc: &Document, roots: &[CellKey], prev_circular: &[(usize, CellRef)]) -> (Vec<CellKey>, HashSet<CellKey>) {
        if self.dynamic_count > 0 {
            let set: HashSet<CellKey> = self.formula_keys.iter().copied().collect();
            return (self.formula_keys.clone(), set);
        }
        let Body::Workbook { sheets } = &doc.body else {
            return (Vec::new(), HashSet::new());
        };
        let mut dirty_formulas = HashSet::new();
        let mut queue: Vec<CellKey> = Vec::with_capacity(roots.len() + self.volatiles.len() + prev_circular.len());
        let mut queued = HashSet::with_capacity(roots.len() + self.volatiles.len() + prev_circular.len());
        let mut open_done = vec![false; sheets.len()];

        for &k in roots {
            if self.formulas.contains_key(&k) {
                dirty_formulas.insert(k);
            }
            if queued.insert(k) {
                queue.push(k);
            }
        }
        for &k in &self.volatiles {
            dirty_formulas.insert(k);
            if queued.insert(k) {
                queue.push(k);
            }
        }
        for &(s, c) in prev_circular {
            let k = (s, c.row, c.col);
            if self.formulas.contains_key(&k) {
                dirty_formulas.insert(k);
            }
            if queued.insert(k) {
                queue.push(k);
            }
        }

        while let Some(k @ (s, r, c)) = queue.pop() {
            if let Some(deps) = self.rev_cells.get(&k) {
                for &dep in deps {
                    if dirty_formulas.insert(dep) && queued.insert(dep) {
                        queue.push(dep);
                    }
                }
            }
            if let Some(col_ranges) = self.rev_col_ranges.get(&(s, c)) {
                for &(r0, r1, dep) in col_ranges {
                    if r >= r0 && r <= r1 && dirty_formulas.insert(dep) && queued.insert(dep) {
                        queue.push(dep);
                    }
                }
            }
            if let Some(wide) = self.rev_wide_ranges.get(s) {
                for &(r0, r1, c0, c1, dep) in wide {
                    if r >= r0 && r <= r1 && c >= c0 && c <= c1 && dirty_formulas.insert(dep) && queued.insert(dep) {
                        queue.push(dep);
                    }
                }
            }
            if let Some(done) = open_done.get_mut(s) {
                if !*done {
                    *done = true;
                    if let Some(open) = self.rev_open_sheets.get(s) {
                        for &dep in open {
                            if dirty_formulas.insert(dep) && queued.insert(dep) {
                                queue.push(dep);
                            }
                        }
                    }
                }
            }
        }

        let mut keys: Vec<CellKey> = dirty_formulas.iter().copied().collect();
        keys.sort_unstable();
        (keys, dirty_formulas)
    }

    fn insert_deps(&mut self, key: CellKey, deps: FormulaDeps) {
        for &target in &deps.cells {
            self.rev_cells.entry(target).or_default().insert(key);
        }
        for &(s, r0, r1, c0, c1) in &deps.ranges {
            if c1 - c0 <= 16 {
                for c in c0..=c1 {
                    self.rev_col_ranges.entry((s, c)).or_default().push((r0, r1, key));
                }
            } else if let Some(wide) = self.rev_wide_ranges.get_mut(s) {
                wide.push((r0, r1, c0, c1, key));
            }
        }
        for &s in &deps.open_sheets {
            if let Some(open) = self.rev_open_sheets.get_mut(s) {
                open.insert(key);
            }
        }
        if deps.volatile {
            self.volatiles.insert(key);
        }
        if deps.dynamic_book {
            self.dynamic_count += 1;
        }
        self.deps.insert(key, deps);
    }

    fn remove_deps(&mut self, key: CellKey) {
        let Some(old) = self.deps.remove(&key) else { return };
        for target in old.cells {
            if let Some(set) = self.rev_cells.get_mut(&target) {
                set.remove(&key);
                if set.is_empty() {
                    self.rev_cells.remove(&target);
                }
            }
        }
        for (s, _, _, c0, c1) in old.ranges {
            if c1 - c0 <= 16 {
                for c in c0..=c1 {
                    if let Some(vec) = self.rev_col_ranges.get_mut(&(s, c)) {
                        vec.retain(|&(_, _, dep)| dep != key);
                    }
                }
            } else if let Some(wide) = self.rev_wide_ranges.get_mut(s) {
                wide.retain(|&(_, _, _, _, dep)| dep != key);
            }
        }
        for s in old.open_sheets {
            if let Some(open) = self.rev_open_sheets.get_mut(s) {
                open.remove(&key);
            }
        }
        if old.volatile {
            self.volatiles.remove(&key);
        }
        if old.dynamic_book {
            self.dynamic_count = self.dynamic_count.saturating_sub(1);
        }
    }
}

fn find_sheet(names: &[&str], sheet: Option<&str>, current: usize) -> Option<usize> {
    match sheet {
        None => Some(current),
        Some(n) => names.iter().position(|s| s.eq_ignore_ascii_case(n)),
    }
}

fn push_ref(r: &Reference, current: usize, sheets: &[&str], out: &mut FormulaDeps) {
    let Some(si) = find_sheet(sheets, r.sheet.as_deref(), current) else { return };
    let Some(end) = r.end else {
        if let (Some(row), Some(col)) = (r.start.row, r.start.col) {
            out.cells.push((si, row, col));
        } else {
            out.open_sheets.push(si);
        }
        return;
    };
    if let (Some(r0), Some(c0), Some(r1), Some(c1)) = (r.start.row, r.start.col, end.row, end.col) {
        let (r0, r1) = (r0.min(r1), r0.max(r1));
        let (c0, c1) = (c0.min(c1), c0.max(c1));
        if r0 == r1 && c0 == c1 {
            out.cells.push((si, r0, c0));
        } else {
            out.ranges.push((si, r0, r1, c0, c1));
        }
    } else {
        out.open_sheets.push(si);
    }
}

fn extract_deps(
    root: &Expr,
    origin: CellKey,
    sheets: &[&str],
    tables: &[Table],
    name_asts: &HashMap<(Option<usize>, String), Rc<Expr>>,
) -> FormulaDeps {
    let mut out = FormulaDeps::default();
    let mut work: Vec<(&Expr, usize)> = vec![(root, origin.0)];
    let mut visited_names: HashSet<(usize, String)> = HashSet::new();
    while let Some((expr, cur_sheet)) = work.pop() {
        match expr {
            Expr::Number(_) | Expr::Text(_) | Expr::Bool(_) | Expr::Error(_) | Expr::Missing => {}
            Expr::Ref(r) => push_ref(r, cur_sheet, sheets, &mut out),
            Expr::Table(tr) => {
                let here = Some((origin.0, CellRef::new(origin.1, origin.2)));
                if let Some(Ok(refs)) = Table::find(tables, tr.table.as_deref(), here).map(|t| t.resolve(tr, here)) {
                    for r in refs {
                        push_ref(&r, cur_sheet, sheets, &mut out);
                    }
                }
            }
            Expr::Name(name) => {
                let low = name.to_ascii_lowercase();
                if let Some((ns_sheet, resolved)) = name_asts
                    .get(&(Some(cur_sheet), low.clone()))
                    .map(|e| (cur_sheet, e))
                    .or_else(|| name_asts.get(&(None, low.clone())).map(|e| (cur_sheet, e)))
                {
                    if visited_names.insert((ns_sheet, low)) {
                        work.push((resolved, ns_sheet));
                    }
                }
            }
            Expr::Neg(a) | Expr::Percent(a) => work.push((a, cur_sheet)),
            Expr::Binary(op, a, b) => {
                if matches!(op, Op::Range | Op::Intersect | Op::Union) {
                    out.open_sheets.push(cur_sheet);
                }
                work.push((a, cur_sheet));
                work.push((b, cur_sheet));
            }
            Expr::Call(name, args) => {
                if matches!(name.as_str(), "NOW" | "TODAY" | "RAND" | "RANDBETWEEN") {
                    out.volatile = true;
                }
                if matches!(name.as_str(), "INDIRECT" | "OFFSET") {
                    out.dynamic_book = true;
                }
                for a in args {
                    work.push((a, cur_sheet));
                }
            }
            Expr::Array(rows) => {
                for row in rows {
                    for item in row {
                        work.push((item, cur_sheet));
                    }
                }
            }
        }
    }
    out.cells.sort_unstable();
    out.cells.dedup();
    out.open_sheets.sort_unstable();
    out.open_sheets.dedup();
    out
}
