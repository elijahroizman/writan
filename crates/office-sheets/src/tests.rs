use office_core::model::{Body, CellRef, Value};
use office_core::opc::Package;
use office_core::{Compiler, Document, Kind};

use super::*;

fn compiler() -> Compiler {
    Compiler::new().with_layer(Sheets)
}

fn at(s: &str) -> CellRef {
    CellRef::parse(s).unwrap()
}

/// A workbook written by the core, then edited as Excel would have saved it: `edit` changes
/// the parts' XML text.
fn workbook_with(edit: impl Fn(&str, String) -> String) -> Document {
    let mut doc = Document::new(Kind::Sheets);
    let Body::Workbook { sheets } = &mut doc.body else { unreachable!() };
    sheets[0].name = "Data".into();
    sheets.push(office_core::model::Sheet::new("Other"));
    let bytes = office_core::write(&doc, Kind::Sheets).unwrap();
    let mut pkg = Package::read(&bytes).unwrap();
    for part in ["xl/workbook.xml", "xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"] {
        let text = String::from_utf8(pkg.part(part).unwrap().to_vec()).unwrap();
        pkg.replace(part, edit(part, text).into_bytes()).unwrap();
    }
    compiler().read(&pkg.to_bytes().unwrap(), Some("book.xlsx")).unwrap()
}

/// Puts `cells` in a sheet part's empty sheetData, however it's written.
fn fill(text: String, cells: &str) -> String {
    text.replace("<sheetData/>", "<sheetData></sheetData>").replace("<sheetData></sheetData>", &format!("<sheetData>{cells}</sheetData>"))
}

fn cell_xml(cells: &str) -> impl Fn(&str, String) -> String + '_ {
    move |part, text| if part.ends_with("sheet1.xml") { fill(text, cells) } else { text }
}

fn value(doc: &Document, s: usize, cell: &str) -> Value {
    let Body::Workbook { sheets } = &doc.body else { unreachable!() };
    sheets[s].value(at(cell)).clone()
}

#[test]
fn shared_formulas_and_recalculation() {
    // B1:B3 share B1's formula; C1 refers to the other sheet and a defined name.
    let rows = r#"<row r="1"><c r="A1"><v>1</v></c><c r="B1"><f t="shared" ref="B1:B3" si="0">A1*10</f><v>0</v></c><c r="C1"><f>SUM(B1:B3)+Other!A1+Rate</f></c></row><row r="2"><c r="A2"><v>2</v></c><c r="B2"><f t="shared" si="0"/></c></row><row r="3"><c r="A3"><v>3</v></c><c r="B3"><f t="shared" si="0"/></c></row>"#;
    let doc = workbook_with(|part, text| {
        let text = cell_xml(rows)(part, text);
        match part {
            "xl/worksheets/sheet2.xml" => fill(text, r#"<row r="1"><c r="A1"><v>100</v></c></row>"#),
            "xl/workbook.xml" => {
                text.replace("</sheets>", r#"</sheets><definedNames><definedName name="Rate">0.5</definedName></definedNames>"#)
            }
            _ => text,
        }
    });
    assert_eq!(formula(&doc, 0, at("B3")).as_deref(), Some("A3*10"));
    assert_eq!(names(&doc)[0].name, "Rate");
    let mut doc = doc;
    assert_eq!(recalculate(&mut doc, None), 4);
    assert_eq!(value(&doc, 0, "B2"), Value::Number(20.0));
    assert_eq!(value(&doc, 0, "C1"), Value::Number(160.5));

    // Saved, the shared formula stays shared and the new results are there.
    let saved = compiler().write(&doc, Kind::Sheets).unwrap();
    let sheet = String::from_utf8(Package::read(&saved).unwrap().part("xl/worksheets/sheet1.xml").unwrap().to_vec()).unwrap();
    assert!(sheet.contains(r#"<c r="B2"><f t="shared" si="0"/><v>20</v></c>"#), "{sheet}");
}

#[test]
fn circular_references_and_errors() {
    let rows = r#"<row r="1"><c r="A1"><f>B1+1</f></c><c r="B1"><f>A1+1</f></c><c r="C1"><f>1/0</f></c><c r="D1"><f>"x"&amp;C1</f></c><c r="E1"><f>NOPE(</f></c></row>"#;
    let mut doc = workbook_with(cell_xml(rows));
    recalculate(&mut doc, None);
    assert_eq!(value(&doc, 0, "C1"), Value::Error("#DIV/0!".into()));
    assert_eq!(value(&doc, 0, "D1"), Value::Error("#DIV/0!".into()), "errors carry through");
    assert_eq!(value(&doc, 0, "E1"), Value::Error("#NAME?".into()), "a formula that doesn't parse");
    assert_eq!(value(&doc, 0, "A1"), Value::Number(0.0), "a circular reference comes to 0, as in Excel");
    assert_eq!(value(&doc, 0, "B1"), Value::Number(0.0));
}

#[test]
fn circular_references_are_reported_and_come_to_zero() {
    // A1 refers to itself; B1 and C1 refer to each other; D1 only reads the loop; E1 sums a
    // range that holds it.
    let rows = r#"<row r="1"><c r="A1"><f>A1+1</f></c><c r="B1"><f>C1*2</f></c><c r="C1"><f>B1+1</f></c><c r="D1"><f>A1+B1+10</f></c><c r="E1"><f>SUM(A1:E1)</f></c><c r="F1"><f>1+1</f></c></row>"#;
    let mut doc = workbook_with(cell_xml(rows));
    assert_eq!(circular_references(&doc, None), vec![(0, at("A1")), (0, at("B1")), (0, at("C1")), (0, at("E1"))]);
    let report = recalculate_with_report(&mut doc, None);
    assert_eq!(report.formulas, 6);
    assert_eq!(report.circular, vec![(0, at("A1")), (0, at("B1")), (0, at("C1")), (0, at("E1"))]);
    for cell in ["A1", "B1", "C1", "E1"] {
        assert_eq!(value(&doc, 0, cell), Value::Number(0.0), "{cell}");
    }
    assert_eq!(value(&doc, 0, "D1"), Value::Number(10.0), "a formula that reads the loop sees 0");
    assert_eq!(value(&doc, 0, "F1"), Value::Number(2.0));
}

#[test]
fn circular_references_found_however_they_are_entered() {
    // A1 reads B1 and C1, and both lead back to A1; C1 is reached only after B1 is done.
    let rows = r#"<row r="1"><c r="A1"><f>B1+C1</f></c><c r="B1"><f>A1+1</f></c><c r="C1"><f>B1+1</f></c><c r="D1"><f>C1+1</f></c></row>"#;
    let mut doc = workbook_with(cell_xml(rows));
    let report = recalculate_with_report(&mut doc, None);
    assert_eq!(report.circular, vec![(0, at("A1")), (0, at("B1")), (0, at("C1"))]);
    for cell in ["A1", "B1", "C1"] {
        assert_eq!(value(&doc, 0, cell), Value::Number(0.0), "{cell}");
    }
    assert_eq!(value(&doc, 0, "D1"), Value::Number(1.0));
}

#[test]
fn recalculating_long_formulas_does_not_clone_their_recursive_trees() {
    std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(|| {
            for (operator, term, expected) in
                [("+", "1", Value::Number(100_000.0)), ("^", "1", Value::Number(1.0)), ("&amp;", "\"\"", Value::Text(String::new()))]
            {
                let formula = std::iter::repeat_n(term, 100_000).collect::<Vec<_>>().join(operator);
                let rows = format!(r#"<row r="1"><c r="A1"><f>{formula}</f></c></row>"#);
                let mut doc = workbook_with(cell_xml(&rows));
                assert_eq!(recalculate(&mut doc, None), 1);
                assert_eq!(value(&doc, 0, "A1"), expected);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn dates_in_the_1904_system() {
    let rows = r#"<row r="1"><c r="A1" s="1"><v>0</v></c></row>"#;
    let doc = workbook_with(|part, text| {
        let text = cell_xml(rows)(part, text);
        if part == "xl/workbook.xml" {
            text.replace("<bookViews>", r#"<workbookPr date1904="1"/><bookViews>"#)
        } else {
            text
        }
    });
    assert_eq!(excel(&doc).unwrap().date_system, DateSystem::From1904);
    let mut doc = doc;
    let mut f = format_of(&doc, 0, at("A1"));
    f.number_format = "yyyy-mm-dd".into();
    set_format(&mut doc, 0, at("A1"), &f);
    assert_eq!(display(&doc, 0, at("A1")), "1904-01-01");
    assert_eq!(date_of(&doc, 0, at("A1")), Some(DateTime::date(1904, 1, 1)));
}

#[test]
fn formats_widths_and_heights_survive_saving() {
    let mut doc = Document::new(Kind::Sheets);
    let Body::Workbook { sheets } = &mut doc.body else { unreachable!() };
    sheets[0].set(at("A1"), Value::Number(1234.5));
    sheets[0].set(at("B2"), Value::Text("note".into()));
    let mut money = format_of(&doc, 0, at("A1"));
    money.number_format = "\"$\"#,##0.00".into();
    money.font.bold = true;
    money.fill =
        Fill { pattern: "solid".into(), fg: Some(office_core::drawing::Color::rgb(office_core::drawing::Rgb(0xFF, 0xF2, 0xCC))), bg: None };
    set_format(&mut doc, 0, at("A1"), &money);
    set_column_width(&mut doc, 0, 0, Some(20.0));
    set_column_width(&mut doc, 0, 2, Some(8.5));
    set_row_height(&mut doc, 0, 1, Some(30.0));

    let back = compiler().read(&compiler().write(&doc, Kind::Sheets).unwrap(), Some("x.xlsx")).unwrap();
    assert_eq!(display(&back, 0, at("A1")), "$1,234.50");
    assert_eq!(format_of(&back, 0, at("A1")), money);
    assert_eq!(column_width(&back, 0, 0), Some(20.0));
    assert_eq!(column_width(&back, 0, 1), None);
    assert_eq!(column_width(&back, 0, 2), Some(8.5));
    assert_eq!(row_height(&back, 0, 1), Some(30.0));
    assert_eq!(value(&back, 0, "B2"), Value::Text("note".into()));

    // Splitting a column range, and clearing.
    let mut back = back;
    set_column_width(&mut back, 0, 0, None);
    assert_eq!(column_width(&back, 0, 0), None);
    assert_eq!(column_width(&back, 0, 2), Some(8.5));
}

#[test]
fn reading_an_unchanged_workbook_with_the_layer_changes_nothing() {
    let rows = r#"<row r="1"><c r="A1" s="0"><v>1</v></c></row>"#;
    let doc = workbook_with(cell_xml(rows));
    let saved = compiler().write(&doc, Kind::Sheets).unwrap();
    let again = compiler().write(&compiler().read(&saved, Some("x.xlsx")).unwrap(), Kind::Sheets).unwrap();
    assert_eq!(saved, again);
}

#[test]
fn long_chains_of_formulas() {
    // A running total down column A, a chain running the other way in B (each cell needs the
    // one below), and a loop through C far longer than the evaluator goes deep at once.
    let n = 5000;
    let mut rows = String::new();
    for r in 1..=n {
        let a = if r == 1 { "<v>1</v>".to_string() } else { format!("<f>A{}+1</f>", r - 1) };
        let b = if r == n { "<v>1</v>".to_string() } else { format!("<f>B{}+1</f>", r + 1) };
        let c = format!("<f>C{}</f>", if r == n { 1 } else { r + 1 });
        rows.push_str(&format!(r#"<row r="{r}"><c r="A{r}">{a}</c><c r="B{r}">{b}</c><c r="C{r}">{c}</c></row>"#));
    }
    let mut doc = workbook_with(cell_xml(&rows));
    recalculate(&mut doc, None);
    assert_eq!(value(&doc, 0, &format!("A{n}")), Value::Number(n as f64));
    assert_eq!(value(&doc, 0, "B1"), Value::Number(n as f64));
    assert_eq!(value(&doc, 0, "C1"), Value::Number(0.0), "a long circular reference still finishes");
    let circular = circular_references(&doc, None);
    assert_eq!(circular.len(), n as usize, "every cell of it is reported");
    assert!(circular.iter().all(|(_, c)| c.col == 2));
    for r in 1..=n {
        assert_eq!(value(&doc, 0, &format!("C{r}")), Value::Number(0.0), "C{r}");
    }
}
