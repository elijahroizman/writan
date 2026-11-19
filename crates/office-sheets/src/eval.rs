//! Working formulas out: Excel's values and how they turn into one another, references
//! through a [`Book`], and the functions people use most.

use std::collections::{HashMap, HashSet};

use office_core::model::{format_number, Value};

use crate::dates::{self, DateSystem};
use crate::formula::{Expr, Op, RefPart, Reference, TableReference};

/// A value while a formula is worked out.
#[derive(Clone, Debug, PartialEq)]
pub enum V {
    Num(f64),
    Text(String),
    Bool(bool),
    Err(&'static str),
    Empty,
    /// A range's values, rows of columns.
    Range(Vec<Vec<V>>),
}

impl V {
    pub fn from_value(v: &Value) -> V {
        match v {
            Value::Empty => V::Empty,
            Value::Number(n) => V::Num(*n),
            Value::Text(_) | Value::Rich(_) => V::Text(v.display()),
            Value::Bool(b) => V::Bool(*b),
            Value::Error(e) => V::Err(error_name(e)),
        }
    }

    /// The value a cell holds after its formula: a range's top-left value, as Excel shows it
    /// without spilling.
    pub fn to_value(&self) -> Value {
        match self {
            V::Num(n) => Value::Number(*n),
            V::Text(t) => Value::Text(t.clone()),
            V::Bool(b) => Value::Bool(*b),
            V::Err(e) => Value::Error(e.to_string()),
            // A formula that comes to an empty cell shows 0.
            V::Empty => Value::Number(0.0),
            V::Range(r) => r.first().and_then(|row| row.first()).map(V::to_value).unwrap_or(Value::Number(0.0)),
        }
    }
}

const ERRORS: [&str; 8] = ["#NULL!", "#DIV/0!", "#VALUE!", "#REF!", "#NAME?", "#NUM!", "#N/A", "#GETTING_DATA"];

fn error_name(e: &str) -> &'static str {
    ERRORS.iter().find(|x| x.eq_ignore_ascii_case(e)).copied().unwrap_or("#VALUE!")
}

/// What formulas can reach.
pub trait Book {
    /// A cell's value, working its formula out first if it has one. `sheet` is `None` for the
    /// formula's own sheet.
    fn cell(&mut self, sheet: Option<&str>, row: u32, col: u32) -> V;
    /// The last row and column a sheet uses, to bound whole-column and whole-row ranges.
    fn extent(&self, sheet: Option<&str>) -> Option<(u32, u32)>;
    /// A defined name's formula, from the formula's own sheet's point of view.
    fn name(&self, name: &str) -> Option<Expr>;
    fn table(&self, _reference: &TableReference) -> Option<Result<Vec<Reference>, &'static str>> {
        None
    }
    fn date_system(&self) -> DateSystem;
    /// Now, as a serial number, for NOW() and TODAY(); `None` makes them #N/A.
    fn now(&self) -> Option<f64>;
}

type R = Result<V, &'static str>;

/// Evaluation state for names. Cell evaluation can re-enter through Book, so the depth
/// guard is shared by nested evaluators on this thread as well.
struct Context<'a> {
    book: &'a mut dyn Book,
    names: HashSet<String>,
}

impl Book for Context<'_> {
    fn cell(&mut self, sheet: Option<&str>, row: u32, col: u32) -> V {
        self.book.cell(sheet, row, col)
    }
    fn extent(&self, sheet: Option<&str>) -> Option<(u32, u32)> {
        self.book.extent(sheet)
    }
    fn name(&self, name: &str) -> Option<Expr> {
        self.book.name(name)
    }
    fn table(&self, reference: &TableReference) -> Option<Result<Vec<Reference>, &'static str>> {
        self.book.table(reference)
    }
    fn date_system(&self) -> DateSystem {
        self.book.date_system()
    }
    fn now(&self) -> Option<f64> {
        self.book.now()
    }
}

thread_local! { static DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
pub(crate) fn depth() -> usize {
    DEPTH.with(std::cell::Cell::get)
}
struct Depth;
impl Depth {
    fn enter() -> Result<Self, &'static str> {
        if DEPTH.with(|d| {
            if d.get() >= 128 {
                false
            } else {
                d.set(d.get() + 1);
                true
            }
        }) {
            Ok(Self)
        } else {
            Err("#REF!")
        }
    }
}
impl Drop for Depth {
    fn drop(&mut self) {
        DEPTH.with(|d| d.set(d.get() - 1));
    }
}

/// Works out `expr`.
pub fn eval(expr: &Expr, book: &mut dyn Book) -> V {
    eval_inner(expr, &mut Context { book, names: HashSet::new() }).unwrap_or_else(V::Err)
}

fn eval_inner(expr: &Expr, book: &mut Context<'_>) -> R {
    let _depth = Depth::enter()?;
    let value = eval_unchecked(expr, book)?;
    match value {
        V::Num(n) => finite(n),
        other => Ok(other),
    }
}

fn eval_unchecked(expr: &Expr, book: &mut Context<'_>) -> R {
    Ok(match expr {
        Expr::Number(n) => V::Num(*n),
        Expr::Text(t) => V::Text(t.clone()),
        Expr::Bool(b) => V::Bool(*b),
        Expr::Error(e) => V::Err(error_name(e)),
        Expr::Missing => V::Empty,
        Expr::Ref(r) => reference(r, book)?,
        Expr::Name(n) => {
            let key = n.to_uppercase();
            if !book.names.insert(key.clone()) {
                return Err("#REF!");
            }
            let value = match book.name(n) {
                Some(e) => eval_inner(&e, book),
                None => table_name(n, book).ok_or("#NAME?").and_then(|r| range_values(r?, book)),
            };
            book.names.remove(&key);
            value?
        }
        Expr::Table(_) | Expr::Binary(Op::Range | Op::Intersect | Op::Union, _, _) => range_values(references(expr, book)?, book)?,
        Expr::Neg(e) => V::Num(-num(&scalar(eval_inner(e, book)?))?),
        Expr::Percent(e) => V::Num(num(&scalar(eval_inner(e, book)?))? / 100.0),
        Expr::Binary(op, a, b) => eval_chain(*op, a, b, book)?,
        Expr::Array(rows) => {
            let width = rows.first().map_or(0, Vec::len);
            if rows.iter().any(|r| r.len() != width) {
                return Err("#VALUE!");
            }
            V::Range(rows.iter().map(|r| r.iter().map(|e| eval_inner(e, book).unwrap_or_else(V::Err)).collect()).collect())
        }
        Expr::Call(name, args) => call(name, args, book)?,
    })
}

/// `A1+A2+...+A130` is left-nested, and each `eval_inner` used to take one depth
/// slot, so the sum came out as `#REF!`. A chain is not nested: walk its left
/// spine in a loop. The leftmost operand still runs first, and an error there
/// still skips every operand to its right. Range, intersection and union stay
/// on the recursive path.
fn eval_chain(op: Op, left: &Expr, right: &Expr, book: &mut Context<'_>) -> R {
    let mut spine = vec![(op, right)];
    let mut left = left;
    while let Expr::Binary(next, lower, rhs) = left {
        if matches!(next, Op::Range | Op::Intersect | Op::Union) {
            break;
        }
        spine.push((*next, rhs.as_ref()));
        left = lower.as_ref();
    }
    let mut acc = scalar(eval_inner(left, book)?);
    for (op, rhs) in spine.into_iter().rev() {
        acc = binary(op, &acc, &scalar(eval_inner(rhs, book)?))?;
    }
    Ok(acc)
}

fn reference(r: &Reference, book: &mut Context<'_>) -> R {
    let sheet = r.sheet.as_deref();
    let Some(end) = r.end else {
        let (row, col) = (r.start.row.ok_or("#REF!")?, r.start.col.ok_or("#REF!")?);
        return Ok(book.cell(sheet, row, col));
    };
    // A whole column (or row) runs to the last row (or column) the sheet uses; only those need
    // the sheet's extent.
    let open = [r.start.row, r.start.col, end.row, end.col].iter().any(Option::is_none);
    let (last_row, last_col) = if open { book.extent(sheet).unwrap_or((0, 0)) } else { (0, 0) };
    let span = |a: Option<u32>, b: Option<u32>, last: u32| match (a, b) {
        (Some(a), Some(b)) => (a.min(b), a.max(b)),
        _ => (0, last),
    };
    let bound = |a: &RefPart, b: &RefPart| (span(a.row, b.row, last_row), span(a.col, b.col, last_col));
    let ((r0, r1), (c0, c1)) = bound(&r.start, &end);
    if (r1 - r0 + 1) as u64 * (c1 - c0 + 1) as u64 > 10_000_000 {
        return Err("#NUM!");
    }
    Ok(V::Range((r0..=r1).map(|row| (c0..=c1).map(|col| book.cell(sheet, row, col)).collect()).collect()))
}

fn range_values(refs: Vec<Reference>, book: &mut Context<'_>) -> R {
    let mut rows = Vec::new();
    for r in refs {
        match reference(&r, book)? {
            V::Range(r) => rows.extend(r),
            v => rows.push(vec![v]),
        }
    }
    Ok(V::Range(rows))
}

/// A single value from what may be a range: its top-left one.
fn scalar(v: V) -> V {
    match v {
        V::Range(r) => r.into_iter().next().and_then(|row| row.into_iter().next()).unwrap_or(V::Empty),
        v => v,
    }
}

/// A value as a number, the way arithmetic takes it: text that reads as a number is one,
/// TRUE is 1, an empty cell 0.
fn num(v: &V) -> Result<f64, &'static str> {
    match v {
        V::Num(n) => Ok(*n),
        V::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
        V::Empty => Ok(0.0),
        V::Text(t) => t.trim().parse::<f64>().ok().or_else(|| percent_text(t)).ok_or("#VALUE!"),
        V::Err(e) => Err(e),
        V::Range(_) => num(&scalar(v.clone())),
    }
}

fn percent_text(t: &str) -> Option<f64> {
    t.trim().strip_suffix('%')?.trim().parse::<f64>().ok().map(|n| n / 100.0)
}

pub fn text(v: &V) -> Result<String, &'static str> {
    match v {
        V::Num(n) => Ok(crate::numfmt::general(*n)),
        V::Text(t) => Ok(t.clone()),
        V::Bool(b) => Ok(if *b { "TRUE" } else { "FALSE" }.into()),
        V::Empty => Ok(String::new()),
        V::Err(e) => Err(e),
        V::Range(_) => text(&scalar(v.clone())),
    }
}

fn truthy(v: &V) -> Result<bool, &'static str> {
    match v {
        V::Bool(b) => Ok(*b),
        V::Num(n) => Ok(*n != 0.0),
        V::Empty => Ok(false),
        V::Text(t) if t.eq_ignore_ascii_case("TRUE") => Ok(true),
        V::Text(t) if t.eq_ignore_ascii_case("FALSE") => Ok(false),
        V::Text(_) => Err("#VALUE!"),
        V::Err(e) => Err(e),
        V::Range(_) => truthy(&scalar(v.clone())),
    }
}

/// Excel's order for comparing: numbers, then text (ignoring case), then booleans.
fn compare(a: &V, b: &V) -> Result<std::cmp::Ordering, &'static str> {
    use std::cmp::Ordering;
    let rank = |v: &V| match v {
        V::Num(_) | V::Empty => 0,
        V::Text(_) => 1,
        V::Bool(_) => 2,
        _ => 3,
    };
    if let V::Err(e) = a {
        return Err(e);
    }
    if let V::Err(e) = b {
        return Err(e);
    }
    // An empty cell compares as whatever it's compared with: 0, "" or FALSE.
    let (a, b) = match (a, b) {
        (V::Empty, V::Text(_)) => (&V::Text(String::new()), b),
        (V::Text(_), V::Empty) => (a, &V::Text(String::new())),
        (V::Empty, V::Bool(_)) => (&V::Bool(false), b),
        (V::Bool(_), V::Empty) => (a, &V::Bool(false)),
        _ => (a, b),
    };
    Ok(match (a, b) {
        (V::Text(x), V::Text(y)) => x.to_lowercase().cmp(&y.to_lowercase()),
        (V::Bool(x), V::Bool(y)) => x.cmp(y),
        _ if rank(a) != rank(b) => rank(a).cmp(&rank(b)),
        _ => num(a)?.partial_cmp(&num(b)?).unwrap_or(Ordering::Equal),
    })
}

fn binary(op: Op, a: &V, b: &V) -> R {
    use std::cmp::Ordering::*;
    Ok(match op {
        Op::Concat => V::Text(text(a)? + &text(b)?),
        Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge => {
            let o = compare(a, b)?;
            V::Bool(match op {
                Op::Eq => o == Equal,
                Op::Ne => o != Equal,
                Op::Lt => o == Less,
                Op::Le => o != Greater,
                Op::Gt => o == Greater,
                _ => o != Less,
            })
        }
        _ => {
            let (x, y) = (num(a)?, num(b)?);
            let n = match op {
                Op::Add => x + y,
                Op::Sub => x - y,
                Op::Mul => x * y,
                Op::Div if y == 0.0 => return Err("#DIV/0!"),
                Op::Div => x / y,
                _ if x == 0.0 && y < 0.0 => return Err("#DIV/0!"),
                _ => x.powf(y),
            };
            finite(n)?
        }
    })
}

fn finite(n: f64) -> R {
    if n.is_finite() {
        Ok(V::Num(n))
    } else {
        Err("#NUM!")
    }
}

fn table_name(name: &str, book: &Context<'_>) -> Option<Result<Vec<Reference>, &'static str>> {
    book.table(&TableReference { table: Some(name.to_string()), ..TableReference::default() })
}

fn is_reference(expr: &Expr, book: &Context<'_>, depth: usize) -> bool {
    match expr {
        Expr::Ref(_) | Expr::Table(_) | Expr::Binary(Op::Range | Op::Intersect | Op::Union, _, _) => true,
        Expr::Name(n) if depth < 128 => book.name(n).map_or_else(|| table_name(n, book).is_some(), |e| is_reference(&e, book, depth + 1)),
        _ => false,
    }
}

/// Resolve reference operators before fetching cells, so intersection never materializes
/// the non-overlapping parts of either operand.
fn references(expr: &Expr, book: &mut Context<'_>) -> Result<Vec<Reference>, &'static str> {
    let _depth = Depth::enter()?;
    match expr {
        Expr::Ref(r) => Ok(vec![r.clone()]),
        Expr::Table(r) => book.table(r).ok_or(if r.table.is_some() { "#NAME?" } else { "#VALUE!" })?,
        Expr::Name(n) => {
            let key = n.to_uppercase();
            if !book.names.insert(key.clone()) {
                return Err("#REF!");
            }
            let result = match book.name(n) {
                Some(e) => references(&e, book),
                None => table_name(n, book).ok_or("#NAME?").and_then(|r| r),
            };
            book.names.remove(&key);
            result
        }
        Expr::Binary(op @ (Op::Range | Op::Intersect | Op::Union), a, b) => {
            let mut a = references(a, book)?;
            let b = references(b, book)?;
            if *op == Op::Union {
                a.extend(b);
                return Ok(a);
            }
            let bounds = |r: &Reference| {
                let end = r.end.unwrap_or(r.start);
                let extent = if [r.start.row, r.start.col, end.row, end.col].iter().any(Option::is_none) {
                    book.extent(r.sheet.as_deref()).unwrap_or((0, 0))
                } else {
                    (0, 0)
                };
                (
                    r.start.row.unwrap_or(0).min(end.row.unwrap_or(0)),
                    r.start.row.unwrap_or(extent.0).max(end.row.unwrap_or(extent.0)),
                    r.start.col.unwrap_or(0).min(end.col.unwrap_or(0)),
                    r.start.col.unwrap_or(extent.1).max(end.col.unwrap_or(extent.1)),
                )
            };
            let mut out = Vec::new();
            for x in &a {
                for y in &b {
                    if x.sheet != y.sheet {
                        continue;
                    }
                    let (xr0, xr1, xc0, xc1) = bounds(x);
                    let (yr0, yr1, yc0, yc1) = bounds(y);
                    let (r0, r1, c0, c1) = if *op == Op::Intersect {
                        (xr0.max(yr0), xr1.min(yr1), xc0.max(yc0), xc1.min(yc1))
                    } else {
                        (xr0.min(yr0), xr1.max(yr1), xc0.min(yc0), xc1.max(yc1))
                    };
                    if r0 <= r1 && c0 <= c1 {
                        let part = |r, c| RefPart { row: Some(r), col: Some(c), row_abs: false, col_abs: false };
                        out.push(Reference { sheet: x.sheet.clone(), start: part(r0, c0), end: Some(part(r1, c1)) });
                    }
                }
            }
            if out.is_empty() {
                Err("#NULL!")
            } else {
                Ok(out)
            }
        }
        _ => Err("#VALUE!"),
    }
}

/// Every value in the arguments, ranges flattened, with whether each came from a range (range
/// cells and direct arguments count differently in SUM, COUNT and the like).
fn flatten(args: &[Expr], book: &mut Context<'_>) -> Result<Vec<(V, bool)>, &'static str> {
    let mut out = Vec::new();
    for a in args {
        match eval_inner(a, book)? {
            V::Range(rows) => out.extend(rows.into_iter().flatten().map(|v| (v, true))),
            v => out.push((v, is_reference(a, book, 0))),
        }
    }
    Ok(out)
}

/// The numbers SUM, AVERAGE, MIN and MAX use: in ranges only numbers; as direct arguments,
/// numbers, booleans and numeric text. An error anywhere is the answer.
fn numbers(args: &[Expr], book: &mut Context<'_>) -> Result<Vec<f64>, &'static str> {
    let mut out = Vec::new();
    for (v, in_range) in flatten(args, book)? {
        match (&v, in_range) {
            (V::Err(e), _) => return Err(e),
            (V::Num(n), _) => out.push(*n),
            (_, true) | (V::Empty, false) => {}
            (v, false) => out.push(num(v)?),
        }
    }
    Ok(out)
}

/// Rows of values from an argument that should be a range (or a single value, as a 1×1).
fn grid(arg: &Expr, book: &mut Context<'_>) -> Result<Vec<Vec<V>>, &'static str> {
    Ok(match eval_inner(arg, book)? {
        V::Range(r) => r,
        V::Err(e) => return Err(e),
        v => vec![vec![v]],
    })
}

/// A criterion as SUMIF and COUNTIF take one: `">5"`, `"<>x"`, `"=a*"`, `"apple"`, 3.
fn matches_criterion(v: &V, criterion: &V) -> bool {
    let (op, operand) = match criterion {
        V::Text(t) => {
            let t = t.as_str();
            let op = ["<=", ">=", "<>", "<", ">", "="].into_iter().find(|o| t.starts_with(o)).unwrap_or("");
            let rest = &t[op.len()..];
            let operand = match rest.parse::<f64>() {
                Ok(n) => V::Num(n),
                Err(_) if rest.eq_ignore_ascii_case("TRUE") => V::Bool(true),
                Err(_) if rest.eq_ignore_ascii_case("FALSE") => V::Bool(false),
                Err(_) => V::Text(rest.to_string()),
            };
            (if op.is_empty() { "=" } else { op }, operand)
        }
        V::Empty => ("=", V::Num(0.0)),
        other => ("=", other.clone()),
    };
    // Equality with text uses wildcards; numbers compare only with numbers.
    match (&operand, v) {
        (V::Text(pattern), _) if op == "=" || op == "<>" => {
            let s = match v {
                V::Text(s) => s.clone(),
                V::Empty => String::new(),
                _ => return op == "<>",
            };
            let m = wildcard(&pattern.to_lowercase(), &s.to_lowercase());
            if op == "=" {
                m
            } else {
                !m
            }
        }
        (V::Num(_), V::Num(_) | V::Empty) | (V::Bool(_), V::Bool(_)) | (V::Text(_), V::Text(_)) => match compare(v, &operand) {
            Ok(o) => match op {
                "=" => o.is_eq(),
                "<>" => o.is_ne(),
                "<" => o.is_lt(),
                "<=" => o.is_le(),
                ">" => o.is_gt(),
                _ => o.is_ge(),
            },
            Err(_) => false,
        },
        _ => op == "<>",
    }
}

/// `*` any run, `?` any one character, `~` escapes.
fn wildcard(pattern: &str, s: &str) -> bool {
    let s: Vec<char> = s.chars().collect();
    let mut matched = vec![false; s.len() + 1];
    matched[0] = true;
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c == '*' {
            for i in 1..=s.len() {
                matched[i] |= matched[i - 1];
            }
        } else {
            let literal = if c == '~' { chars.next().unwrap_or('~') } else { c };
            for i in (1..=s.len()).rev() {
                matched[i] = matched[i - 1] && (c == '?' || s[i - 1] == literal);
            }
            matched[0] = false;
        }
    }
    matched[s.len()]
}

fn round_to(n: f64, digits: i32, how: fn(f64) -> f64) -> f64 {
    let f = 10f64.powi(digits);
    // Excel rounds what it shows: 2.675 to 2 places is 2.68, though 2.675 is a hair under.
    let scaled = (n * f * 1e9).round() / 1e9;
    how(scaled) / f
}

fn round_half_away(x: f64) -> f64 {
    x.signum() * (x.abs() + 0.5).floor()
}

fn call(name: &str, args: &[Expr], book: &mut Context<'_>) -> R {
    let arg = |i: usize, book: &mut Context<'_>| -> R { args.get(i).map(|a| eval_inner(a, book).map(scalar)).unwrap_or(Ok(V::Empty)) };
    let n_arg = |i: usize, book: &mut Context<'_>| -> Result<f64, &'static str> { num(&arg(i, book)?) };
    let optional_num = |i: usize, default: f64, book: &mut Context<'_>| -> Result<f64, &'static str> {
        if args.get(i).is_none_or(|a| matches!(a, Expr::Missing)) {
            Ok(default)
        } else {
            n_arg(i, book)
        }
    };
    let need = |min: usize, max: usize| if args.len() < min || args.len() > max { Err("#VALUE!") } else { Ok(()) };
    let sys = book.date_system();
    Ok(match name {
        // ---- additional worksheet functions ----
        "SUMIFS" | "COUNTIFS" | "AVERAGEIFS" | "MAXIFS" | "MINIFS" => conditional_aggregate(name, args, book)?,
        "IFS" => {
            need(2, 254)?;
            if !args.len().is_multiple_of(2) {
                return Err("#VALUE!");
            }
            let mut result = None;
            for pair in args.as_chunks::<2>().0 {
                if truthy(&scalar(eval_inner(&pair[0], book)?))? {
                    result = Some(scalar(eval_inner(&pair[1], book)?));
                    break;
                }
            }
            result.ok_or("#N/A")?
        }
        "SWITCH" => {
            need(3, 254)?;
            let value = arg(0, book)?;
            let mut result = None;
            for pair in args[1..].as_chunks::<2>().0 {
                if compare(&value, &scalar(eval_inner(&pair[0], book)?))?.is_eq() {
                    result = Some(scalar(eval_inner(&pair[1], book)?));
                    break;
                }
            }
            match result {
                Some(v) => v,
                None if args.len().is_multiple_of(2) => arg(args.len() - 1, book)?,
                None => return Err("#N/A"),
            }
        }
        "XLOOKUP" | "XMATCH" => {
            need(if name == "XLOOKUP" { 3 } else { 2 }, if name == "XLOOKUP" { 6 } else { 4 })?;
            let key = arg(0, book)?;
            if let V::Err(e) = key {
                return Err(e);
            }
            let lookup = grid(&args[1], book)?;
            let width = lookup.first().map_or(0, Vec::len);
            if lookup.is_empty() || width == 0 || (lookup.len() > 1 && width > 1) {
                return Err("#VALUE!");
            }
            let list: Vec<V> = lookup.iter().flatten().cloned().collect();
            let mode_at = if name == "XLOOKUP" { 4 } else { 2 };
            let mode = optional_num(mode_at, 0.0, book)?;
            let search = optional_num(mode_at + 1, 1.0, book)?;
            if ![-1.0, 0.0, 1.0, 2.0].contains(&mode) || ![-1.0, 1.0].contains(&search) {
                return Err("#VALUE!");
            }
            let returns = if name == "XLOOKUP" {
                let g = grid(&args[2], book)?;
                if (lookup.len() == 1 && g.first().map_or(0, Vec::len) != width) || (lookup.len() > 1 && g.len() != lookup.len()) {
                    return Err("#VALUE!");
                }
                Some(g)
            } else {
                None
            };
            match xmatch(&key, &list, mode as i32, search < 0.0) {
                Ok(Some(i)) => match returns {
                    Some(g) => {
                        if lookup.len() == 1 {
                            g[0][i].clone()
                        } else {
                            g[i][0].clone()
                        }
                    }
                    None => V::Num((i + 1) as f64),
                },
                Ok(None) if name == "XLOOKUP" && args.get(3).is_some_and(|a| !matches!(a, Expr::Missing)) => arg(3, book)?,
                Ok(None) => return Err("#N/A"),
                Err(e) => return Err(e),
            }
        }
        "MEDIAN" | "MODE" | "MODE.SNGL" | "STDEV" | "STDEV.S" | "STDEV.P" | "VAR" | "VAR.S" | "VAR.P" => {
            need(1, 255)?;
            let mut ns = numbers(args, book)?;
            let n = ns.len();
            if matches!(name, "MEDIAN" | "MODE" | "MODE.SNGL") {
                ns.sort_by(f64::total_cmp);
            }
            if name == "MEDIAN" {
                if n == 0 {
                    return Err("#NUM!");
                }
                V::Num(ns[n / 2] / 2.0 + ns[(n - 1) / 2] / 2.0)
            } else if name == "MODE" || name == "MODE.SNGL" {
                let mut best = (0, 0.0);
                let mut start = 0;
                while start < n {
                    let mut end = start + 1;
                    while end < n && ns[end] == ns[start] {
                        end += 1;
                    }
                    if end - start > best.0 {
                        best = (end - start, ns[start]);
                    }
                    start = end;
                }
                if best.0 < 2 {
                    return Err("#N/A");
                }
                V::Num(best.1)
            } else {
                let population = name.ends_with(".P");
                if n < if population { 1 } else { 2 } {
                    return Err("#DIV/0!");
                }
                // Welford's recurrence avoids subtracting two large squared sums.
                let (mut mean, mut m2) = (0.0, 0.0);
                for (i, x) in ns.iter().enumerate() {
                    let delta = x - mean;
                    mean += delta / (i + 1) as f64;
                    m2 += delta * (x - mean);
                }
                let variance = m2 / (n - usize::from(!population)) as f64;
                if !variance.is_finite() {
                    return Err("#NUM!");
                }
                let variance = variance.max(0.0);
                V::Num(if name.starts_with("STDEV") { variance.sqrt() } else { variance })
            }
        }
        "AVERAGEA" => {
            need(1, 255)?;
            let mut ns = Vec::new();
            for (v, in_range) in flatten(args, book)? {
                match v {
                    V::Empty => {}
                    V::Text(_) if in_range => ns.push(0.0),
                    _ => ns.push(num(&v)?),
                }
            }
            if ns.is_empty() {
                return Err("#DIV/0!");
            }
            V::Num(ns.iter().sum::<f64>() / ns.len() as f64)
        }
        "LARGE" | "SMALL" | "RANK" | "RANK.EQ" => {
            need(2, if name.starts_with("RANK") { 3 } else { 2 })?;
            if name.starts_with("RANK") {
                let n = n_arg(0, book)?;
                let ns = numbers(&args[1..2], book)?;
                if !ns.contains(&n) {
                    return Err("#N/A");
                }
                let ascending = args.len() > 2 && n_arg(2, book)? != 0.0;
                V::Num(1.0 + ns.iter().filter(|&&x| if ascending { x < n } else { x > n }).count() as f64)
            } else {
                let mut ns = numbers(&args[..1], book)?;
                let k = n_arg(1, book)?.ceil();
                if k < 1.0 || k > ns.len() as f64 {
                    return Err("#NUM!");
                }
                ns.sort_by(f64::total_cmp);
                V::Num(ns[if name == "SMALL" { k as usize - 1 } else { ns.len() - k as usize }])
            }
        }
        "CEILING" | "FLOOR" | "CEILING.MATH" | "FLOOR.MATH" | "MROUND" => {
            let math = name.ends_with(".MATH");
            need(if math { 1 } else { 2 }, if math { 3 } else { 2 })?;
            let n = n_arg(0, book)?;
            let significance = optional_num(1, 1.0, book)?;
            let mode = optional_num(2, 0.0, book)?;
            if significance == 0.0 && name == "FLOOR" {
                return Err("#DIV/0!");
            }
            if significance == 0.0 || n == 0.0 {
                return Ok(V::Num(0.0));
            }
            if (name == "MROUND" && n.signum() != significance.signum()) || (!math && n > 0.0 && significance < 0.0) {
                return Err("#NUM!");
            }
            let s = significance.abs();
            let q = n / s;
            // Remove division noise around an integer multiple without changing genuine fractions.
            let q = if (q - q.round()).abs() <= q.abs().max(1.0) * f64::EPSILON * 4.0 { q.round() } else { q };
            let rounded = match name {
                "MROUND" => round_half_away(q),
                "CEILING.MATH" if n < 0.0 && mode != 0.0 => q.floor(),
                "FLOOR.MATH" if n < 0.0 && mode != 0.0 => q.ceil(),
                "CEILING" if significance < 0.0 => q.floor(),
                "FLOOR" if significance < 0.0 => q.ceil(),
                "CEILING" | "CEILING.MATH" => q.ceil(),
                _ => q.floor(),
            };
            finite(rounded * s)?
        }
        "TRUNC" => {
            need(1, 2)?;
            let n = n_arg(0, book)?;
            let digits = optional_num(1, 0.0, book)?.trunc();
            if digits > 308.0 {
                V::Num(n)
            } else if digits < -308.0 {
                V::Num(0.0)
            } else {
                let f = 10f64.powi(digits as i32);
                V::Num(if (n * f).is_finite() { (n * f).trunc() / f } else { n })
            }
        }
        "LOG" => {
            need(1, 2)?;
            let n = n_arg(0, book)?;
            let base = optional_num(1, 10.0, book)?;
            if n <= 0.0 || base <= 0.0 {
                return Err("#NUM!");
            }
            if base == 1.0 {
                return Err("#DIV/0!");
            }
            finite(n.log(base))?
        }
        "EVEN" | "ODD" => {
            need(1, 1)?;
            let n = n_arg(0, book)?;
            let offset = if name == "ODD" { 1.0 } else { 0.0 };
            V::Num(if n == 0.0 { offset } else { n.signum() * (((n.abs() - offset) / 2.0).ceil() * 2.0 + offset) })
        }
        "RAND" | "RANDBETWEEN" => {
            need(if name == "RAND" { 0 } else { 2 }, if name == "RAND" { 0 } else { 2 })?;
            if name == "RAND" {
                V::Num(random(book.now()))
            } else {
                let bottom = n_arg(0, book)?.ceil();
                let top = n_arg(1, book)?.floor();
                if bottom > top {
                    return Err("#NUM!");
                }
                finite(bottom + (random(book.now()) * (top - bottom + 1.0)).floor())?
            }
        }
        "SUMSQ" => {
            need(1, 255)?;
            finite(numbers(args, book)?.iter().map(|n| n * n).sum())?
        }
        "QUOTIENT" => {
            need(2, 2)?;
            let (n, d) = (n_arg(0, book)?, n_arg(1, book)?);
            if d == 0.0 {
                return Err("#DIV/0!");
            }
            finite((n / d).trunc())?
        }
        "GCD" | "LCM" => {
            need(1, 255)?;
            let ns = numbers(args, book)?;
            let mut result = if name == "GCD" { 0_u64 } else { 1_u64 };
            for n in ns {
                if !(0.0..9_007_199_254_740_992.0).contains(&n) {
                    return Err("#NUM!");
                }
                let n = n.trunc() as u64;
                result = if name == "GCD" {
                    gcd(result, n)
                } else if result == 0 || n == 0 {
                    0
                } else {
                    result.checked_div(gcd(result, n)).and_then(|x| x.checked_mul(n)).ok_or("#NUM!")?
                };
                if result >= 9_007_199_254_740_992 {
                    return Err("#NUM!");
                }
            }
            V::Num(result as f64)
        }
        "FACT" => {
            need(1, 1)?;
            let n = n_arg(0, book)?;
            if !(0.0..171.0).contains(&n) {
                return Err("#NUM!");
            }
            V::Num((1..=n as u32).fold(1.0, |a, b| a * b as f64))
        }
        "DEGREES" | "RADIANS" | "SIN" | "COS" | "TAN" | "ASIN" | "ACOS" | "ATAN" | "ATAN2" => {
            need(if name == "ATAN2" { 2 } else { 1 }, if name == "ATAN2" { 2 } else { 1 })?;
            let n = n_arg(0, book)?;
            finite(match name {
                "DEGREES" => n.to_degrees(),
                "RADIANS" => n.to_radians(),
                "SIN" => n.sin(),
                "COS" => n.cos(),
                "TAN" => n.tan(),
                "ASIN" => n.asin(),
                "ACOS" => n.acos(),
                "ATAN" => n.atan(),
                _ => {
                    let y = n_arg(1, book)?;
                    if n == 0.0 && y == 0.0 {
                        return Err("#DIV/0!");
                    }
                    y.atan2(n)
                }
            })?
        }
        "EDATE" | "EOMONTH" => {
            need(2, 2)?;
            let start = date_arg(&arg(0, book)?, sys)?;
            let dt = dates::from_serial(start, sys).ok_or("#NUM!")?;
            let months = n_arg(1, book)?.trunc();
            if months.abs() > 120_000.0 {
                return Err("#NUM!");
            }
            let index = dt.year as i64 * 12 + dt.month as i64 - 1 + months as i64;
            let (y, m) = (index.div_euclid(12), index.rem_euclid(12) as u32 + 1);
            if !(1900..=9999).contains(&y) {
                return Err("#NUM!");
            }
            let last = if y == 1900 && m == 2 && sys == DateSystem::From1900 { 29 } else { month_days(y as i32, m) };
            let day = if name == "EOMONTH" { last } else { dt.day.min(last) };
            let out = dates::to_serial(dates::DateTime::date(y as i32, m, day), sys);
            checked_date(out, sys)?;
            V::Num(out)
        }
        "DAYS" | "DATEDIF" => {
            need(if name == "DAYS" { 2 } else { 3 }, if name == "DAYS" { 2 } else { 3 })?;
            let a = date_arg(&arg(0, book)?, sys)?;
            let b = date_arg(&arg(1, book)?, sys)?;
            if name == "DAYS" {
                V::Num(a - b)
            } else {
                let (a, b) = (a.floor(), b.floor());
                if a > b {
                    return Err("#NUM!");
                }
                let start = dates::from_serial(a, sys).ok_or("#NUM!")?;
                let end = dates::from_serial(b, sys).ok_or("#NUM!")?;
                let months = (end.year - start.year) * 12 + end.month as i32 - start.month as i32 - i32::from(end.day < start.day);
                V::Num(match text(&arg(2, book)?)?.to_ascii_uppercase().as_str() {
                    "Y" => (months / 12) as f64,
                    "M" => months as f64,
                    "D" => b - a,
                    "YM" => months.rem_euclid(12) as f64,
                    "MD" => {
                        if end.day >= start.day {
                            (end.day - start.day) as f64
                        } else {
                            let prev = dates::from_serial(dates::to_serial(dates::DateTime::date(end.year, end.month, 1), sys) - 1.0, sys)
                                .ok_or("#NUM!")?;
                            (prev.day as i32 - start.day as i32 + end.day as i32) as f64
                        }
                    }
                    "YD" => {
                        let year = start.year + i32::from((end.month, end.day) < (start.month, start.day));
                        dates::to_serial(dates::DateTime::date(year, end.month, 1), sys) + end.day as f64 - 1.0 - a
                    }
                    _ => return Err("#NUM!"),
                })
            }
        }
        "NETWORKDAYS" | "WORKDAY" => {
            need(2, 3)?;
            let start = date_arg(&arg(0, book)?, sys)?.floor();
            let mut holidays = HashSet::new();
            if args.len() > 2 {
                for (v, _) in flatten(&args[2..3], book)? {
                    if !matches!(v, V::Empty) {
                        holidays.insert(date_arg(&v, sys)?.floor() as i64);
                    }
                }
            }
            let business = |day: f64| -> Result<bool, &'static str> {
                checked_date(day, sys)?;
                let dt = dates::from_serial(day, sys).ok_or("#NUM!")?;
                Ok(dt.weekday() <= 5 && !holidays.contains(&(day as i64)))
            };
            if name == "NETWORKDAYS" {
                let end = date_arg(&arg(1, book)?, sys)?.floor();
                let (lo, hi) = (start.min(end) as i64, start.max(end) as i64);
                let mut count = 0;
                for day in lo..=hi {
                    if business(day as f64)? {
                        count += 1;
                    }
                }
                V::Num(count as f64 * if end < start { -1.0 } else { 1.0 })
            } else {
                let days = n_arg(1, book)?.trunc();
                if days.abs() > 3_000_000.0 {
                    return Err("#NUM!");
                }
                let mut remaining = days.abs() as u32;
                let mut day = start;
                while remaining > 0 {
                    day += days.signum();
                    if business(day)? {
                        remaining -= 1;
                    }
                }
                V::Num(day)
            }
        }
        "WEEKNUM" => {
            need(1, 2)?;
            let serial = date_arg(&arg(0, book)?, sys)?.floor();
            let dt = dates::from_serial(serial, sys).ok_or("#NUM!")?;
            let kind = optional_num(1, 1.0, book)?;
            let first_weekday = match kind {
                1.0 | 17.0 => 7,
                2.0 | 11.0 | 21.0 => 1,
                12.0 => 2,
                13.0 => 3,
                14.0 => 4,
                15.0 => 5,
                16.0 => 6,
                _ => return Err("#NUM!"),
            };
            if kind == 21.0 {
                // The ISO week belongs to the year of its Thursday.
                let thursday = serial + 4.0 - dt.weekday() as f64;
                let year = dates::from_serial(thursday, sys).ok_or("#NUM!")?.year;
                let jan4 = dates::to_serial(dates::DateTime::date(year, 1, 4), sys);
                let weekday = dates::DateTime::date(year, 1, 4).weekday();
                V::Num(((serial - (jan4 - weekday as f64 + 1.0)) / 7.0).floor() + 1.0)
            } else {
                let jan1 = dates::to_serial(dates::DateTime::date(dt.year, 1, 1), sys);
                let offset = (dates::DateTime::date(dt.year, 1, 1).weekday() as i32 - first_weekday).rem_euclid(7);
                V::Num(((serial - jan1 + offset as f64) / 7.0).floor() + 1.0)
            }
        }
        "DATEVALUE" | "TIMEVALUE" => {
            need(1, 1)?;
            let s = text(&arg(0, book)?)?;
            V::Num(if name == "DATEVALUE" { parse_date_text(&s, sys).ok_or("#VALUE!")? } else { parse_time_text(&s).ok_or("#VALUE!")? })
        }
        "CLEAN" => {
            need(1, 1)?;
            V::Text(text(&arg(0, book)?)?.chars().filter(|&c| c as u32 >= 32).collect())
        }
        "NUMBERVALUE" => {
            need(1, 3)?;
            let s = text(&arg(0, book)?)?;
            let decimal = if args.len() > 1 { text(&arg(1, book)?)? } else { ".".into() };
            let group = if args.len() > 2 { text(&arg(2, book)?)? } else { ",".into() };
            V::Num(number_value(&s, &decimal, &group)?)
        }
        "FIXED" | "DOLLAR" => {
            need(1, if name == "FIXED" { 3 } else { 2 })?;
            let n = n_arg(0, book)?;
            let digits = optional_num(1, 2.0, book)?.trunc();
            if !(-308.0..=127.0).contains(&digits) {
                return Err("#VALUE!");
            }
            let commas = name == "DOLLAR" || args.len() < 3 || !truthy(&arg(2, book)?)?;
            let rounded = round_to(n, digits as i32, round_half_away);
            if !rounded.is_finite() {
                return Err("#NUM!");
            }
            let magnitude = if name == "DOLLAR" { rounded.abs() } else { rounded };
            let mut out = format!("{:.*}", digits.max(0.0) as usize, magnitude);
            if commas {
                out = group_number(&out);
            }
            if name == "DOLLAR" {
                out = if rounded < 0.0 { format!("(${out})") } else { format!("${out}") };
            }
            V::Text(out)
        }
        "TEXTBEFORE" | "TEXTAFTER" => {
            need(2, 6)?;
            let s = text(&arg(0, book)?)?;
            let delimiter = text(&arg(1, book)?)?;
            let instance = optional_num(2, 1.0, book)?.trunc();
            let mode = optional_num(3, 0.0, book)?;
            let match_end = optional_num(4, 0.0, book)?;
            if instance == 0.0 || ![0.0, 1.0].contains(&mode) || ![0.0, 1.0].contains(&match_end) {
                return Err("#VALUE!");
            }
            if s.is_empty() {
                return Ok(V::Text(String::new()));
            }
            if instance.abs() > s.chars().count() as f64 {
                return Err("#VALUE!");
            }
            if delimiter.is_empty() {
                return Ok(V::Text(if (name == "TEXTAFTER") == (instance > 0.0) { s } else { String::new() }));
            }
            let hits = delimiter_hits(&s, &delimiter, mode != 0.0, instance < 0.0);
            let hit = hits.get(instance.abs() as usize - 1).copied().or_else(|| {
                if match_end != 0.0 && instance.abs() as usize == hits.len() + 1 {
                    Some(if instance > 0.0 { (s.len(), s.len()) } else { (0, 0) })
                } else {
                    None
                }
            });
            match hit {
                Some((a, b)) => V::Text(if name == "TEXTBEFORE" { s[..a].into() } else { s[b..].into() }),
                None if args.get(5).is_some_and(|a| !matches!(a, Expr::Missing)) => arg(5, book)?,
                None => return Err("#N/A"),
            }
        }
        // ---- maths ----
        "SUM" => V::Num(numbers(args, book)?.iter().sum()),
        "PRODUCT" => V::Num(numbers(args, book)?.iter().product()),
        "AVERAGE" => {
            let n = numbers(args, book)?;
            if n.is_empty() {
                return Err("#DIV/0!");
            }
            V::Num(n.iter().sum::<f64>() / n.len() as f64)
        }
        "MIN" => V::Num(numbers(args, book)?.into_iter().reduce(f64::min).unwrap_or(0.0)),
        "MAX" => V::Num(numbers(args, book)?.into_iter().reduce(f64::max).unwrap_or(0.0)),
        "COUNT" => V::Num(
            flatten(args, book)?
                .iter()
                .filter(|(v, in_range)| matches!(v, V::Num(_)) || !in_range && num(v).is_ok() && !matches!(v, V::Empty))
                .count() as f64,
        ),
        "COUNTA" => V::Num(flatten(args, book)?.iter().filter(|(v, _)| !matches!(v, V::Empty)).count() as f64),
        "COUNTBLANK" => {
            V::Num(flatten(args, book)?.iter().filter(|(v, _)| matches!(v, V::Empty) || matches!(v, V::Text(t) if t.is_empty())).count()
                as f64)
        }
        "ABS" => V::Num(n_arg(0, book)?.abs()),
        "INT" => V::Num(n_arg(0, book)?.floor()),
        "SIGN" => V::Num(n_arg(0, book)?.signum()),
        "SQRT" => {
            let n = n_arg(0, book)?;
            if n < 0.0 {
                return Err("#NUM!");
            }
            V::Num(n.sqrt())
        }
        "POWER" => binary(Op::Pow, &arg(0, book)?, &arg(1, book)?)?,
        "MOD" => {
            let (n, d) = (n_arg(0, book)?, n_arg(1, book)?);
            if d == 0.0 {
                return Err("#DIV/0!");
            }
            // The result has the divisor's sign, as in Excel.
            V::Num(n - d * (n / d).floor())
        }
        "ROUND" | "ROUNDUP" | "ROUNDDOWN" => {
            need(1, 2)?;
            let (n, d) = (n_arg(0, book)?, n_arg(1, book)? as i32);
            let how: fn(f64) -> f64 = match name {
                "ROUNDUP" => |x| x.signum() * x.abs().ceil(),
                "ROUNDDOWN" => f64::trunc,
                _ => round_half_away,
            };
            V::Num(round_to(n, d, how))
        }
        "PI" => V::Num(std::f64::consts::PI),
        "LN" | "LOG10" | "EXP" => {
            let n = n_arg(0, book)?;
            finite(match name {
                "EXP" => n.exp(),
                _ if n <= 0.0 => return Err("#NUM!"),
                "LN" => n.ln(),
                _ => n.log10(),
            })?
        }
        "SUMPRODUCT" => {
            let grids: Vec<Vec<Vec<V>>> = args.iter().map(|a| grid(a, book)).collect::<Result<_, _>>()?;
            let shape = grids.first().map(|g| (g.len(), g.first().map_or(0, Vec::len))).unwrap_or((0, 0));
            if grids.iter().any(|g| g.len() != shape.0 || g.iter().any(|r| r.len() != shape.1)) {
                return Err("#VALUE!");
            }
            for v in grids.iter().flatten().flatten() {
                if let V::Err(e) = v {
                    return Err(e);
                }
            }
            let mut total = 0.0;
            for r in 0..shape.0 {
                for c in 0..shape.1 {
                    total += grids.iter().map(|g| if let V::Num(n) = g[r][c] { n } else { 0.0 }).product::<f64>();
                }
            }
            V::Num(total)
        }
        "SUMIF" | "COUNTIF" | "AVERAGEIF" => {
            let range = grid(args.first().ok_or("#VALUE!")?, book)?;
            let criterion = arg(1, book)?;
            let values = if args.len() > 2 && name != "COUNTIF" { grid(&args[2], book)? } else { range.clone() };
            let mut picked = Vec::new();
            for (r, row) in range.iter().enumerate() {
                for (c, v) in row.iter().enumerate() {
                    if matches_criterion(v, &criterion) {
                        picked.push(values.get(r).and_then(|x| x.get(c)).cloned().unwrap_or(V::Empty));
                    }
                }
            }
            match name {
                "COUNTIF" => V::Num(picked.len() as f64),
                _ => {
                    let nums: Vec<f64> = picked.iter().filter_map(|v| if let V::Num(n) = v { Some(*n) } else { None }).collect();
                    if name == "SUMIF" {
                        V::Num(nums.iter().sum())
                    } else if nums.is_empty() {
                        return Err("#DIV/0!");
                    } else {
                        V::Num(nums.iter().sum::<f64>() / nums.len() as f64)
                    }
                }
            }
        }
        // ---- logic ----
        "IF" => {
            need(1, 3)?;
            if truthy(&arg(0, book)?)? {
                if args.len() > 1 {
                    arg(1, book)?
                } else {
                    V::Bool(true)
                }
            } else if args.len() > 2 {
                arg(2, book)?
            } else {
                V::Bool(false)
            }
        }
        "IFERROR" => match arg(0, book) {
            Ok(V::Err(_)) | Err(_) => arg(1, book)?,
            Ok(v) => v,
        },
        "IFNA" => match arg(0, book) {
            Ok(V::Err("#N/A")) | Err("#N/A") => arg(1, book)?,
            Ok(v) => v,
            Err(e) => return Err(e),
        },
        "AND" | "OR" | "XOR" => {
            let vals: Vec<bool> = flatten(args, book)?
                .iter()
                .filter(|(v, in_range)| !(*in_range && matches!(v, V::Text(_) | V::Empty)))
                .map(|(v, _)| truthy(v))
                .collect::<Result<_, _>>()?;
            if vals.is_empty() {
                return Err("#VALUE!");
            }
            V::Bool(match name {
                "AND" => vals.iter().all(|b| *b),
                "OR" => vals.iter().any(|b| *b),
                _ => vals.iter().filter(|b| **b).count() % 2 == 1,
            })
        }
        "NOT" => V::Bool(!truthy(&arg(0, book)?)?),
        "TRUE" => V::Bool(true),
        "FALSE" => V::Bool(false),
        "ISBLANK" => V::Bool(matches!(arg(0, book), Ok(V::Empty))),
        "ISNUMBER" => V::Bool(matches!(arg(0, book), Ok(V::Num(_)))),
        "ISTEXT" => V::Bool(matches!(arg(0, book), Ok(V::Text(_)))),
        "ISNONTEXT" => V::Bool(!matches!(arg(0, book), Ok(V::Text(_)))),
        "ISLOGICAL" => V::Bool(matches!(arg(0, book), Ok(V::Bool(_)))),
        "ISERR" => V::Bool(matches!(arg(0, book), Ok(V::Err(e)) | Err(e) if e != "#N/A")),
        "ISERROR" => V::Bool(matches!(arg(0, book), Ok(V::Err(_)) | Err(_))),
        "ISNA" => V::Bool(matches!(arg(0, book), Ok(V::Err("#N/A")) | Err("#N/A"))),
        "NA" => return Err("#N/A"),
        // ---- text ----
        "CONCATENATE" | "CONCAT" => {
            let mut s = String::new();
            for (v, _) in flatten(args, book)? {
                s.push_str(&text(&v)?);
            }
            V::Text(s)
        }
        "TEXTJOIN" => {
            let sep = text(&arg(0, book)?)?;
            let skip_empty = truthy(&arg(1, book)?)?;
            let parts: Vec<String> = flatten(&args[2.min(args.len())..], book)?.iter().map(|(v, _)| text(v)).collect::<Result<_, _>>()?;
            V::Text(parts.into_iter().filter(|p| !(skip_empty && p.is_empty())).collect::<Vec<_>>().join(&sep))
        }
        "LEN" => V::Num(text(&arg(0, book)?)?.chars().count() as f64),
        "UPPER" => V::Text(text(&arg(0, book)?)?.to_uppercase()),
        "LOWER" => V::Text(text(&arg(0, book)?)?.to_lowercase()),
        "PROPER" => {
            let s = text(&arg(0, book)?)?;
            let mut out = String::new();
            let mut start = true;
            for c in s.chars() {
                out.extend(if start { c.to_uppercase().collect::<Vec<_>>() } else { c.to_lowercase().collect() });
                start = !c.is_alphabetic();
            }
            V::Text(out)
        }
        // Excel's TRIM also squeezes runs of spaces inside.
        "TRIM" => V::Text(text(&arg(0, book)?)?.split(' ').filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")),
        "LEFT" | "RIGHT" => {
            let s: Vec<char> = text(&arg(0, book)?)?.chars().collect();
            let n = if args.len() > 1 { n_arg(1, book)? } else { 1.0 };
            if n < 0.0 {
                return Err("#VALUE!");
            }
            let n = (n as usize).min(s.len());
            V::Text(if name == "LEFT" { s[..n].iter().collect() } else { s[s.len() - n..].iter().collect() })
        }
        "MID" => {
            let s: Vec<char> = text(&arg(0, book)?)?.chars().collect();
            let (start, len) = (n_arg(1, book)?, n_arg(2, book)?);
            if start < 1.0 || len < 0.0 {
                return Err("#VALUE!");
            }
            let from = (start as usize - 1).min(s.len());
            V::Text(s[from..from.saturating_add(len as usize).min(s.len())].iter().collect())
        }
        "FIND" | "SEARCH" => {
            let (needle, hay) = (text(&arg(0, book)?)?, text(&arg(1, book)?)?);
            let start = if args.len() > 2 { n_arg(2, book)? as usize } else { 1 };
            let chars: Vec<char> = hay.chars().collect();
            if start < 1 || start > chars.len() + 1 {
                return Err("#VALUE!");
            }
            let rest: String = chars[start - 1..].iter().collect();
            let at = if name == "FIND" {
                rest.find(&needle).map(|b| rest[..b].chars().count())
            } else {
                let pattern = format!("{}*", needle.to_lowercase());
                (0..=rest.chars().count()).find(|&i| wildcard(&pattern, &rest.chars().skip(i).collect::<String>().to_lowercase()))
            };
            V::Num((at.ok_or("#VALUE!")? + start) as f64)
        }
        "SUBSTITUTE" => {
            let (s, old, new) = (text(&arg(0, book)?)?, text(&arg(1, book)?)?, text(&arg(2, book)?)?);
            if old.is_empty() {
                V::Text(s)
            } else if args.len() > 3 {
                let nth = n_arg(3, book)? as usize;
                match s.match_indices(&old).nth(nth.saturating_sub(1)) {
                    Some((i, _)) => V::Text(format!("{}{}{}", &s[..i], new, &s[i + old.len()..])),
                    None => V::Text(s),
                }
            } else {
                V::Text(s.replace(&old, &new))
            }
        }
        "REPT" => {
            let (s, n) = (text(&arg(0, book)?)?, n_arg(1, book)?.max(0.0) as usize);
            // Excel's limit on a cell's text.
            if s.chars().count().saturating_mul(n) > 32_767 {
                return Err("#VALUE!");
            }
            V::Text(s.repeat(n))
        }
        "VALUE" => V::Num(num(&V::Text(text(&arg(0, book)?)?))?),
        "TEXT" => {
            let v = arg(0, book)?;
            let code = text(&arg(1, book)?)?;
            let value = match &v {
                V::Text(t) => t.trim().parse::<f64>().map(Value::Number).unwrap_or_else(|_| Value::Text(t.clone())),
                other => other.to_value(),
            };
            V::Text(crate::numfmt::format(&value, &code, sys).text)
        }
        "CHAR" => {
            V::Text(char::from_u32(n_arg(0, book)? as u32).filter(|c| (1..=255).contains(&(*c as u32))).ok_or("#VALUE!")?.to_string())
        }
        "CODE" => V::Num(text(&arg(0, book)?)?.chars().next().ok_or("#VALUE!")? as u32 as f64),
        "EXACT" => V::Bool(text(&arg(0, book)?)? == text(&arg(1, book)?)?),
        // ---- lookup ----
        "VLOOKUP" | "HLOOKUP" => {
            need(3, 4)?;
            let key = arg(0, book)?;
            let mut table = grid(&args[1], book)?;
            if name == "HLOOKUP" {
                table = transpose(table);
            }
            let index = n_arg(2, book)? as usize;
            let approximate = args.len() < 4 || truthy(&arg(3, book)?)?;
            if index < 1 || table.first().is_none_or(|r| index > r.len()) {
                return Err(if index < 1 { "#VALUE!" } else { "#REF!" });
            }
            let column: Vec<V> = table.iter().map(|r| r[0].clone()).collect();
            let row = lookup(&key, &column, if approximate { 1 } else { 0 })?;
            table[row][index - 1].clone()
        }
        "MATCH" => {
            let key = arg(0, book)?;
            let g = grid(args.get(1).ok_or("#N/A")?, book)?;
            let list: Vec<V> = if g.len() == 1 {
                g.into_iter().next().unwrap()
            } else {
                g.into_iter().map(|r| r.into_iter().next().unwrap_or(V::Empty)).collect()
            };
            let kind = if args.len() > 2 { n_arg(2, book)? as i32 } else { 1 };
            V::Num((lookup(&key, &list, kind)? + 1) as f64)
        }
        "INDEX" => {
            let g = grid(args.first().ok_or("#VALUE!")?, book)?;
            let (r, c) =
                (if args.len() > 1 { n_arg(1, book)? as usize } else { 1 }, if args.len() > 2 { n_arg(2, book)? as usize } else { 0 });
            // INDEX(column, n) and INDEX(row, n) both pick the nth.
            let (r, c) = match (g.len(), c) {
                (1, 0) => (1, r),
                (_, 0) => (r, 1),
                _ => (r, c),
            };
            g.get(r.wrapping_sub(1)).and_then(|row| row.get(c.wrapping_sub(1))).cloned().ok_or("#REF!")?
        }
        "CHOOSE" => {
            let i = n_arg(0, book)? as usize;
            if i < 1 || i >= args.len() {
                return Err("#VALUE!");
            }
            arg(i, book)?
        }
        "ROWS" | "COLUMNS" => {
            let g = grid(args.first().ok_or("#VALUE!")?, book)?;
            V::Num(if name == "ROWS" { g.len() } else { g.first().map_or(0, Vec::len) } as f64)
        }
        // ---- dates ----
        "DATE" => finite(dates::date(n_arg(0, book)? as i64, n_arg(1, book)? as i64, n_arg(2, book)? as i64, sys))?,
        "YEAR" | "MONTH" | "DAY" | "WEEKDAY" | "HOUR" | "MINUTE" | "SECOND" => {
            let dt = dates::from_serial(n_arg(0, book)?, sys).ok_or("#NUM!")?;
            V::Num(match name {
                "YEAR" => dt.year as f64,
                "MONTH" => dt.month as f64,
                "DAY" => dt.day as f64,
                // Sunday is 1, as WEEKDAY's default has it.
                "WEEKDAY" => (dt.weekday() % 7 + 1) as f64,
                "HOUR" => dt.hour as f64,
                "MINUTE" => dt.minute as f64,
                _ => dt.second as f64,
            })
        }
        "TIME" => V::Num((n_arg(0, book)? * 3600.0 + n_arg(1, book)? * 60.0 + n_arg(2, book)?).rem_euclid(86_400.0) / 86_400.0),
        "NOW" => V::Num(book.now().ok_or("#N/A")?),
        "TODAY" => V::Num(book.now().ok_or("#N/A")?.floor()),
        _ => return Err("#NAME?"),
    })
}

/// IFS aggregates require matching shapes, including the criteria ranges.
fn conditional_aggregate(name: &str, args: &[Expr], book: &mut Context<'_>) -> R {
    let offset = usize::from(name != "COUNTIFS");
    if args.len() < offset + 2 || !(args.len() - offset).is_multiple_of(2) || args.len() > offset + 254 {
        return Err("#VALUE!");
    }
    let values = grid(&args[0], book)?;
    let shape = (values.len(), values.first().map_or(0, Vec::len));
    let mut conditions = Vec::new();
    for pair in args[offset..].as_chunks::<2>().0 {
        let range = grid(&pair[0], book)?;
        if range.len() != shape.0 || range.iter().any(|r| r.len() != shape.1) {
            return Err("#VALUE!");
        }
        let criterion = scalar(eval_inner(&pair[1], book)?);
        if let V::Err(e) = criterion {
            return Err(e);
        }
        conditions.push((range, criterion));
    }
    let mut count = 0;
    let mut ns = Vec::new();
    for (r, row) in values.iter().enumerate() {
        for (c, value) in row.iter().enumerate() {
            if conditions.iter().all(|(g, criterion)| matches_criterion(&g[r][c], criterion)) {
                count += 1;
                if name != "COUNTIFS" {
                    match value {
                        V::Err(e) => return Err(e),
                        V::Num(n) => ns.push(*n),
                        V::Empty if name == "MINIFS" || name == "MAXIFS" => ns.push(0.0),
                        _ => {}
                    }
                }
            }
        }
    }
    Ok(V::Num(match name {
        "COUNTIFS" => count as f64,
        "SUMIFS" => ns.iter().sum(),
        "MAXIFS" => ns.into_iter().reduce(f64::max).unwrap_or(0.0),
        "MINIFS" => ns.into_iter().reduce(f64::min).unwrap_or(0.0),
        _ if ns.is_empty() => return Err("#DIV/0!"),
        _ => ns.iter().sum::<f64>() / ns.len() as f64,
    }))
}

fn xmatch(key: &V, list: &[V], mode: i32, reverse: bool) -> Result<Option<usize>, &'static str> {
    let mut best = None;
    for step in 0..list.len() {
        let i = if reverse { list.len() - step - 1 } else { step };
        let v = &list[i];
        if let V::Err(e) = v {
            return Err(e);
        }
        if mode == 2 {
            if let (V::Text(k), V::Text(s)) = (key, v) {
                if wildcard(&k.to_lowercase(), &s.to_lowercase()) {
                    return Ok(Some(i));
                }
            } else if compare(v, key)?.is_eq() {
                return Ok(Some(i));
            }
            continue;
        }
        if std::mem::discriminant(v) != std::mem::discriminant(key) && !matches!(v, V::Empty) && !matches!(key, V::Empty) {
            continue;
        }
        let ordering = compare(v, key)?;
        if ordering.is_eq() {
            return Ok(Some(i));
        }
        if ((mode == -1 && ordering.is_lt()) || (mode == 1 && ordering.is_gt()))
            && best.is_none_or(|b| compare(v, &list[b]).is_ok_and(|o| if mode == -1 { o.is_gt() } else { o.is_lt() }))
        {
            best = Some(i);
        }
    }
    Ok(best)
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// A local PRNG also works in WebAssembly. The host supplies the clock through Book,
/// as it does for NOW; state advances even when a caller has no clock.
fn random(now: Option<f64>) -> f64 {
    thread_local! { static STATE: std::cell::Cell<u64> = const { std::cell::Cell::new(0) }; }
    STATE.with(|state| {
        let mut x = state.get();
        if x == 0 {
            x = now.unwrap_or(1.0).to_bits() ^ 0x9e37_79b9_7f4a_7c15;
        }
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        state.set(x);
        (x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11) as f64 / 9_007_199_254_740_992.0
    })
}

fn month_days(year: i32, month: u32) -> u32 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 31,
    }
}

fn checked_date(serial: f64, sys: DateSystem) -> Result<(), &'static str> {
    if dates::from_serial(serial, sys).is_none_or(|d| d.year > 9999) {
        Err("#NUM!")
    } else {
        Ok(())
    }
}

fn date_arg(v: &V, sys: DateSystem) -> Result<f64, &'static str> {
    let serial = match v {
        V::Text(s) => parse_date_text(s, sys).ok_or("#VALUE!")?,
        _ => num(v)?,
    };
    checked_date(serial, sys)?;
    Ok(serial)
}

/// Parse ISO, US numeric, and English month-name dates without a platform locale.
/// DATEVALUE ignores a time suffix and returns only the date.
fn parse_date_text(s: &str, sys: DateSystem) -> Option<f64> {
    let s = s.trim();
    let s = if s.as_bytes().get(10) == Some(&b'T') { &s[..10] } else { s };
    let normalized = s.replace(['-', '/', ','], " ");
    let parts: Vec<&str> = normalized.split_whitespace().take(3).collect();
    if parts.len() != 3 {
        return None;
    }
    let month = |word: &str| -> Option<i32> {
        if let Ok(n) = word.parse() {
            return Some(n);
        }
        let lower = word.to_ascii_lowercase();
        ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"]
            .iter()
            .position(|m| lower == *m || (lower.len() == 3 && m.starts_with(&lower)))
            .map(|i| i as i32 + 1)
    };
    let (y, m, d) = if parts[0].len() == 4 && parts[0].chars().all(|c| c.is_ascii_digit()) {
        (parts[0].parse().ok()?, month(parts[1])?, parts[2].parse().ok()?)
    } else if parts[1].chars().any(|c| c.is_ascii_alphabetic()) {
        (parts[2].parse().ok()?, month(parts[1])?, parts[0].parse().ok()?)
    } else {
        (parts[2].parse().ok()?, month(parts[0])?, parts[1].parse().ok()?)
    };
    let y = if (0..30).contains(&y) {
        2000 + y
    } else if (30..100).contains(&y) {
        1900 + y
    } else {
        y
    };
    if !(1900..=9999).contains(&y) || !(1..=12).contains(&m) || !(1..=month_days(y, m as u32) as i32).contains(&d) {
        return None;
    }
    let serial = dates::to_serial(dates::DateTime::date(y, m as u32, d as u32), sys);
    checked_date(serial, sys).ok()?;
    Some(serial)
}

fn parse_time_text(s: &str) -> Option<f64> {
    let upper = s.trim().to_ascii_uppercase();
    let (s, meridiem) = if let Some(s) = upper.strip_suffix("AM") {
        (s.trim(), Some(false))
    } else if let Some(s) = upper.strip_suffix("PM") {
        (s.trim(), Some(true))
    } else {
        (upper.as_str(), None)
    };
    let time = s.rsplit([' ', 'T']).next()?;
    let parts = time.split(':').map(|p| p.parse::<f64>().ok()).collect::<Option<Vec<_>>>()?;
    if !(2..=3).contains(&parts.len()) && !(parts.len() == 1 && meridiem.is_some()) {
        return None;
    }
    let mut hour = parts[0];
    let minute = parts.get(1).copied().unwrap_or(0.0);
    let second = parts.get(2).copied().unwrap_or(0.0);
    if hour.fract() != 0.0 || minute.fract() != 0.0 || !(0.0..60.0).contains(&minute) || !(0.0..60.0).contains(&second) {
        return None;
    }
    if let Some(pm) = meridiem {
        if !(1.0..=12.0).contains(&hour) {
            return None;
        }
        hour = hour % 12.0 + if pm { 12.0 } else { 0.0 };
    }
    if !(0.0..24.0).contains(&hour) {
        return None;
    }
    Some((hour * 3600.0 + minute * 60.0 + second) / 86_400.0)
}

fn number_value(s: &str, decimal: &str, group: &str) -> Result<f64, &'static str> {
    let decimal = decimal.chars().next();
    let group = group.chars().next();
    if decimal.is_some() && decimal == group {
        return Err("#VALUE!");
    }
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let trimmed = s.trim_end_matches('%');
    let percents = s.len() - trimmed.len();
    let mut out = String::new();
    let mut point = false;
    for c in trimmed.chars() {
        if Some(c) == decimal {
            if point {
                return Err("#VALUE!");
            }
            point = true;
            out.push('.');
        } else if Some(c) == group {
            if point {
                return Err("#VALUE!");
            }
        } else if c.is_ascii_digit() || ((c == '+' || c == '-') && out.is_empty()) {
            out.push(c);
        } else {
            return Err("#VALUE!");
        }
    }
    if s.is_empty() {
        return Ok(0.0);
    }
    let n = out.parse::<f64>().map_err(|_| "#VALUE!")? / 100f64.powi(percents.min(i32::MAX as usize) as i32);
    if n.is_finite() {
        Ok(n)
    } else {
        Err("#NUM!")
    }
}

fn group_number(s: &str) -> String {
    let whole = s.find('.').unwrap_or(s.len());
    let mut out = String::new();
    for (i, c) in s.char_indices() {
        if i > 0 && i < whole && (whole - i).is_multiple_of(3) && s.as_bytes()[i - 1].is_ascii_digit() {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Keep byte positions in the original string even when Unicode case folding changes length.
fn delimiter_hits(s: &str, delimiter: &str, ignore_case: bool, reverse: bool) -> Vec<(usize, usize)> {
    let needle: Vec<char> = delimiter.chars().collect();
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let mut candidates = Vec::new();
    for i in 0..=chars.len().saturating_sub(needle.len()) {
        if i + needle.len() > chars.len() {
            break;
        }
        if needle.iter().zip(&chars[i..i + needle.len()]).all(
            |(a, (_, b))| {
                if ignore_case {
                    a.to_lowercase().eq(b.to_lowercase())
                } else {
                    a == b
                }
            },
        ) {
            candidates.push((chars[i].0, chars.get(i + needle.len()).map_or(s.len(), |c| c.0)));
        }
    }
    if reverse {
        candidates.reverse();
    }
    let mut out: Vec<(usize, usize)> = Vec::new();
    for hit in candidates {
        if out.last().is_none_or(|prev| if reverse { hit.1 <= prev.0 } else { hit.0 >= prev.1 }) {
            out.push(hit);
        }
    }
    out
}

fn transpose(g: Vec<Vec<V>>) -> Vec<Vec<V>> {
    let cols = g.first().map_or(0, Vec::len);
    (0..cols).map(|c| g.iter().map(|r| r.get(c).cloned().unwrap_or(V::Empty)).collect()).collect()
}

/// Where `key` is in `list`: exactly (kind 0, with wildcards for text), or the last value not
/// greater (kind 1, list ascending) or not smaller (kind -1, descending).
fn lookup(key: &V, list: &[V], kind: i32) -> Result<usize, &'static str> {
    if kind == 0 {
        return list
            .iter()
            .position(|v| match (key, v) {
                (V::Text(k), V::Text(s)) => wildcard(&k.to_lowercase(), &s.to_lowercase()),
                _ => compare(v, key).is_ok_and(|o| o.is_eq()) && std::mem::discriminant(v) == std::mem::discriminant(key),
            })
            .ok_or("#N/A");
    }
    let mut best = None;
    for (i, v) in list.iter().enumerate() {
        if matches!(v, V::Empty) || std::mem::discriminant(v) != std::mem::discriminant(key) {
            continue;
        }
        let Ok(o) = compare(v, key) else { continue };
        if o.is_eq() || (kind > 0 && o.is_lt()) || (kind < 0 && o.is_gt()) {
            best = Some(i);
        } else {
            break;
        }
    }
    best.ok_or("#N/A")
}

/// A [`Book`] over plain values, for evaluating formulas without a workbook (and for tests).
#[derive(Default)]
pub struct Values {
    pub cells: HashMap<(u32, u32), V>,
    pub names: HashMap<String, Expr>,
}

impl Book for Values {
    fn cell(&mut self, _sheet: Option<&str>, row: u32, col: u32) -> V {
        self.cells.get(&(row, col)).cloned().unwrap_or(V::Empty)
    }
    fn extent(&self, _sheet: Option<&str>) -> Option<(u32, u32)> {
        let rows = self.cells.keys().map(|k| k.0).max()?;
        let cols = self.cells.keys().map(|k| k.1).max()?;
        Some((rows, cols))
    }
    fn name(&self, name: &str) -> Option<Expr> {
        self.names.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, e)| e.clone())
    }
    fn date_system(&self) -> DateSystem {
        DateSystem::From1900
    }
    fn now(&self) -> Option<f64> {
        None
    }
}

/// The text a value shows as in a formula's result, with no number format.
pub fn plain(v: &V) -> String {
    match v {
        V::Num(n) => format_number(*n),
        V::Err(e) => e.to_string(),
        other => text(other).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formula::parse;

    fn book() -> Values {
        let mut b = Values::default();
        // A1:A5 = 1..5, B1:B5 = fruit, C1 = "3", D1 = TRUE.
        for (r, fruit) in ["apple", "banana", "cherry", "apple", "date"].iter().enumerate() {
            b.cells.insert((r as u32, 0), V::Num(r as f64 + 1.0));
            b.cells.insert((r as u32, 1), V::Text(fruit.to_string()));
        }
        b.cells.insert((0, 2), V::Text("3".into()));
        b.cells.insert((0, 3), V::Bool(true));
        b.names.insert("Rate".into(), parse("0.2").unwrap());
        b
    }

    fn ev(f: &str) -> V {
        eval(&parse(f).unwrap(), &mut book())
    }

    #[test]
    fn conditional_aggregates_and_logic() {
        for (f, expected) in [
            ("SUMIFS(A1:A5,B1:B5,\"a*\",A1:A5,\">1\")", 4.0),
            ("COUNTIFS(B1:B5,\"apple\",A1:A5,\">1\")", 1.0),
            ("AVERAGEIFS(A1:A5,B1:B5,\"apple\")", 2.5),
            ("MAXIFS(A1:A5,B1:B5,\"apple\")", 4.0),
            ("MINIFS(A1:A5,B1:B5,\"apple\")", 1.0),
            ("MAXIFS(A1:A5,B1:B5,\"missing\")", 0.0),
            ("COUNTIFS(Z1:Z3,0)", 3.0),
            ("MINIFS(A1:A6,B1:B6,\"\")", 0.0),
            ("MINIFS({-3,-1},{1,1},1)", -3.0),
            ("IFS(FALSE,1/0,TRUE,7,TRUE,9)", 7.0),
            ("SWITCH(2,1,1/0,2,7,9)", 7.0),
            ("SWITCH(8,1,2,9)", 9.0),
        ] {
            assert_eq!(ev(f), V::Num(expected), "{f}");
        }
        for (f, error) in [
            ("SUMIFS(A1:A5,B1:B4,1)", "#VALUE!"),
            ("COUNTIFS(A1:A5,1,B1:B4,1)", "#VALUE!"),
            ("AVERAGEIFS(A1:A5,B1:B5,\"missing\")", "#DIV/0!"),
            ("MAXIFS(A1:A5,B1:B4,1)", "#VALUE!"),
            ("MINIFS(A1:A5,B1:B4,1)", "#VALUE!"),
            ("SUMIFS({1,#N/A},{1,1},1)", "#N/A"),
            ("SUMIFS(A1:A5,B1:B5,#N/A)", "#N/A"),
            ("IFS(FALSE,1)", "#N/A"),
            ("IFS(TRUE,1,FALSE)", "#VALUE!"),
            ("IFS(\"bad\",1)", "#VALUE!"),
            ("SWITCH(3,1,2)", "#N/A"),
            ("SWITCH(#N/A,1,2)", "#N/A"),
        ] {
            assert_eq!(ev(f), V::Err(error), "{f}");
        }
    }

    #[test]
    fn modern_lookups() {
        for (f, expected) in [
            ("XLOOKUP(\"apple\",B1:B5,A1:A5)", 1.0),
            ("XLOOKUP(\"apple\",B1:B5,A1:A5,,0,-1)", 4.0),
            ("XLOOKUP(3,{5,1,4},{50,10,40},,-1)", 10.0),
            ("XLOOKUP(3,{5,1,4},{50,10,40},,1)", 40.0),
            ("XLOOKUP(\"a*\",B1:B5,A1:A5,,2,-1)", 4.0),
            ("XLOOKUP(\"a*\",B1:B5,A1:A5,99)", 99.0),
            ("XLOOKUP(2,{1,2},{10,20;30,40})", 20.0),
            ("XLOOKUP(2,{1;2},{10,11;20,21})", 20.0),
            ("XMATCH(\"apple\",B1:B5)", 1.0),
            ("XMATCH(Z99,{0,1})", 1.0),
            ("XMATCH(\"apple\",B1:B5,0,-1)", 4.0),
            ("XMATCH(3,{5,1,4},-1)", 2.0),
            ("XMATCH(3,{5,1,4},1)", 3.0),
            ("XMATCH(\"a*\",B1:B5,2,-1)", 4.0),
        ] {
            assert_eq!(ev(f), V::Num(expected), "{f}");
        }
        for (f, error) in [
            ("XLOOKUP(8,A1:A5,B1:B5)", "#N/A"),
            ("XMATCH(8,A1:A5)", "#N/A"),
            ("XLOOKUP(8,A1:A5,B1:B5,,0)", "#N/A"),
            ("XLOOKUP(1,A1:A5,B1:B4)", "#VALUE!"),
            ("XMATCH(1,{1,2;3,4})", "#VALUE!"),
            ("XLOOKUP(1,A1:A5,B1:B5,99,3)", "#VALUE!"),
            ("XMATCH(1,A1:A5,0,2)", "#VALUE!"),
            ("XLOOKUP(1,A1:A5,{#N/A;2;3;4;5},99)", "#N/A"),
            ("XLOOKUP(#N/A,A1:A5,B1:B5,99)", "#N/A"),
            ("XLOOKUP(2,{#N/A,2},{10,20},99)", "#N/A"),
        ] {
            assert_eq!(ev(f), V::Err(error), "{f}");
        }
    }

    #[test]
    fn added_statistics() {
        for (f, expected) in [
            ("MEDIAN(A1:A5)", 3.0),
            ("MEDIAN({4,1,3,2})", 2.5),
            ("MODE({3,2,3,2})", 2.0),
            ("MODE.SNGL({1,2,2,3})", 2.0),
            ("STDEV({1,3})", 2f64.sqrt()),
            ("STDEV.S({1,3})", 2f64.sqrt()),
            ("STDEV.P({1,3})", 1.0),
            ("VAR({1,3})", 2.0),
            ("VAR.S({1,3})", 2.0),
            ("VAR.P({1,3})", 1.0),
            ("LARGE(A1:A5,2)", 4.0),
            ("SMALL(A1:A5,2)", 2.0),
            ("RANK(4,A1:A5)", 2.0),
            ("RANK.EQ(4,A1:A5,1)", 4.0),
            ("RANK.EQ(2,{1,2,2,3})", 2.0),
            ("AVERAGEA({1,TRUE,\"x\"})", 2.0 / 3.0),
            ("AVERAGEA(\"2\",TRUE)", 1.5),
        ] {
            assert_eq!(ev(f), V::Num(expected), "{f}");
        }
        for (f, error) in [
            ("MEDIAN(B1:B5)", "#NUM!"),
            ("MODE(A1:A5)", "#N/A"),
            ("MODE.SNGL(A1:A5)", "#N/A"),
            ("STDEV(1)", "#DIV/0!"),
            ("STDEV.S(1)", "#DIV/0!"),
            ("STDEV.P(B1:B5)", "#DIV/0!"),
            ("VAR(1)", "#DIV/0!"),
            ("VAR.S(1)", "#DIV/0!"),
            ("VAR.P(B1:B5)", "#DIV/0!"),
            ("LARGE(A1:A5,6)", "#NUM!"),
            ("SMALL(A1:A5,0)", "#NUM!"),
            ("RANK(8,A1:A5)", "#N/A"),
            ("RANK.EQ(8,A1:A5)", "#N/A"),
            ("AVERAGEA(Z1:Z3)", "#DIV/0!"),
            ("MEDIAN({1,#N/A})", "#N/A"),
            ("STDEV.S({1E308,-1E308})", "#NUM!"),
        ] {
            assert_eq!(ev(f), V::Err(error), "{f}");
        }
    }

    #[test]
    fn added_maths() {
        for (f, expected) in [
            ("CEILING(4.2,2)", 6.0),
            ("CEILING(-4.2,2)", -4.0),
            ("CEILING(-4.2,-2)", -6.0),
            ("CEILING.MATH(-4.2,2)", -4.0),
            ("CEILING.MATH(-4.2,2,1)", -6.0),
            ("FLOOR(4.2,2)", 4.0),
            ("FLOOR(-4.2,2)", -6.0),
            ("FLOOR(-4.2,-2)", -4.0),
            ("FLOOR.MATH(-4.2,2)", -6.0),
            ("FLOOR.MATH(-4.2,2,1)", -4.0),
            ("MROUND(-7,-2)", -8.0),
            ("MROUND(7,2)", 8.0),
            ("TRUNC(-3.99)", -3.0),
            ("TRUNC(123.456,-1)", 120.0),
            ("LOG(8,2)", 3.0),
            ("LOG(100)", 2.0),
            ("EVEN(-3)", -4.0),
            ("EVEN(0)", 0.0),
            ("ODD(0)", 1.0),
            ("ODD(-2)", -3.0),
            ("SUMSQ(A1:A3)", 14.0),
            ("QUOTIENT(-7,2)", -3.0),
            ("GCD(12.9,18)", 6.0),
            ("GCD(0,0)", 0.0),
            ("LCM(4,6)", 12.0),
            ("LCM(0,5)", 0.0),
            ("FACT(5.9)", 120.0),
            ("FACT(0)", 1.0),
            ("DEGREES(PI())", 180.0),
            ("RADIANS(180)", std::f64::consts::PI),
            ("SIN(0)", 0.0),
            ("COS(0)", 1.0),
            ("TAN(0)", 0.0),
            ("ASIN(0)", 0.0),
            ("ACOS(1)", 0.0),
            ("ATAN(0)", 0.0),
            ("ATAN2(0,1)", std::f64::consts::FRAC_PI_2),
            ("RANDBETWEEN(3,3)", 3.0),
        ] {
            assert_eq!(ev(f), V::Num(expected), "{f}");
        }
        for (f, error) in [
            ("CEILING(1,-1)", "#NUM!"),
            ("FLOOR(1,-1)", "#NUM!"),
            ("FLOOR(1,0)", "#DIV/0!"),
            ("CEILING.MATH(\"x\")", "#VALUE!"),
            ("FLOOR.MATH(\"x\")", "#VALUE!"),
            ("MROUND(1,-1)", "#NUM!"),
            ("TRUNC(\"x\")", "#VALUE!"),
            ("LOG(-1,2)", "#NUM!"),
            ("LOG(2,1)", "#DIV/0!"),
            ("EVEN(\"x\")", "#VALUE!"),
            ("ODD(\"x\")", "#VALUE!"),
            ("RAND(1)", "#VALUE!"),
            ("RANDBETWEEN(3,1)", "#NUM!"),
            ("SUMSQ({1,#N/A})", "#N/A"),
            ("QUOTIENT(1,0)", "#DIV/0!"),
            ("GCD(-1,2)", "#NUM!"),
            ("LCM(-1,2)", "#NUM!"),
            ("LCM(9007199254740991,2)", "#NUM!"),
            ("FACT(171)", "#NUM!"),
            ("DEGREES(\"x\")", "#VALUE!"),
            ("RADIANS(\"x\")", "#VALUE!"),
            ("SIN(\"x\")", "#VALUE!"),
            ("COS(\"x\")", "#VALUE!"),
            ("TAN(\"x\")", "#VALUE!"),
            ("ASIN(2)", "#NUM!"),
            ("ACOS(-2)", "#NUM!"),
            ("ATAN(\"x\")", "#VALUE!"),
            ("ATAN2(0,0)", "#DIV/0!"),
        ] {
            assert_eq!(ev(f), V::Err(error), "{f}");
        }
        let draws: Vec<_> = (0..100).map(|_| ev("RAND()")).collect();
        assert!(draws.iter().all(|v| matches!(v, V::Num(n) if (0.0..1.0).contains(n))));
        assert!(draws.windows(2).any(|pair| pair[0] != pair[1]));
        for _ in 0..100 {
            assert!(matches!(ev("RANDBETWEEN(-3.5,4.5)"), V::Num(n) if (-3.0..=4.0).contains(&n) && n.fract() == 0.0));
        }
    }

    #[test]
    fn added_dates() {
        for (f, expected) in [
            ("EDATE(DATE(2024,1,31),1)", ev("DATE(2024,2,29)")),
            ("EOMONTH(DATE(2024,1,15),1)", ev("DATE(2024,2,29)")),
            ("EDATE(DATE(2024,3,31),-1)", ev("DATE(2024,2,29)")),
            ("DAYS(\"2024-03-01\",\"2024-02-28\")", V::Num(2.0)),
            ("DAYS(2.5,1)", V::Num(1.5)),
            ("DATEDIF(DATE(2020,6,15),DATE(2024,8,20),\"Y\")", V::Num(4.0)),
            ("DATEDIF(DATE(2020,6,15),DATE(2024,8,20),\"M\")", V::Num(50.0)),
            ("DATEDIF(DATE(2024,2,28),DATE(2024,3,1),\"D\")", V::Num(2.0)),
            ("DATEDIF(DATE(2020,6,15),DATE(2024,8,20),\"MD\")", V::Num(5.0)),
            ("DATEDIF(DATE(2020,6,15),DATE(2024,8,20),\"YM\")", V::Num(2.0)),
            ("DATEDIF(DATE(2020,6,15),DATE(2024,8,20),\"YD\")", V::Num(66.0)),
            ("NETWORKDAYS(DATE(2024,1,1),DATE(2024,1,7))", V::Num(5.0)),
            ("NETWORKDAYS(DATE(2024,1,7),DATE(2024,1,1),{45292,45292})", V::Num(-4.0)),
            ("WORKDAY(DATE(2024,1,5),1)", ev("DATE(2024,1,8)")),
            ("WORKDAY(DATE(2024,1,5),1,{45299})", ev("DATE(2024,1,9)")),
            ("WORKDAY(DATE(2024,1,8),-1)", ev("DATE(2024,1,5)")),
            ("WEEKNUM(DATE(2024,1,7))", V::Num(2.0)),
            ("WEEKNUM(DATE(2024,1,7),2)", V::Num(1.0)),
            ("WEEKNUM(DATE(2021,1,1),21)", V::Num(53.0)),
            ("DATEVALUE(\"2024-02-29\")", ev("DATE(2024,2,29)")),
            ("DATEVALUE(\"29-Feb-2024\")", ev("DATE(2024,2,29)")),
            ("DATEVALUE(\"February 29, 2024\")", ev("DATE(2024,2,29)")),
            ("TIMEVALUE(\"12 PM\")", V::Num(0.5)),
            ("DATEVALUE(\"2/29/2024 12:30\")", ev("DATE(2024,2,29)")),
            ("TIMEVALUE(\"12:00 PM\")", V::Num(0.5)),
            ("TIMEVALUE(\"12:00 AM\")", V::Num(0.0)),
            ("TIMEVALUE(\"2024-02-29 12:00:00\")", V::Num(0.5)),
        ] {
            assert_eq!(ev(f), expected, "{f}");
        }
        for (f, error) in [
            ("EDATE(1,-120000)", "#NUM!"),
            ("EOMONTH(1,-120000)", "#NUM!"),
            ("DATEDIF(2,1,\"D\")", "#NUM!"),
            ("DATEDIF(1,2,\"bad\")", "#NUM!"),
            ("DAYS(\"bad\",1)", "#VALUE!"),
            ("NETWORKDAYS(-1,1)", "#NUM!"),
            ("WORKDAY(0,-1)", "#NUM!"),
            ("WEEKNUM(1,3)", "#NUM!"),
            ("DATEVALUE(\"2023-02-29\")", "#VALUE!"),
            ("TIMEVALUE(\"25:00\")", "#VALUE!"),
        ] {
            assert_eq!(ev(f), V::Err(error), "{f}");
        }
        let mut b = book();
        // Date parsing and month arithmetic honor the workbook's date system.
        struct MacBook(Values);
        impl Book for MacBook {
            fn cell(&mut self, s: Option<&str>, r: u32, c: u32) -> V {
                self.0.cell(s, r, c)
            }
            fn extent(&self, s: Option<&str>) -> Option<(u32, u32)> {
                self.0.extent(s)
            }
            fn name(&self, n: &str) -> Option<Expr> {
                self.0.name(n)
            }
            fn date_system(&self) -> DateSystem {
                DateSystem::From1904
            }
            fn now(&self) -> Option<f64> {
                None
            }
        }
        assert_eq!(eval(&parse("DATEVALUE(\"1904-01-01\")").unwrap(), &mut MacBook(std::mem::take(&mut b))), V::Num(0.0));
    }

    #[test]
    fn added_text() {
        for (f, expected) in [
            ("TEXTBEFORE(\"a-b-c\",\"-\")", "a"),
            ("TEXTAFTER(\"a-b-c\",\"-\")", "b-c"),
            ("TEXTBEFORE(\"a-b-c\",\"-\",-1)", "a-b"),
            ("TEXTAFTER(\"a-b-c\",\"-\",2)", "c"),
            ("TEXTBEFORE(\"İA-b\",\"a\",1,1)", "İ"),
            ("TEXTBEFORE(\"abc\",\"-\",1,0,1)", "abc"),
            ("TEXTAFTER(\"abc\",\"-\",,,1)", ""),
            ("TEXTAFTER(\"abc\",\"-\",1,0,0,\"missing\")", "missing"),
            ("CLEAN(CHAR(9)&\"a\"&CHAR(10)&\"b\")", "ab"),
            ("FIXED(1234.567,2)", "1,234.57"),
            ("FIXED(-1234.567,1,TRUE)", "-1234.6"),
            ("FIXED(1234.567,-2)", "1,200"),
            ("DOLLAR(1234.567)", "$1,234.57"),
            ("DOLLAR(-1234.567,1)", "($1,234.6)"),
        ] {
            assert_eq!(ev(f), V::Text(expected.into()), "{f}");
        }
        assert_eq!(ev("NUMBERVALUE(\"1.234,5\",\",\",\".\")"), V::Num(1234.5));
        assert_eq!(ev("NUMBERVALUE(\" 3 000 % \" )"), V::Num(30.0));
        assert_eq!(ev("NUMBERVALUE(\"10%%\")"), V::Num(0.001));
        assert_eq!(ev("NUMBERVALUE(\"\")"), V::Num(0.0));
        for (f, error) in [
            ("TEXTBEFORE(\"abc\",\"-\")", "#N/A"),
            ("TEXTAFTER(\"abc\",\"-\")", "#N/A"),
            ("TEXTBEFORE(\"abc\",\"-\",0)", "#VALUE!"),
            ("CLEAN(#N/A)", "#N/A"),
            ("NUMBERVALUE(\"1.2.3\")", "#VALUE!"),
            ("NUMBERVALUE(\"1\",\".\",\".\")", "#VALUE!"),
            ("FIXED(1,128)", "#VALUE!"),
            ("DOLLAR(\"bad\")", "#VALUE!"),
        ] {
            assert_eq!(ev(f), V::Err(error), "{f}");
        }
    }

    /// A book that counts how often a formula asks how big a sheet is.
    struct Counting(Values, std::cell::Cell<usize>);

    impl Book for Counting {
        fn cell(&mut self, sheet: Option<&str>, row: u32, col: u32) -> V {
            self.0.cell(sheet, row, col)
        }
        fn extent(&self, sheet: Option<&str>) -> Option<(u32, u32)> {
            self.1.set(self.1.get() + 1);
            self.0.extent(sheet)
        }
        fn name(&self, name: &str) -> Option<Expr> {
            self.0.name(name)
        }
        fn date_system(&self) -> DateSystem {
            self.0.date_system()
        }
        fn now(&self) -> Option<f64> {
            None
        }
    }

    #[test]
    fn only_whole_columns_measure_the_sheet() {
        // Measuring a sheet means looking at every row, so a formula with an ordinary range
        // (most of them) mustn't: with thousands of formulas, recalculation would be quadratic.
        let mut b = Counting(book(), Default::default());
        assert_eq!(eval(&parse("SUM(A1:A3)").unwrap(), &mut b), V::Num(6.0));
        assert_eq!(b.1.get(), 0);
        assert!(matches!(eval(&parse("SUM(A:A)").unwrap(), &mut b), V::Num(_)));
        assert_eq!(b.1.get(), 1);
    }

    #[test]
    fn arithmetic_and_coercion() {
        assert_eq!(ev("1+2*3"), V::Num(7.0));
        assert_eq!(ev("-2^2"), V::Num(4.0));
        assert_eq!(ev("10%"), V::Num(0.1));
        assert_eq!(ev("C1+1"), V::Num(4.0), "numeric text is a number in arithmetic");
        assert_eq!(ev("D1+1"), V::Num(2.0));
        assert_eq!(ev("B1+1"), V::Err("#VALUE!"));
        assert_eq!(ev("1/0"), V::Err("#DIV/0!"));
        assert_eq!(ev("\"a\"&1&TRUE"), V::Text("a1TRUE".into()));
        assert_eq!(ev("Z99"), V::Empty);
        assert_eq!(ev("Z99+1"), V::Num(1.0));
        assert_eq!(ev("Rate*100"), V::Num(20.0));
        assert_eq!(ev("Nope*2"), V::Err("#NAME?"));
        assert_eq!(ev("\"B\">\"a\""), V::Bool(true), "text compares without case");
        assert_eq!(ev("1<\"a\""), V::Bool(true), "numbers come before text");
    }

    #[test]
    fn aggregates() {
        assert_eq!(ev("SUM(A1:A5)"), V::Num(15.0));
        assert_eq!(ev("SUM(A1:A5,C1,D1)"), V::Num(15.0), "cell references ignore text and TRUE");
        assert_eq!(ev("SUM(A1:A5,\"3\",TRUE)"), V::Num(19.0), "literal arguments count text and TRUE");
        assert_eq!(ev("SUM(A1:D1)"), V::Num(1.0), "in ranges only numbers count");
        assert_eq!(ev("AVERAGE(A1:A4)"), V::Num(2.5));
        assert_eq!(ev("MAX(A:A)"), V::Num(5.0));
        assert_eq!(ev("COUNT(A1:D5)"), V::Num(5.0));
        assert_eq!(ev("COUNTA(A1:D5)"), V::Num(12.0));
        assert_eq!(ev("SUMIF(B1:B5,\"apple\",A1:A5)"), V::Num(5.0));
        assert_eq!(ev("COUNTIF(A1:A5,\">2\")"), V::Num(3.0));
        assert_eq!(ev("COUNTIF(B1:B5,\"*an*\")"), V::Num(1.0));
        assert_eq!(ev("SUMPRODUCT(A1:A3,A1:A3)"), V::Num(14.0));
        assert_eq!(ev("ROUND(2.675,2)"), V::Num(2.68));
        assert_eq!(ev("ROUND(-2.5,0)"), V::Num(-3.0));
        assert_eq!(ev("ROUNDDOWN(3.99,0)"), V::Num(3.0));
        assert_eq!(ev("MOD(-7,3)"), V::Num(2.0));
    }

    #[test]
    fn logic_text_and_lookup() {
        assert_eq!(ev("IF(A1>0,\"yes\",\"no\")"), V::Text("yes".into()));
        assert_eq!(ev("IF(FALSE,1)"), V::Bool(false));
        assert_eq!(ev("IFERROR(1/0,\"oops\")"), V::Text("oops".into()));
        assert_eq!(ev("AND(A1:A5)"), V::Bool(true));
        assert_eq!(ev("OR(A1>9,D1)"), V::Bool(true));
        assert_eq!(ev("CONCAT(B1:B2)"), V::Text("applebanana".into()));
        assert_eq!(ev("TEXTJOIN(\", \",TRUE,B1:B3)"), V::Text("apple, banana, cherry".into()));
        assert_eq!(ev("UPPER(LEFT(B2,3))&MID(B3,2,3)"), V::Text("BANher".into()));
        assert_eq!(ev("MID(B3,2,1E+30)"), V::Text("herry".into()), "a huge length is the rest of the text");
        assert_eq!(ev("REPT(\"ab\",3)"), V::Text("ababab".into()));
        assert_eq!(ev("REPT(\"x\",1E+18)"), V::Err("#VALUE!"), "more than a cell can hold");
        assert_eq!(ev("TRIM(\"  a   b \")"), V::Text("a b".into()));
        assert_eq!(ev("PROPER(\"hello wORLD\")"), V::Text("Hello World".into()));
        assert_eq!(ev("FIND(\"an\",B2)"), V::Num(2.0));
        assert_eq!(ev("SEARCH(\"N?N\",B2)"), V::Num(3.0));
        assert_eq!(ev("SUBSTITUTE(\"a-b-c\",\"-\",\"+\",2)"), V::Text("a-b+c".into()));
        assert_eq!(ev("TEXT(0.256,\"0.0%\")"), V::Text("25.6%".into()));
        assert_eq!(ev("VLOOKUP(\"cherry\",B1:B5,1,FALSE)"), V::Text("cherry".into()));
        assert_eq!(ev("VLOOKUP(3.5,A1:B5,2)"), V::Text("cherry".into()), "approximate match finds the last not greater");
        assert_eq!(ev("VLOOKUP(\"fig\",B1:B5,1,FALSE)"), V::Err("#N/A"));
        assert_eq!(ev("MATCH(\"date\",B1:B5,0)"), V::Num(5.0));
        assert_eq!(ev("INDEX(B1:B5,MATCH(4,A1:A5,0))"), V::Text("apple".into()));
        assert_eq!(ev("CHOOSE(2,\"a\",\"b\")"), V::Text("b".into()));
        assert_eq!(ev("NOSUCHFUNCTION(1)"), V::Err("#NAME?"));
    }

    #[test]
    fn dates() {
        assert_eq!(ev("DATE(2023,3,15)"), V::Num(45000.0));
        assert_eq!(ev("YEAR(45000)&\"-\"&MONTH(45000)&\"-\"&DAY(45000)"), V::Text("2023-3-15".into()));
        assert_eq!(ev("WEEKDAY(45000)"), V::Num(4.0), "a Wednesday");
        assert_eq!(ev("TIME(18,0,0)"), V::Num(0.75));
        assert_eq!(ev("TODAY()"), V::Err("#N/A"), "no clock given");
    }

    fn nested_ones(terms: usize) -> String {
        let mut formula = String::from("1");
        for _ in 1..terms {
            formula = format!("1+({formula})");
        }
        formula
    }

    #[test]
    fn long_flat_sums_and_concatenation() {
        // A1:A5 are 1..5. A6:A130 are empty, and an empty cell is 0 in arithmetic.
        let cells = (1..=130).map(|row| format!("A{row}")).collect::<Vec<_>>().join("+");
        assert_eq!(ev(&cells), V::Num(15.0));

        let ones = std::iter::repeat_n("1", 200).collect::<Vec<_>>().join("+");
        assert_eq!(ev(&ones), V::Num(200.0));
        let triangle = (1..=200).map(|n| n.to_string()).collect::<Vec<_>>().join("+");
        assert_eq!(ev(&triangle), V::Num((200 * 201 / 2) as f64));

        let cat = std::iter::repeat_n("\"ab\"", 130).collect::<Vec<_>>().join("&");
        assert_eq!(ev(&cat), V::Text("ab".repeat(130)));
    }

    #[test]
    fn spine_keeps_operator_order() {
        assert_eq!(ev("1-2-3"), V::Num(-4.0));
        assert_eq!(ev("2^3^2"), V::Num(64.0));
        assert_eq!(ev("1+2*3+4*5"), V::Num(27.0));
        assert_eq!(ev("\"a\"&1+2&\"b\""), V::Text("a3b".into()));
    }

    #[test]
    fn nested_additions_hit_the_depth_guard() {
        // Real nesting still counts. 128 ones fit. 140 do not.
        assert_eq!(ev(&nested_ones(128)), V::Num(128.0));
        assert_eq!(ev(&nested_ones(140)), V::Err("#REF!"));
    }

    #[test]
    fn deep_unary_hits_the_depth_guard() {
        assert_eq!(ev(&format!("{}1", "-".repeat(127))), V::Num(-1.0));
        assert_eq!(ev(&format!("{}1", "-".repeat(140))), V::Err("#REF!"));
    }

    #[test]
    fn chain_skips_operands_after_an_error() {
        struct Clock(std::cell::Cell<u32>);
        impl Book for Clock {
            fn cell(&mut self, _: Option<&str>, _: u32, _: u32) -> V {
                V::Num(1.0)
            }
            fn extent(&self, _: Option<&str>) -> Option<(u32, u32)> {
                None
            }
            fn name(&self, _: &str) -> Option<Expr> {
                None
            }
            fn date_system(&self) -> DateSystem {
                DateSystem::From1900
            }
            fn now(&self) -> Option<f64> {
                self.0.set(self.0.get() + 1);
                Some(10.0)
            }
        }

        let mut clock = Clock(std::cell::Cell::new(0));
        assert_eq!(eval(&parse("1/0+NOW()+NOW()").unwrap(), &mut clock), V::Err("#DIV/0!"));
        assert_eq!(clock.0.get(), 0, "nothing after the failing left operand runs");

        let mut chain = std::iter::repeat_n("1", 100).collect::<Vec<_>>().join("+");
        chain.push_str("+1/0+NOW()");
        assert_eq!(eval(&parse(&chain).unwrap(), &mut clock), V::Err("#DIV/0!"));
        assert_eq!(clock.0.get(), 0);

        assert_eq!(eval(&parse("NOW()+1/0+NOW()").unwrap(), &mut clock), V::Err("#DIV/0!"));
        assert_eq!(clock.0.get(), 1, "the operand before the error did run, and the one after it did not");
    }
}
