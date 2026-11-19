//! Formulas: reading Excel's formula text into a tree, and moving a formula's relative
//! references the way copying it to another cell does (which is how shared formulas work).

use office_core::model::{CellRef, Range};

/// One end of a reference. A whole column (`A:A`) has no row; a whole row (`1:1`) no column.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RefPart {
    pub row: Option<u32>,
    pub col: Option<u32>,
    pub row_abs: bool,
    pub col_abs: bool,
}

/// A cell or a range, perhaps on another sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub sheet: Option<String>,
    pub start: RefPart,
    pub end: Option<RefPart>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Concat,
    Range,
    Intersect,
    Union,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Number(f64),
    Text(String),
    Bool(bool),
    Error(String),
    Ref(Reference),
    /// A defined name.
    Name(String),
    Neg(Box<Expr>),
    Percent(Box<Expr>),
    Binary(Op, Box<Expr>, Box<Expr>),
    /// A function, by its name in capitals (`_xlfn.` prefixes dropped).
    Call(String, Vec<Expr>),
    /// `{1,2;3,4}`: rows of constants.
    Array(Vec<Vec<Expr>>),
    /// An argument left out: the second in `IF(A1,,0)`.
    Missing,
}

thread_local! {
    static EXPR_DROP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Clears [`EXPR_DROP`] if dropping a chain panics halfway through.
struct ExprDropWalk;
impl Drop for ExprDropWalk {
    fn drop(&mut self) {
        EXPR_DROP.with(|d| d.set(false));
    }
}

/// A long sum is a left-nested `Binary` chain. Derived drop walks that chain
/// with one call per term, which overflows a small stack, so the children are
/// taken out and dropped from a heap list.
impl Drop for Expr {
    fn drop(&mut self) {
        if EXPR_DROP.with(std::cell::Cell::get) || !expr_has_child(self) {
            return;
        }
        EXPR_DROP.with(|d| d.set(true));
        let _walk = ExprDropWalk;
        let mut stack = Vec::new();
        detach_expr(self, &mut stack);
        while let Some(mut child) = stack.pop() {
            detach_expr(&mut child, &mut stack);
        }
    }
}

fn expr_has_child(expr: &Expr) -> bool {
    match expr {
        Expr::Neg(_) | Expr::Percent(_) | Expr::Binary(_, _, _) => true,
        Expr::Call(_, args) => !args.is_empty(),
        Expr::Array(rows) => rows.iter().any(|row| !row.is_empty()),
        _ => false,
    }
}

fn detach_expr(expr: &mut Expr, stack: &mut Vec<Expr>) {
    match expr {
        Expr::Neg(inner) | Expr::Percent(inner) => stack.push(std::mem::replace(inner.as_mut(), Expr::Missing)),
        Expr::Binary(_, left, right) => {
            stack.push(std::mem::replace(left.as_mut(), Expr::Missing));
            stack.push(std::mem::replace(right.as_mut(), Expr::Missing));
        }
        Expr::Call(_, args) => stack.extend(std::mem::take(args)),
        Expr::Array(rows) => {
            for row in rows {
                stack.extend(std::mem::take(row));
            }
        }
        _ => {}
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Number(f64),
    Text(String),
    Error(String),
    Ref(Reference),
    Ident(String),
    Op(String),
    Open,
    Close,
    Comma,
    Semi,
    Space,
    Colon,
    OpenBrace,
    CloseBrace,
}

/// A typed formula with the brackets left open closed at its end, as Excel does on Enter.
pub fn close_brackets(src: &str) -> String {
    let (mut depth, mut quote) = (0usize, None);
    for c in src.chars() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    if quote.is_some() {
        return src.to_string();
    }
    format!("{src}{}", ")".repeat(depth))
}

/// Add Excel's future-function markers for new formulas without changing strings,
/// references, whitespace, or markers already present in imported files.
/// See MS-XLSX section 2.2.3, Functions.
pub fn file_text(src: &str) -> String {
    rewrite_function_names(src, true)
}

/// The formula bar uses Excel's visible names; the model retains the file's text.
pub fn display_text(src: &str) -> String {
    rewrite_function_names(src, false)
}

fn rewrite_function_names(src: &str, for_file: bool) -> String {
    let Ok(tokens) = lex(src) else { return src.to_string() };
    let mut out = src.to_string();
    // Replacing from right to left preserves the original token offsets.
    for pair in tokens.windows(2).rev() {
        let (Tok::Ident(name), span) = &pair[0] else { continue };
        if pair[1].0 != Tok::Open {
            continue;
        }
        let upper = name.to_ascii_uppercase();
        let visible = upper.strip_prefix("_XLFN.").unwrap_or(&upper);
        let visible = visible.strip_prefix("_XLWS.").unwrap_or(visible);
        if for_file {
            if upper == visible
                && matches!(
                    visible,
                    "XLOOKUP"
                        | "XMATCH"
                        | "IFS"
                        | "SWITCH"
                        | "MAXIFS"
                        | "MINIFS"
                        | "STDEV.S"
                        | "STDEV.P"
                        | "VAR.S"
                        | "VAR.P"
                        | "MODE.SNGL"
                        | "RANK.EQ"
                        | "CEILING.MATH"
                        | "FLOOR.MATH"
                        | "DAYS"
                        | "TEXTBEFORE"
                        | "TEXTAFTER"
                        | "CONCAT"
                        | "TEXTJOIN"
                        | "IFNA"
                        | "XOR"
                        | "NUMBERVALUE"
                )
            {
                out.insert_str(span.start, "_xlfn.");
            }
        } else if visible.len() != upper.len() {
            out.replace_range(span.start..span.start + upper.len() - visible.len(), "");
        }
    }
    out
}

const ERRORS: [&str; 8] = ["#NULL!", "#DIV/0!", "#VALUE!", "#REF!", "#NAME?", "#NUM!", "#N/A", "#GETTING_DATA"];

/// Splits formula text into tokens with where each one is (so references can be rewritten in
/// place). Spaces between reference operands are intersection operators.
fn lex(src: &str) -> Result<Vec<(Tok, std::ops::Range<usize>)>, String> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let start = i;
        let c = src[i..].chars().next().unwrap();
        let tok = match c {
            ' ' | '\t' | '\n' | '\r' => {
                while b.get(i).is_some_and(u8::is_ascii_whitespace) {
                    i += 1;
                }
                Tok::Space
            }
            '"' => {
                let mut s = String::new();
                i += 1;
                loop {
                    match src[i..].find('"') {
                        Some(j) => {
                            s.push_str(&src[i..i + j]);
                            i += j + 1;
                            if b.get(i) == Some(&b'"') {
                                s.push('"');
                                i += 1;
                            } else {
                                break;
                            }
                        }
                        None => return Err("a string isn't closed".into()),
                    }
                }
                Tok::Text(s)
            }
            '#' => {
                let e = ERRORS.iter().find(|e| src[i..].to_ascii_uppercase().starts_with(*e)).ok_or("unknown error value")?;
                i += e.len();
                Tok::Error(e.to_string())
            }
            '(' => {
                i += 1;
                Tok::Open
            }
            ')' => {
                i += 1;
                Tok::Close
            }
            ',' => {
                i += 1;
                Tok::Comma
            }
            ':' => {
                i += 1;
                Tok::Colon
            }
            ';' => {
                i += 1;
                Tok::Semi
            }
            '{' => {
                i += 1;
                Tok::OpenBrace
            }
            '}' => {
                i += 1;
                Tok::CloseBrace
            }
            '<' | '>' => {
                let len = if b.get(i + 1) == Some(&b'=') || c == '<' && b.get(i + 1) == Some(&b'>') { 2 } else { 1 };
                let op = &src[i..i + len];
                i += op.len();
                Tok::Op(op.to_string())
            }
            '+' | '-' | '*' | '/' | '^' | '&' | '=' | '%' => {
                i += 1;
                Tok::Op(c.to_string())
            }
            '0'..='9' | '.' => {
                // A number, unless it's a whole-row range (1:3) or a cell further on.
                if let Some((r, len)) = reference(src, i, None) {
                    i += len;
                    Tok::Ref(r)
                } else {
                    let mut j = i;
                    while j < b.len() && (b[j].is_ascii_digit() || b[j] == b'.') {
                        j += 1;
                    }
                    if j < b.len() && (b[j] == b'e' || b[j] == b'E') {
                        let mut k = j + 1;
                        if k < b.len() && (b[k] == b'+' || b[k] == b'-') {
                            k += 1;
                        }
                        if k < b.len() && b[k].is_ascii_digit() {
                            while k < b.len() && b[k].is_ascii_digit() {
                                k += 1;
                            }
                            j = k;
                        }
                    }
                    let n = src[i..j].parse().map_err(|_| format!("not a number: {}", &src[i..j]))?;
                    i = j;
                    Tok::Number(n)
                }
            }
            '\'' => {
                // 'A sheet name'!A1
                let mut j = i + 1;
                let mut name = String::new();
                loop {
                    match b.get(j) {
                        Some(b'\'') if b.get(j + 1) == Some(&b'\'') => {
                            name.push('\'');
                            j += 2;
                        }
                        Some(b'\'') => break,
                        Some(_) => {
                            let ch = src[j..].chars().next().unwrap();
                            name.push(ch);
                            j += ch.len_utf8();
                        }
                        None => return Err("a sheet name isn't closed".into()),
                    }
                }
                if b.get(j + 1) != Some(&b'!') {
                    return Err("a quoted name that isn't a sheet".into());
                }
                let (r, len) = reference(src, j + 2, Some(name)).ok_or("no reference after the sheet name")?;
                i = j + 2 + len;
                Tok::Ref(r)
            }
            c if c == '$' || c == '_' || c == '\\' || c.is_alphabetic() => {
                let mut j = i;
                while j < src.len() {
                    let ch = src[j..].chars().next().unwrap();
                    if ch.is_alphanumeric() || matches!(ch, '_' | '.' | '\\' | '$') {
                        j += ch.len_utf8();
                    } else {
                        break;
                    }
                }
                // Sheet prefixes take priority over cell-like names such as Q1.
                if b.get(j) == Some(&b'!') {
                    let (r, len) = reference(src, j + 1, Some(src[i..j].to_string())).ok_or("no reference after the sheet name")?;
                    i = j + 1 + len;
                    Tok::Ref(r)
                } else if let Some((r, len)) = reference(src, i, None) {
                    i += len;
                    Tok::Ref(r)
                } else {
                    let word = src[i..j].to_string();
                    i = j;
                    Tok::Ident(word)
                }
            }
            c => return Err(format!("unexpected {c:?}")),
        };
        if i <= start {
            return Err("tokenizer made no progress".into());
        }
        out.push((tok, start..i));
    }
    let significant = |at: usize| {
        let before = at.checked_sub(1).and_then(|i| out.get(i)).map(|(t, _)| t);
        let after = out.get(at + 1).map(|(t, _)| t);
        matches!(before, Some(Tok::Ref(_) | Tok::Ident(_) | Tok::Close))
            && matches!(after, Some(Tok::Ref(_) | Tok::Ident(_) | Tok::Open))
            && !matches!((before, after), (Some(Tok::Ident(_)), Some(Tok::Open)))
    };
    Ok(out.iter().enumerate().filter(|(i, (t, _))| *t != Tok::Space || significant(*i)).map(|(_, t)| t.clone()).collect())
}

/// A reference at `at`: `A1`, `$A$1`, `A1:B2`, `A:C`, `1:3`. Not when it's a function's name
/// (`LOG10(`), or runs on into a longer name.
fn reference(src: &str, at: usize, sheet: Option<String>) -> Option<(Reference, usize)> {
    fn part(s: &[u8], mut i: usize) -> Option<(RefPart, usize)> {
        let start = i;
        let col_abs = s.get(i) == Some(&b'$');
        if col_abs {
            i += 1;
        }
        let cs = i;
        while i < s.len() && s[i].is_ascii_alphabetic() {
            i += 1;
        }
        let col = if i > cs && i - cs <= 3 {
            Some(s[cs..i].iter().fold(0u32, |n, c| n * 26 + (c.to_ascii_uppercase() - b'A' + 1) as u32) - 1)
        } else if i == cs {
            None
        } else {
            return None;
        };
        let row_abs = s.get(i) == Some(&b'$');
        let ri = if row_abs { i + 1 } else { i };
        let mut j = ri;
        while j < s.len() && s[j].is_ascii_digit() {
            j += 1;
        }
        let row = if j > ri { std::str::from_utf8(&s[ri..j]).ok()?.parse::<u32>().ok().filter(|r| *r >= 1).map(|r| r - 1) } else { None };
        if j > ri {
            i = j;
        } else if row_abs {
            return None;
        }
        if col.is_none() && row.is_none() || col.is_some_and(|c| c > 16_383) || row.is_some_and(|r| r > 1_048_575) {
            return None;
        }
        Some((RefPart { row, col, row_abs: row_abs && row.is_some(), col_abs: col_abs && col.is_some() }, i - start))
    }
    let s = src.as_bytes();
    let (first, n1) = part(s, at)?;
    // Not a function (LOG10()), a sheet name (Q1!), or a prefix of a longer name.
    let ends_ref = |end| !src[end..].chars().next().is_some_and(|c| c.is_alphanumeric() || matches!(c, '(' | '!' | '_' | '.' | '\\' | '$'));
    let mut len = n1;
    let mut end = None;
    if let Some(tail) = src[at + len..].trim_start().strip_prefix(':') {
        let second_at = src.len() - tail.trim_start().len();
        if let Some((second, n2)) = part(s, second_at).filter(|(_, n)| ends_ref(second_at + n)) {
            // Both ends must be the same kind: cells, whole columns or whole rows.
            let kind = |p: &RefPart| (p.row.is_some(), p.col.is_some());
            if kind(&first) == kind(&second) {
                end = Some(second);
                len = second_at - at + n2;
            }
        }
    }
    // Alone, a whole column or row isn't a reference (A is a name, 1 is a number).
    if end.is_none() && (first.row.is_none() || first.col.is_none()) {
        return None;
    }
    if !ends_ref(at + len) {
        return None;
    }
    Some((Reference { sheet, start: first, end }, len))
}

/// Parses formula text (without its `=`).
pub fn parse(src: &str) -> Result<Expr, String> {
    let toks = lex(src)?;
    let mut p = Parser { toks: toks.into_iter().map(|(t, _)| t).collect(), at: 0, union_allowed: true, depth: 0 };
    let e = p.compare()?;
    if p.at != p.toks.len() {
        return Err(format!("unexpected {:?}", p.toks[p.at]));
    }
    Ok(e)
}

/// How deep parentheses and unary prefixes may nest. A flat sum does not count:
/// its operators sit on a heap stack. This is above the evaluator's 128, so a
/// formula the evaluator can nest still parses. 160 keeps the recursive calls
/// on a WASM stack. A few thousand parentheses fit in Excel's 8,192 characters
/// and would overflow that stack.
const MAX_PARSE_DEPTH: usize = 160;

struct Parser {
    toks: Vec<Tok>,
    at: usize,
    union_allowed: bool,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.at)
    }

    fn op(&self, ops: &[&str]) -> Option<String> {
        match self.peek() {
            Some(Tok::Op(o)) if ops.contains(&o.as_str()) => Some(o.clone()),
            _ => None,
        }
    }

    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_PARSE_DEPTH {
            self.depth -= 1;
            Err("the formula is nested too deeply".into())
        } else {
            Ok(())
        }
    }

    fn compare(&mut self) -> Result<Expr, String> {
        self.enter()?;
        let e = self.infix();
        self.depth -= 1;
        e
    }

    /// Infix operators, left-associative, loosest tier first. The operator stack
    /// is on the heap, so a long sum does not grow the call stack. Parentheses
    /// come back through [`Parser::compare`].
    fn infix(&mut self) -> Result<Expr, String> {
        let mut values = vec![self.operand()?];
        let mut ops: Vec<(u8, Op)> = Vec::new();
        while let Some((prec, op)) = self.infix_op() {
            self.at += 1;
            while ops.last().is_some_and(|(p, _)| *p >= prec) {
                let (_, prev) = ops.pop().unwrap();
                let right = values.pop().unwrap();
                let left = values.pop().unwrap();
                values.push(Expr::Binary(prev, Box::new(left), Box::new(right)));
            }
            ops.push((prec, op));
            values.push(self.operand()?);
        }
        while let Some((_, op)) = ops.pop() {
            let right = values.pop().unwrap();
            let left = values.pop().unwrap();
            values.push(Expr::Binary(op, Box::new(left), Box::new(right)));
        }
        Ok(values.pop().unwrap())
    }

    fn infix_op(&self) -> Option<(u8, Op)> {
        let symbol = match self.peek() {
            Some(Tok::Op(symbol)) => symbol.as_str(),
            _ => return None,
        };
        Some(match symbol {
            "=" => (1, Op::Eq),
            "<>" => (1, Op::Ne),
            "<" => (1, Op::Lt),
            "<=" => (1, Op::Le),
            ">" => (1, Op::Gt),
            ">=" => (1, Op::Ge),
            "&" => (2, Op::Concat),
            "+" => (3, Op::Add),
            "-" => (3, Op::Sub),
            "*" => (4, Op::Mul),
            "/" => (4, Op::Div),
            "^" => (5, Op::Pow),
            _ => return None,
        })
    }

    fn operand(&mut self) -> Result<Expr, String> {
        let mut e = self.unary()?;
        while self.op(&["%"]).is_some() {
            self.at += 1;
            e = Expr::Percent(Box::new(e));
        }
        Ok(e)
    }

    /// Excel's minus binds tighter than ^: -2^2 is 4. Prefixes recurse. The
    /// reference operators below them stay in this frame.
    fn unary(&mut self) -> Result<Expr, String> {
        if let Some(o) = self.op(&["-", "+"]) {
            self.at += 1;
            self.enter()?;
            let e = self.unary();
            self.depth -= 1;
            return e.map(|e| if o == "-" { Expr::Neg(Box::new(e)) } else { e });
        }
        macro_rules! ranges {
            ($parser:ident) => {{
                let mut e = $parser.primary()?;
                while $parser.peek() == Some(&Tok::Colon) {
                    $parser.at += 1;
                    e = Expr::Binary(Op::Range, Box::new(e), Box::new($parser.primary()?));
                }
                e
            }};
        }
        macro_rules! intersections {
            ($parser:ident) => {{
                let mut e = ranges!($parser);
                while $parser.peek() == Some(&Tok::Space) {
                    $parser.at += 1;
                    e = Expr::Binary(Op::Intersect, Box::new(e), Box::new(ranges!($parser)));
                }
                e
            }};
        }
        let mut e = intersections!(self);
        while self.union_allowed && self.peek() == Some(&Tok::Comma) {
            self.at += 1;
            e = Expr::Binary(Op::Union, Box::new(e), Box::new(intersections!(self)));
        }
        Ok(e)
    }

    fn primary(&mut self) -> Result<Expr, String> {
        let tok = self.peek().cloned().ok_or("the formula ends too soon")?;
        self.at += 1;
        Ok(match tok {
            Tok::Number(n) => Expr::Number(n),
            Tok::Text(s) => Expr::Text(s),
            Tok::Error(e) => Expr::Error(e),
            Tok::Ref(r) => Expr::Ref(r),
            Tok::Open => {
                let previous = self.union_allowed;
                self.union_allowed = true;
                let e = self.compare()?;
                self.expect(Tok::Close)?;
                self.union_allowed = previous;
                e
            }
            Tok::OpenBrace => {
                let previous = self.union_allowed;
                self.union_allowed = false;
                let mut rows = vec![Vec::new()];
                loop {
                    rows.last_mut().unwrap().push(self.unary()?);
                    match self.peek() {
                        Some(Tok::Comma) => self.at += 1,
                        Some(Tok::Semi) => {
                            self.at += 1;
                            rows.push(Vec::new());
                        }
                        _ => break,
                    }
                }
                self.expect(Tok::CloseBrace)?;
                if rows.iter().any(|r| r.len() != rows[0].len()) {
                    return Err("ragged array".into());
                }
                self.union_allowed = previous;
                Expr::Array(rows)
            }
            Tok::Ident(name) => {
                if self.peek() == Some(&Tok::Open) {
                    self.at += 1;
                    let previous = self.union_allowed;
                    self.union_allowed = false;
                    let mut args = Vec::new();
                    if self.peek() != Some(&Tok::Close) {
                        loop {
                            if matches!(self.peek(), Some(Tok::Comma | Tok::Close)) {
                                args.push(Expr::Missing);
                            } else {
                                args.push(self.compare()?);
                            }
                            if self.peek() == Some(&Tok::Comma) {
                                self.at += 1;
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(Tok::Close)?;
                    self.union_allowed = previous;
                    let upper = name.to_ascii_uppercase();
                    // Newer functions are saved with a prefix older Excel ignores.
                    let upper = ["_XLFN.", "_XLWS."].iter().fold(upper.as_str(), |n, p| n.strip_prefix(p).unwrap_or(n)).to_string();
                    Expr::Call(upper, args)
                } else if name.eq_ignore_ascii_case("TRUE") {
                    Expr::Bool(true)
                } else if name.eq_ignore_ascii_case("FALSE") {
                    Expr::Bool(false)
                } else {
                    Expr::Name(name)
                }
            }
            t => return Err(format!("unexpected {t:?}")),
        })
    }

    fn expect(&mut self, t: Tok) -> Result<(), String> {
        if self.peek() == Some(&t) {
            self.at += 1;
            Ok(())
        } else {
            Err(format!("expected {t:?}"))
        }
    }
}

fn write_part(p: &RefPart) -> String {
    let mut s = String::new();
    if let Some(c) = p.col {
        if p.col_abs {
            s.push('$');
        }
        s.push_str(&CellRef::column_name(c));
    }
    if let Some(r) = p.row {
        if p.row_abs {
            s.push('$');
        }
        s.push_str(&(r + 1).to_string());
    }
    s
}

fn write_sheet(name: &str) -> String {
    let plain = name.chars().next().is_some_and(|c| c == '_' || c.is_alphabetic())
        && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.');
    if plain {
        format!("{name}!")
    } else {
        format!("'{}'!", name.replace('\'', "''"))
    }
}

/// The source and destination of a cut within one workbook.
pub(crate) struct MovedRange<'a> {
    pub source: Range,
    pub from_sheet: &'a str,
    pub to: CellRef,
    pub to_sheet: &'a str,
}

impl MovedRange<'_> {
    /// Retarget references entirely inside the moved rectangle. References left behind still use
    /// their original sheet, even when the formula itself has moved to another sheet.
    pub fn rewrite(&self, src: &str, old_home: &str, new_home: &str) -> Option<String> {
        let tokens = lex(src).ok()?;
        let mut result = None;
        let mut end = tokens.len();
        while end != 0 {
            let mut start = end - 1;
            if !matches!(tokens[start].0, Tok::Ref(_)) {
                end = start;
                continue;
            }
            // Repeated sheet qualifiers produce separate reference tokens joined by colons.
            while start >= 2 && tokens[start - 1].0 == Tok::Colon && matches!(tokens[start - 2].0, Tok::Ref(_)) {
                start -= 2;
            }
            let refs = tokens[start..end].iter().step_by(2);
            let has_expression_endpoint =
                start.checked_sub(1).is_some_and(|i| tokens[i].0 == Tok::Colon) || tokens.get(end).is_some_and(|(t, _)| *t == Tok::Colon);
            let moved = !has_expression_endpoint
                && refs.clone().all(|(token, _)| {
                    let Tok::Ref(r) = token else { unreachable!() };
                    self.contains(r, old_home)
                });
            for (token, span) in refs.rev() {
                let Tok::Ref(r) = token else { unreachable!() };
                let name = r.sheet.as_deref().unwrap_or(old_home);
                let target = if moved { self.to_sheet } else { name };
                if !moved && (r.sheet.is_some() || target.eq_ignore_ascii_case(new_home)) {
                    continue;
                }
                let qualify = r.sheet.is_some() || !target.eq_ignore_ascii_case(new_home);
                let mut replacement = if qualify { write_sheet(target) } else { String::new() };
                if moved {
                    let translate = |p: RefPart| RefPart {
                        row: p.row.map(|row| self.to.row + (row - self.source.start.row)),
                        col: p.col.map(|col| self.to.col + (col - self.source.start.col)),
                        ..p
                    };
                    replacement.push_str(&write_part(&translate(r.start)));
                    if let Some(end) = r.end {
                        replacement.push(':');
                        replacement.push_str(&write_part(&translate(end)));
                    }
                } else {
                    replacement.push_str(&src[span.clone()]);
                }
                if src[span.clone()] != replacement {
                    result.get_or_insert_with(|| src.to_string()).replace_range(span.clone(), &replacement);
                }
            }
            end = start;
        }
        result
    }

    fn contains(&self, r: &Reference, home: &str) -> bool {
        let name = r.sheet.as_deref().unwrap_or(home);
        let end = r.end.unwrap_or(r.start);
        name.eq_ignore_ascii_case(self.from_sheet)
            && self.source.contains(CellRef::new(r.start.row.unwrap_or(0), r.start.col.unwrap_or(0)))
            && self.source.contains(CellRef::new(end.row.unwrap_or(1_048_575), end.col.unwrap_or(16_383)))
    }
}

/// The formula as it reads copied `rows` down and `cols` across: relative references move,
/// absolute ones (`$A$1`) stay. A reference moved off the sheet becomes `#REF!`.
pub fn shift(src: &str, rows: i64, cols: i64) -> Result<String, String> {
    let toks = lex(src)?;
    let mut out = String::with_capacity(src.len());
    let mut last = 0;
    for (tok, span) in toks {
        let Tok::Ref(r) = tok else { continue };
        out.push_str(&src[last..span.start]);
        last = span.end;
        let mv = |p: &RefPart| -> Option<RefPart> {
            let row = match p.row {
                Some(r) if !p.row_abs => Some(u32::try_from(r as i64 + rows).ok().filter(|r| *r <= 1_048_575)?),
                r => r,
            };
            let col = match p.col {
                Some(c) if !p.col_abs => Some(u32::try_from(c as i64 + cols).ok().filter(|c| *c <= 16_383)?),
                c => c,
            };
            Some(RefPart { row, col, ..*p })
        };
        let moved = (mv(&r.start), r.end.as_ref().map(mv));
        let text = match moved {
            (Some(a), None) => write_part(&a),
            (Some(a), Some(Some(b))) => format!("{}:{}", write_part(&a), write_part(&b)),
            _ => "#REF!".to_string(),
        };
        if let Some(sheet) = &r.sheet {
            if src[span.start..].starts_with('\'') {
                out.push_str(&format!("'{}'!", sheet.replace('\'', "''")));
            } else {
                out.push_str(&write_sheet(sheet));
            }
        }
        out.push_str(&text);
    }
    out.push_str(&src[last..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(row: u32, col: u32) -> RefPart {
        RefPart { row: Some(row), col: Some(col), row_abs: false, col_abs: false }
    }

    #[test]
    fn spaced_ranges_move_only_when_both_endpoints_are_cut() {
        let mut moved =
            MovedRange { source: Range::parse("A1").unwrap(), from_sheet: "Sheet1", to: CellRef::parse("D1").unwrap(), to_sheet: "Sheet1" };
        assert_eq!(moved.rewrite("SUM(A1 : B1)", "Sheet1", "Sheet1"), None);
        assert_eq!(moved.rewrite("SUM(A1 : B1)", "Sheet1", "Sheet2").as_deref(), Some("SUM(Sheet1!A1 : B1)"));
        moved.source = Range::parse("A1:B1").unwrap();
        assert_eq!(moved.rewrite("SUM(A1 : B1)", "Sheet1", "Sheet1").as_deref(), Some("SUM(D1:E1)"));
    }

    #[test]
    fn repeated_sheet_qualifiers_do_not_split_a_cut_range() {
        let mut moved =
            MovedRange { source: Range::parse("A1").unwrap(), from_sheet: "Q1", to: CellRef::parse("D1").unwrap(), to_sheet: "New totals" };
        let formula = "SUM(Q1!A1:Q1!B1)";
        assert_eq!(moved.rewrite(formula, "Q1", "Q1"), None);
        assert_eq!(moved.rewrite("SUM(A1:B1)+SUM(Q1!A1 : 'Q1'!B1)", "Q1", "Other").as_deref(), Some("SUM(Q1!A1:B1)+SUM(Q1!A1 : 'Q1'!B1)"));
        assert_eq!(moved.rewrite("SUM(Q1!A1:B1_Total)", "Q1", "Q1"), None);
        moved.source = Range::parse("A1:B1").unwrap();
        assert_eq!(moved.rewrite(formula, "Q1", "Q1").as_deref(), Some("SUM('New totals'!D1:'New totals'!E1)"));
    }

    #[test]
    fn closes_open_brackets() {
        assert_eq!(close_brackets("SUM(A1:A3"), "SUM(A1:A3)");
        assert_eq!(close_brackets("IF(A1>0,ROUND(A1,"), "IF(A1>0,ROUND(A1,))");
        assert_eq!(close_brackets("LEN(\"(\")"), "LEN(\"(\")");
        assert_eq!(close_brackets("SUM('a(b'!A1"), "SUM('a(b'!A1)");
        assert_eq!(close_brackets("A1+B1"), "A1+B1");
        assert_eq!(close_brackets("LEN(\"open"), "LEN(\"open");
    }

    #[test]
    fn future_function_names_preserve_formula_text() {
        let src = r#"IFNA(XLOOKUP (A1,'_xlfn.IFS'!B1:B3,C1:C3),"_xlfn.XLOOKUP(")"#;
        let stored = r#"_xlfn.IFNA(_xlfn.XLOOKUP (A1,'_xlfn.IFS'!B1:B3,C1:C3),"_xlfn.XLOOKUP(")"#;
        assert_eq!(file_text(src), stored);
        assert_eq!(display_text(stored), src);
        assert_eq!(file_text(stored), stored);
        assert_eq!(file_text("SUMIFS(A1:A2,B1:B2,1)"), "SUMIFS(A1:A2,B1:B2,1)");
        assert_eq!(file_text("stdev.s(A1:A2)+CEILING.MATH(2.2)"), "_xlfn.stdev.s(A1:A2)+_xlfn.CEILING.MATH(2.2)");
        assert_eq!(display_text("_XLFN._XLWS.FILTER(A1:A2,TRUE)"), "FILTER(A1:A2,TRUE)");
        assert_eq!(parse("_xlfn.STDEV.S(1,3)").unwrap(), parse("STDEV.S(1,3)").unwrap());
        assert_eq!(shift("_xlfn.XLOOKUP(A1,B1:B3,C1:C3)", 1, 0).unwrap(), "_xlfn.XLOOKUP(A2,B2:B4,C2:C4)");
        assert_eq!(display_text("_xlfn.unknown(A1)"), "unknown(A1)");
        // Preserve unsupported syntax and markers rather than reconstructing a formula.
        assert_eq!(file_text("Table1[Amount]"), "Table1[Amount]");
    }

    #[test]
    fn parses_precedence() {
        assert_eq!(
            parse("1+2*3").unwrap(),
            Expr::Binary(
                Op::Add,
                Box::new(Expr::Number(1.0)),
                Box::new(Expr::Binary(Op::Mul, Box::new(Expr::Number(2.0)), Box::new(Expr::Number(3.0))))
            )
        );
        assert_eq!(
            parse("-2^2").unwrap(),
            Expr::Binary(Op::Pow, Box::new(Expr::Neg(Box::new(Expr::Number(2.0)))), Box::new(Expr::Number(2.0)))
        );
        assert_eq!(parse("50%").unwrap(), Expr::Percent(Box::new(Expr::Number(50.0))));
        assert!(matches!(parse("A1&\"x\"=B2").unwrap(), Expr::Binary(Op::Eq, _, _)));
    }

    #[test]
    fn parses_references_and_calls() {
        assert_eq!(parse("A1").unwrap(), Expr::Ref(Reference { sheet: None, start: cell(0, 0), end: None }));
        let sheet = parse("'My Sheet'!$B$2:C10").unwrap();
        let Expr::Ref(r) = &sheet else { panic!() };
        assert_eq!(r.sheet.as_deref(), Some("My Sheet"));
        assert!(r.start.row_abs && r.start.col_abs);
        assert_eq!(r.end.as_ref(), Some(&cell(9, 2)));
        let column = parse("Data!A:A").unwrap();
        let Expr::Ref(r) = &column else { panic!() };
        assert_eq!((r.start.row, r.start.col), (None, Some(0)));
        let rows = parse("2:3").unwrap();
        let Expr::Ref(r) = &rows else { panic!() };
        assert_eq!((r.start.row, r.end.as_ref().and_then(|part| part.row)), (Some(1), Some(2)));
        assert_eq!(
            parse("SUM(A1:A3,5)").unwrap(),
            Expr::Call(
                "SUM".into(),
                vec![Expr::Ref(Reference { sheet: None, start: cell(0, 0), end: Some(cell(2, 0)) }), Expr::Number(5.0)]
            )
        );
        assert_eq!(
            parse("_xlfn.CONCAT(\"a\",\"b\"\"c\")").unwrap(),
            Expr::Call("CONCAT".into(), vec![Expr::Text("a".into()), Expr::Text("b\"c".into())])
        );
        assert_eq!(
            parse("IF(A1,,0)").unwrap(),
            Expr::Call(
                "IF".into(),
                vec![Expr::Ref(Reference { sheet: None, start: cell(0, 0), end: None }), Expr::Missing, Expr::Number(0.0)]
            )
        );
        assert_eq!(parse("LOG10(100)").unwrap(), Expr::Call("LOG10".into(), vec![Expr::Number(100.0)]));
        assert_eq!(parse("Rate*TRUE").unwrap(), Expr::Binary(Op::Mul, Box::new(Expr::Name("Rate".into())), Box::new(Expr::Bool(true))));
        assert_eq!(
            parse("{1,2;3,4}").unwrap(),
            Expr::Array(vec![vec![Expr::Number(1.0), Expr::Number(2.0)], vec![Expr::Number(3.0), Expr::Number(4.0)]])
        );
        assert!(matches!(parse("(A1,A2)").unwrap(), Expr::Binary(Op::Union, _, _)));
        assert!(matches!(parse("A1:A2 A1:B1").unwrap(), Expr::Binary(Op::Intersect, _, _)));
        assert!(matches!(parse("(A1):(B1)").unwrap(), Expr::Binary(Op::Range, _, _)));
        for formula in ["A1 : B1_Total", "A1:B1_Total"] {
            assert_eq!(
                parse(formula).unwrap(),
                Expr::Binary(
                    Op::Range,
                    Box::new(Expr::Ref(Reference { sheet: None, start: cell(0, 0), end: None })),
                    Box::new(Expr::Name("B1_Total".into())),
                )
            );
        }
        assert!(matches!(parse("A1 : B1!C1").unwrap(), Expr::Binary(Op::Range, _, _)));
        assert_eq!(parse("#N/A").unwrap(), Expr::Error("#N/A".into()));
        assert_eq!(parse("1.5E+3").unwrap(), Expr::Number(1500.0));
        assert!(parse("SUM(1").is_err());
        assert!(parse("1+").is_err());
    }

    #[test]
    fn shifts_like_copying() {
        assert_eq!(shift("A1+$B$1+B$2+$C3", 2, 1).unwrap(), "B3+$B$1+C$2+$C5");
        assert_eq!(shift("SUM(A1:A10)*Sheet2!B2", 1, 0).unwrap(), "SUM(A2:A11)*Sheet2!B3");
        assert_eq!(shift("'My Sheet'!A1", 0, 1).unwrap(), "'My Sheet'!B1");
        assert_eq!(shift("A:A", 5, 1).unwrap(), "B:B");
        assert_eq!(shift("A1-1", -1, 0).unwrap(), "#REF!-1");
        assert_eq!(shift("\"A1\"&A1", 1, 0).unwrap(), "\"A1\"&A2", "text that looks like a reference isn't one");
    }

    fn add_links(expr: &Expr) -> usize {
        let mut n = 0;
        let mut expr = expr;
        while let Expr::Binary(Op::Add, left, _) = expr {
            n += 1;
            expr = left;
        }
        n
    }

    #[test]
    fn long_flat_chain_parses() {
        // Longer than the evaluator's old 128-link ceiling, and still one flat sum.
        let terms = 300;
        let sum = (1..=terms).map(|n| n.to_string()).collect::<Vec<_>>().join("+");
        let expr = parse(&sum).unwrap();
        assert_eq!(add_links(&expr), terms - 1);
        let cat = std::iter::repeat_n("\"x\"", terms).collect::<Vec<_>>().join("&");
        assert!(matches!(parse(&cat).unwrap(), Expr::Binary(Op::Concat, _, _)));
    }

    #[test]
    fn nested_parentheses_and_unary_prefixes_have_a_limit() {
        assert_eq!(parse(&format!("{}1{}", "(".repeat(128), ")".repeat(128))).unwrap(), Expr::Number(1.0));
        assert!(parse(&format!("{}1{}", "(".repeat(300), ")".repeat(300))).unwrap_err().contains("nested too deeply"));
        assert!(parse(&format!("{}1", "-".repeat(128))).is_ok());
        assert!(parse(&format!("{}1", "+".repeat(128))).is_ok());
        let err = parse(&format!("{}1", "-".repeat(300))).unwrap_err();
        assert!(err.contains("nested too deeply"), "{err}");
        assert!(parse(&format!("{}1", "+".repeat(300))).is_err());
    }

    #[test]
    fn deep_binary_tree_drops() {
        // Far past a thread's stack, if drop walked the left spine by recursion.
        let mut expr = Expr::Number(1.0);
        for _ in 0..20_000 {
            expr = Expr::Binary(Op::Add, Box::new(expr), Box::new(Expr::Number(1.0)));
        }
        drop(expr);
    }
}
