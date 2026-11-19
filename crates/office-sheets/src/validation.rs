//! Data validation: which values a cell takes, the list it offers and what it says when it's
//! chosen or refused. Rules come from `<dataValidations>` and from the 2010 extension list,
//! where Excel keeps lists that refer to another sheet. They stay in the worksheet's markup
//! and are saved as read.

use office_core::model::{CellRef, Document, Range, Value};
use office_core::xml::{ns, Element};
use serde::Serialize;

use crate::conditional::{compare, scalar, sheets, text_of, truthy, Stored};
use crate::eval::{self, V};
use crate::formula;

/// A cell's validation rule.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Validation {
    /// `list`, `whole`, `decimal`, `date`, `time`, `textLength`, `custom` or `none`.
    pub kind: String,
    pub operator: String,
    pub formula1: Option<String>,
    pub formula2: Option<String>,
    pub allow_blank: bool,
    /// Whether a list shows its drop-down arrow (the file's `showDropDown` says the opposite).
    pub dropdown: bool,
    pub show_input: bool,
    pub prompt_title: Option<String>,
    pub prompt: Option<String>,
    pub show_error: bool,
    /// `stop`, `warning` or `information`.
    pub error_style: String,
    pub error_title: Option<String>,
    pub error: Option<String>,
    ranges: Vec<Range>,
}

/// What the app shows for a cell's validation.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationInfo {
    /// The values a list offers, when the cell has a list with its drop-down showing.
    pub list: Option<Vec<String>>,
    pub prompt_title: Option<String>,
    pub prompt: Option<String>,
}

/// Why a value was refused, and how firmly.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Refusal {
    pub style: String,
    pub title: Option<String>,
    pub message: Option<String>,
}

fn child<'a>(el: &'a Element, local: &str) -> Option<&'a Element> {
    el.elements().find(|e| e.local == local)
}

fn read(el: &Element, sqref: &str) -> Option<Validation> {
    let on = |k: &str| el.attr(k).is_some_and(|v| v == "1" || v == "true");
    let text = |k: &str| el.attr(k).map(str::to_string).filter(|s| !s.is_empty());
    // In the extension list a formula is `<x14:formula1><xm:f>…</xm:f></x14:formula1>`.
    let formula = |k: &str| child(el, k).map(|f| child(f, "f").map(Element::text).unwrap_or_else(|| f.text()));
    let ranges: Vec<Range> = sqref.split_whitespace().filter_map(Range::parse).collect();
    if ranges.is_empty() {
        return None;
    }
    Some(Validation {
        kind: el.attr("type").unwrap_or("none").to_string(),
        operator: el.attr("operator").unwrap_or("between").to_string(),
        formula1: formula("formula1"),
        formula2: formula("formula2"),
        allow_blank: on("allowBlank"),
        dropdown: !on("showDropDown"),
        show_input: on("showInputMessage"),
        prompt_title: text("promptTitle"),
        prompt: text("prompt"),
        show_error: on("showErrorMessage"),
        error_style: el.attr("errorStyle").unwrap_or("stop").to_string(),
        error_title: text("errorTitle"),
        error: text("error"),
        ranges,
    })
}

/// The validation rule on the cell at `at`, if it has one.
pub fn at(doc: &Document, sheet: usize, at: CellRef) -> Option<Validation> {
    let root = sheets(doc).get(sheet)?.origin.get()?;
    let main = root.child(ns::X, "dataValidations").into_iter().flat_map(|l| l.children_named(ns::X, "dataValidation"));
    for el in main {
        if let Some(v) = read(el, el.attr("sqref").unwrap_or_default()).filter(|v| v.ranges.iter().any(|r| r.contains(at))) {
            return Some(v);
        }
    }
    for ext in root.child(ns::X, "extLst").into_iter().flat_map(Element::elements) {
        for list in ext.elements().filter(|e| e.local == "dataValidations") {
            for el in list.elements().filter(|e| e.local == "dataValidation") {
                let sqref = child(el, "sqref").map(Element::text).unwrap_or_default();
                if let Some(v) = read(el, &sqref).filter(|v| v.ranges.iter().any(|r| r.contains(at))) {
                    return Some(v);
                }
            }
        }
    }
    None
}

impl Validation {
    fn eval(&self, book: &mut Stored, src: &str, at: CellRef) -> V {
        let tl = self.ranges.first().map(|r| r.start).unwrap_or_default();
        let Ok(text) = formula::shift(src, i64::from(at.row) - i64::from(tl.row), i64::from(at.col) - i64::from(tl.col)) else {
            return V::Err("#REF!");
        };
        match formula::parse(&text) {
            Ok(e) => eval::eval(&e, book),
            Err(_) => V::Err("#NAME?"),
        }
    }

    /// The values a list rule offers: written out (`"Yes,No"`) or read from cells.
    pub fn items(&self, doc: &Document, sheet: usize, at: CellRef, now: Option<f64>) -> Vec<String> {
        let Some(src) = self.formula1.as_deref().filter(|_| self.kind == "list") else { return Vec::new() };
        if let Some(list) = src.trim().strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
            return list.split(',').map(|s| s.trim().replace("\"\"", "\"")).filter(|s| !s.is_empty()).collect();
        }
        let mut book = Stored::new(doc, sheet, now);
        let mut out = Vec::new();
        match self.eval(&mut book, src, at) {
            V::Range(rows) => {
                for v in rows.into_iter().flatten() {
                    let t = text_of(&v);
                    if !t.is_empty() {
                        out.push(t);
                    }
                }
            }
            v @ (V::Num(_) | V::Text(_) | V::Bool(_)) => out.push(text_of(&v)),
            _ => {}
        }
        out
    }

    /// Whether the rule takes `value` in the cell at `at`.
    pub fn allows(&self, doc: &Document, sheet: usize, at: CellRef, value: &Value, now: Option<f64>) -> bool {
        if value.is_empty() || self.kind == "none" {
            return true;
        }
        let v = V::from_value(value);
        let mut book = Stored::new(doc, sheet, now);
        book.instead = Some((at, value.clone()));
        let bound = |book: &mut Stored, f: &Option<String>| f.as_deref().map(|src| scalar(self.eval(book, src, at)));
        match self.kind.as_str() {
            "list" => {
                let t = text_of(&v).to_lowercase();
                self.items(doc, sheet, at, now).iter().any(|i| i.to_lowercase() == t)
            }
            "custom" => self.formula1.as_deref().is_none_or(|src| truthy(&scalar(self.eval(&mut book, src, at)))),
            kind => {
                let subject = match (kind, &v) {
                    ("textLength", v) => V::Num(text_of(v).chars().count() as f64),
                    ("whole", V::Num(n)) if n.fract() != 0.0 => return false,
                    (_, V::Num(n)) => V::Num(*n),
                    _ => return false,
                };
                let (a, b) = (bound(&mut book, &self.formula1), bound(&mut book, &self.formula2));
                let Some(a) = a else { return true };
                use std::cmp::Ordering::*;
                let cmp = |x: &V| compare(&subject, x);
                match self.operator.as_str() {
                    "equal" => cmp(&a) == Some(Equal),
                    "notEqual" => matches!(cmp(&a), Some(Less | Greater)),
                    "greaterThan" => cmp(&a) == Some(Greater),
                    "lessThan" => cmp(&a) == Some(Less),
                    "greaterThanOrEqual" => matches!(cmp(&a), Some(Greater | Equal)),
                    "lessThanOrEqual" => matches!(cmp(&a), Some(Less | Equal)),
                    op => {
                        let Some(b) = b else { return true };
                        let (Some(from), Some(to)) = (cmp(&a), cmp(&b)) else { return false };
                        let inside = from != Less && to != Greater;
                        inside == (op != "notBetween")
                    }
                }
            }
        }
    }

    /// What the app shows for this rule on the cell at `at`.
    pub fn info(&self, doc: &Document, sheet: usize, at: CellRef, now: Option<f64>) -> ValidationInfo {
        ValidationInfo {
            list: (self.kind == "list" && self.dropdown).then(|| self.items(doc, sheet, at, now)),
            prompt_title: self.prompt_title.clone().filter(|_| self.show_input),
            prompt: self.prompt.clone().filter(|_| self.show_input),
        }
    }
}

/// Why `value` can't go in the cell at `at`, when its validation refuses it and says so.
pub fn refusal(doc: &Document, sheet: usize, at: CellRef, value: &Value, now: Option<f64>) -> Option<Refusal> {
    let v = self::at(doc, sheet, at)?;
    if !v.show_error || v.allows(doc, sheet, at, value, now) {
        return None;
    }
    Some(Refusal { style: v.error_style.clone(), title: v.error_title.clone(), message: v.error.clone() })
}

#[cfg(test)]
mod tests {
    use office_core::model::{CellRef, Value};

    use super::*;
    use crate::conditional::tests::{book, n, row, s};

    const X14: &str = r#"xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main""#;

    #[test]
    fn lists_ranges_and_messages() {
        let rules = format!(
            r#"<dataValidations count="2"><dataValidation type="list" allowBlank="1" showInputMessage="1" showErrorMessage="1" errorTitle="Pick one" error="Choose from the list" promptTitle="Size" prompt="S, M or L" sqref="A1:A5"><formula1>"S,M,L"</formula1></dataValidation><dataValidation type="whole" operator="between" showErrorMessage="1" errorStyle="warning" sqref="B1"><formula1>1</formula1><formula2>10</formula2></dataValidation></dataValidations><extLst><ext uri="{{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}}" {X14}><x14:dataValidations count="1"><x14:dataValidation type="list" allowBlank="1" showErrorMessage="1"><x14:formula1><xm:f>Other!$A$1:$A$3</xm:f></x14:formula1><xm:sqref>C1</xm:sqref></x14:dataValidation></x14:dataValidations></ext></extLst>"#
        );
        let other = row(1, &[s("A1", "North")]) + &row(2, &[s("A2", "South")]) + &row(3, &[n("A3", 3.0)]);
        let doc = book(&row(1, &[s("A1", "M")]), &rules, &other, "");
        let a2 = CellRef::new(1, 0);
        let v = at(&doc, 0, a2).unwrap();
        let info = v.info(&doc, 0, a2, None);
        assert_eq!(info.list, Some(vec!["S".to_string(), "M".into(), "L".into()]));
        assert_eq!(info.prompt.as_deref(), Some("S, M or L"));
        assert!(refusal(&doc, 0, a2, &Value::Text("m".into()), None).is_none());
        let no = refusal(&doc, 0, a2, &Value::Text("XL".into()), None).unwrap();
        assert_eq!((no.style.as_str(), no.message.as_deref()), ("stop", Some("Choose from the list")));
        let b1 = CellRef::new(0, 1);
        assert!(refusal(&doc, 0, b1, &Value::Number(4.0), None).is_none());
        assert_eq!(refusal(&doc, 0, b1, &Value::Number(4.5), None).map(|r| r.style), Some("warning".into()));
        assert!(refusal(&doc, 0, b1, &Value::Number(11.0), None).is_some());
        let c1 = CellRef::new(0, 2);
        assert_eq!(at(&doc, 0, c1).unwrap().items(&doc, 0, c1, None), vec!["North", "South", "3"]);
        assert!(refusal(&doc, 0, c1, &Value::Text("East".into()), None).is_some());
        assert!(at(&doc, 0, CellRef::new(9, 9)).is_none());
    }
}
