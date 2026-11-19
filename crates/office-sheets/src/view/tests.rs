use office_core::model::{CellRef, Origin, Range, Value};
use office_core::opc::Package;
use office_core::xml::{ns, Element};
use office_engine::display::Item;
use office_engine::measure::FixedMeasurer;

use super::*;

fn texts(items: &[Item]) -> Vec<String> {
    let mut out = Vec::new();
    for i in items {
        match i {
            Item::Text { text, .. } => out.push(text.clone()),
            Item::Group { items, .. } => out.extend(texts(items)),
            _ => {}
        }
    }
    out
}

#[test]
fn csv_opens_draws_edits_and_saves() {
    let mut host = FixedMeasurer::default();
    let mut v = SheetsView::open(b"name,qty\nApple,3\nPear,4\n", "fruit.csv").unwrap();
    assert_eq!(v.kind, Kind::Csv);
    let p = v.paint(&mut host, 0, 0, 10, 0, 5);
    assert_eq!(texts(&p.items), vec!["name", "qty", "Apple", "3", "Pear", "4"]);
    v.set_input(0, CellRef::new(3, 0), "Total");
    v.set_input(0, CellRef::new(3, 1), "=SUM(B2:B3)");
    assert_eq!(v.cell_info(0, CellRef::new(3, 1)).display, "7");
    assert_eq!(v.cell_info(0, CellRef::new(3, 1)).input, "=SUM(B2:B3)");
    v.set_input(0, CellRef::new(1, 1), "10");
    assert_eq!(v.cell_info(0, CellRef::new(3, 1)).display, "14");
    v.undo();
    assert_eq!(v.cell_info(0, CellRef::new(3, 1)).display, "7");
    let out = String::from_utf8(v.save().unwrap()).unwrap();
    assert!(out.contains("Total,7"), "{out}");
}

#[test]
fn setting_the_clock_preserves_imported_formula_caches() {
    let bytes = SheetsView::blank(Kind::Sheets).save().unwrap();
    let mut package = Package::read(&bytes).unwrap();
    let source = format!(
        r#"<worksheet xmlns="{}"><sheetData><row r="1"><c r="A1"><f>_xlfn.ANCHORARRAY(B1)</f><v>42</v></c><c r="B1"><f>A1*2</f><v>84</v></c><c r="C1"><f>NOW()</f><v>40000</v></c></row></sheetData></worksheet>"#,
        ns::X
    );
    package.replace("xl/worksheets/sheet1.xml", source.into_bytes()).unwrap();
    let bytes = package.to_bytes().unwrap();
    let mut view = SheetsView::open(&bytes, "cached.xlsx").unwrap();
    view.set_now(46000.5);
    assert!(!view.is_dirty());
    for (col, expected) in [(0, 42.0), (1, 84.0), (2, 40000.0)] {
        assert_eq!(view.sheets()[0].value(CellRef::new(0, col)), &Value::Number(expected));
    }
    let saved = Package::read(&view.save().unwrap()).unwrap();
    for part in package.part_names() {
        assert_eq!(saved.part(part).unwrap(), package.part(part).unwrap(), "{part} changed on open/save");
    }

    // A new formula still uses the clock supplied by the app.
    let mut view = SheetsView::blank(Kind::Sheets);
    view.set_now(46000.5);
    view.set_input(0, CellRef::new(0, 0), "=NOW()");
    assert_eq!(view.sheets()[0].value(CellRef::new(0, 0)), &Value::Number(46000.5));
}

#[test]
fn setting_the_clock_calculates_formulas_without_cached_results() {
    // As openpyxl writes them: formulas with no result, or an empty `v`. B1's cached result
    // is kept even though it is wrong; C1's empty text result is a result.
    let bytes = SheetsView::blank(Kind::Sheets).save().unwrap();
    let mut package = Package::read(&bytes).unwrap();
    let source = format!(
        r#"<worksheet xmlns="{}"><sheetData><row r="1"><c r="A1"><v>2</v></c><c r="B1"><f>A1*100</f><v>7</v></c><c r="C1" t="str"><f>"x"&amp;NOPE()</f><v></v></c></row><row r="2"><c r="A2"><v>3</v></c><c r="B2"><f>SUM(A1:A2)</f></c><c r="C2"><f>B2+B1</f><v></v></c><c r="D2"><f>NOW()</f></c></row></sheetData></worksheet>"#,
        ns::X
    );
    package.replace("xl/worksheets/sheet1.xml", source.into_bytes()).unwrap();
    let bytes = package.to_bytes().unwrap();
    let mut view = SheetsView::open(&bytes, "openpyxl.xlsx").unwrap();
    let revision = view.revision();
    view.set_now(46000.5);
    assert!(!view.is_dirty());
    assert!(view.revision() > revision, "the grid is drawn again with the new results");
    assert_eq!(view.cell_info(0, CellRef::new(1, 1)).display, "5");
    assert_eq!(view.cell_info(0, CellRef::new(1, 2)).display, "12", "uses B1's cached result");
    assert_eq!(view.sheets()[0].value(CellRef::new(1, 3)), &Value::Number(46000.5));
    assert_eq!(view.sheets()[0].value(CellRef::new(0, 1)), &Value::Number(7.0));
    assert_eq!(view.sheets()[0].value(CellRef::new(0, 2)), &Value::Empty);

    // Saved, the results are stored, so they show on the next open before the clock is set.
    let saved = view.save().unwrap();
    let xml = sheet_xml(&saved, "xl/worksheets/sheet1.xml");
    assert!(xml.contains(r#"<c r="C1" t="str"><f>"x"&amp;NOPE()</f><v/></c>"#), "{xml}");
    let mut back = SheetsView::open(&saved, "openpyxl.xlsx").unwrap();
    assert_eq!(back.cell_info(0, CellRef::new(1, 1)).input, "=SUM(A1:A2)");
    assert_eq!(back.cell_info(0, CellRef::new(1, 1)).display, "5");
    assert_eq!(back.cell_info(0, CellRef::new(1, 2)).display, "12");
    assert_eq!(back.sheets()[0].value(CellRef::new(0, 1)), &Value::Number(7.0));
    back.set_now(46000.5);
    assert_eq!(back.sheets()[0].value(CellRef::new(0, 2)), &Value::Empty, "the empty text result is kept");
}

#[test]
fn workbook_formats_columns_and_sheets() {
    let mut host = FixedMeasurer::default();
    let mut v = SheetsView::blank(Kind::Sheets);
    v.set_input(0, CellRef::new(0, 0), "12%");
    assert_eq!(v.cell_info(0, CellRef::new(0, 0)).display, "12%");
    v.set_input(0, CellRef::new(1, 0), "2026-01-31");
    assert_eq!(v.cell_info(0, CellRef::new(1, 0)).display, "2026-01-31");
    let a1 = Range { start: CellRef::new(0, 0), end: CellRef::new(0, 0) };
    v.format(0, a1, &FormatPatch { bold: Some(true), fill: Some("FFFF00".into()), ..Default::default() });
    let info = v.cell_info(0, CellRef::new(0, 0));
    assert!(info.bold);
    assert_eq!(info.fill.as_deref(), Some("FFFF00"));
    v.set_column_width(0, 0, Some(120.0));
    assert_eq!(v.grid(0).width(0), 120.0);
    v.shift_rows(0, 0, 1);
    assert_eq!(v.cell_info(0, CellRef::new(1, 0)).display, "12%");
    v.toggle_merge(0, Range { start: CellRef::new(5, 0), end: CellRef::new(6, 2) });
    assert!(v.cell_info(0, CellRef::new(6, 1)).merged);
    v.add_sheet();
    v.rename_sheet(1, "Data");
    assert_eq!(v.sheets()[1].name, "Data");
    v.paste(1, Range { start: CellRef::new(0, 0), end: CellRef::new(0, 0) }, "a\tb\n1\t2\n", None);
    let (text, clipboard) = v.copy(1, Range { start: CellRef::new(0, 0), end: CellRef::new(1, 1) }, None);
    assert_eq!(text, "a\tb\n1\t2\n");
    assert!(clipboard.is_none());
    let found = v.find("b");
    assert_eq!(found, vec![(1, CellRef::new(0, 1))]);
    let bytes = v.save().unwrap();
    let mut back = SheetsView::open(&bytes, "Book.xlsx").unwrap();
    assert_eq!(back.sheets().len(), 2);
    assert!(back.cell_info(0, CellRef::new(1, 0)).bold);
    assert_eq!(back.grid(0).width(0), 120.0);
    assert!(!back.paint(&mut host, 0, 0, 10, 0, 5).items.is_empty());
}

#[test]
fn clipboard_copies_formula_blocks_and_keeps_tab_separated_values() {
    let mut v = SheetsView::blank(Kind::Sheets);
    let source = CellRef::new(1, 1);
    let dest = CellRef::new(3, 3);
    v.set_input(0, CellRef::new(0, 0), "3");
    v.set_input(0, source, "=A1+$A1+A$1+$A$1");
    v.set_input(0, CellRef::new(1, 2), "4");
    v.set_input(0, CellRef::new(2, 1), "line one\nline two");
    v.set_input(0, CellRef::new(4, 4), "old");

    let (text, clipboard) = v.copy(0, Range { start: source, end: CellRef::new(2, 2) }, None);
    assert_eq!(text, "12\t4\nline one\nline two\t\n");
    v.paste(0, Range { start: dest, end: dest }, &text, clipboard);
    assert_eq!(crate::formula(&v.doc, 0, dest).as_deref(), Some("C3+$A3+C$1+$A$1"));
    assert_eq!(v.cell_info(0, CellRef::new(3, 4)).input, "4");
    assert_eq!(v.cell_info(0, CellRef::new(4, 3)).input, "line one\nline two");
    assert_eq!(v.cell_info(0, CellRef::new(4, 4)).input, "");
    assert!(v.undo());
    assert_eq!(v.cell_info(0, CellRef::new(4, 4)).input, "old");
    assert!(v.redo());
    assert_eq!(crate::formula(&v.doc, 0, dest).as_deref(), Some("C3+$A3+C$1+$A$1"));
}

#[test]
fn pasting_one_copied_formula_shifts_it_for_every_selected_cell() {
    let mut v = SheetsView::blank(Kind::Sheets);
    let source = CellRef::new(1, 1);
    v.set_input(0, source, "=B1");
    let (text, clipboard) = v.copy(0, Range { start: source, end: source }, None);
    v.paste(0, Range { start: CellRef::new(2, 2), end: CellRef::new(3, 2) }, &text, clipboard);
    assert_eq!(crate::formula(&v.doc, 0, CellRef::new(2, 2)).as_deref(), Some("C2"));
    assert_eq!(crate::formula(&v.doc, 0, CellRef::new(3, 2)).as_deref(), Some("C3"));
}

#[test]
fn cutting_a_formula_keeps_its_original_references() {
    let mut v = SheetsView::blank(Kind::Sheets);
    let source = CellRef::new(0, 1);
    let dest = CellRef::new(0, 2);
    let source_range = Range { start: source, end: source };
    v.set_input(0, CellRef::new(0, 0), "5");
    v.set_input(0, source, "=A1");

    let (text, clipboard) = v.copy(0, source_range, Some("cut"));
    v.cut(0, source_range, "cut".into());
    v.paste(0, Range { start: dest, end: dest }, &text, clipboard);
    assert_eq!(crate::formula(&v.doc, 0, source), None);
    assert_eq!(crate::formula(&v.doc, 0, dest).as_deref(), Some("A1"));
    v.set_input(0, CellRef::new(0, 0), "9");
    assert_eq!(v.cell_info(0, dest).display, "9");
}

#[test]
fn cutting_a_formula_across_sheets_keeps_unmoved_dependencies_on_the_source_sheet() {
    let mut v = SheetsView::blank(Kind::Sheets);
    v.rename_sheet(0, "Owner's Data");
    let a1 = CellRef::new(0, 0);
    let b1 = CellRef::new(0, 1);
    let c1 = CellRef::new(0, 2);
    let d1 = CellRef::new(0, 3);
    let source = Range { start: b1, end: b1 };
    v.set_input(0, a1, "5");
    v.set_input(0, b1, "=$A$1");
    v.set_input(0, d1, "=B1");
    let (text, clipboard) = v.copy(0, source, Some("cut"));
    v.cut(0, source, "cut".into());
    v.add_sheet();
    v.rename_sheet(1, "New totals");
    v.set_input(0, a1, "7");
    v.set_input(1, a1, "99");
    v.paste(1, Range { start: c1, end: c1 }, &text, clipboard);

    assert_eq!(v.cell_info(1, c1).input, "='Owner''s Data'!$A$1");
    assert_eq!(v.cell_info(1, c1).display, "7");
    assert_eq!(v.cell_info(0, d1).input, "='New totals'!C1");
    assert_eq!(v.cell_info(0, d1).display, "7");
}

#[test]
fn replacing_cut_source_leaves_its_dependents_at_that_address() {
    let mut v = SheetsView::blank(Kind::Sheets);
    let a1 = CellRef::new(0, 0);
    let b1 = CellRef::new(0, 1);
    let source = Range::parse("A1").unwrap();
    v.set_input(0, a1, "5");
    v.set_input(0, b1, "=A1");
    let (text, clipboard) = v.copy(0, source, Some("cut"));
    v.cut(0, source, "cut".into());
    v.set_input(0, a1, "7");
    v.clear(0, source);
    v.paste(0, Range::parse("C1").unwrap(), &text, clipboard);
    assert_eq!(v.cell_info(0, b1).input, "=A1");
}

#[test]
fn cutting_a_block_remaps_absolute_and_range_dependencies_and_updates_other_formulas() {
    let mut v = SheetsView::blank(Kind::Sheets);
    v.add_sheet();
    let source = Range { start: CellRef::new(0, 0), end: CellRef::new(1, 1) };
    let dest = CellRef::new(1, 0);
    let stationary = CellRef::new(0, 5);
    v.set_input(0, CellRef::new(0, 0), "5");
    v.set_input(0, CellRef::new(1, 0), "7");
    v.set_input(0, CellRef::new(0, 1), "=sum($A$1:A2)");
    v.set_input(0, stationary, "=B1");
    v.set_input(1, stationary, "=Sheet1!$B$1");

    let (text, clipboard) = v.copy(0, source, Some("cut"));
    v.cut(0, source, "cut".into());
    v.paste(0, Range { start: dest, end: dest }, &text, clipboard);

    assert_eq!(v.cell_info(0, CellRef::new(1, 1)).input, "=sum($A$2:A3)");
    assert_eq!(v.cell_info(0, stationary).input, "=B2");
    assert_eq!(v.cell_info(1, stationary).input, "=Sheet1!$B$2");
    assert_eq!(v.cell_info(1, stationary).display, "12");
    assert!(v.undo());
    assert_eq!(v.cell_info(1, stationary).input, "=Sheet1!$B$1");
    assert_eq!(v.cell_info(0, CellRef::new(1, 1)).input, "");
    assert!(v.redo());
    assert_eq!(v.cell_info(1, stationary).display, "12");
}

#[test]
fn cutting_values_retargets_formula_dependents_across_sheets_once() {
    let mut v = SheetsView::blank(Kind::Sheets);
    v.add_sheet();
    let a1 = CellRef::new(0, 0);
    let b1 = CellRef::new(0, 1);
    let c3 = CellRef::new(2, 2);
    let d4 = CellRef::new(3, 3);
    let source = Range { start: a1, end: a1 };
    let selection = Range { start: c3, end: d4 };
    v.set_input(0, a1, "0.125");
    v.format(0, source, &FormatPatch { number_format: Some("0%".into()), ..Default::default() });
    v.set_input(0, b1, "=A1");
    v.set_input(1, b1, "=Sheet1!A1");
    let (text, clipboard) = v.copy(0, source, Some("cut"));
    let (_, repeated) = v.copy(0, source, Some("cut"));
    v.cut(0, source, "cut".into());
    v.paste(1, selection, &text, clipboard);
    assert_eq!(v.cell_info(0, b1).input, "=Sheet2!C3");
    assert_eq!(v.cell_info(1, b1).input, "=Sheet2!C3");
    assert_eq!(v.cell_info(0, b1).display, "0.125");
    assert_eq!(v.cell_info(1, c3).number_format, "0%");
    assert_eq!(v.cell_info(1, d4).input, "");
    v.paste(1, Range { start: d4, end: d4 }, &text, repeated);
    assert_eq!(v.cell_info(0, b1).input, "=Sheet2!C3");
    v.set_input(1, c3, "2");
    assert_eq!(v.cell_info(0, b1).display, "2");
    assert_eq!(v.sheets()[1].value(d4), &Value::Number(0.125));
}

#[test]
fn cutting_all_bounds_of_a_nested_range_preserves_its_value() {
    let mut v = SheetsView::blank(Kind::Sheets);
    v.add_sheet();
    let source = Range::parse("A1:C1").unwrap();
    let dest = Range::parse("D1").unwrap();
    let dependent = CellRef::new(0, 3);
    for col in 0..3 {
        v.set_input(0, CellRef::new(0, col), "2");
    }
    v.set_input(0, dependent, "=SUM((A1:B1):C1)");
    assert_eq!(v.cell_info(0, dependent).display, "6");
    let (text, clipboard) = v.copy(0, source, Some("cut"));
    v.cut(0, source, "cut".into());
    v.paste(1, dest, &text, clipboard);
    assert_eq!(v.cell_info(0, dependent).input, "=SUM((Sheet2!D1:E1):Sheet2!F1)");
    assert_eq!(v.cell_info(0, dependent).display, "6");
}

#[test]
fn cutting_dates_between_workbooks_preserves_the_calendar_date() {
    use crate::DateSystem::{From1900, From1904};
    for (from, to, serial, expected) in [(From1900, From1904, 45_000.5, 43_538.5), (From1904, From1900, 43_538.5, 45_000.5)] {
        let mut source = SheetsView::blank(Kind::Sheets);
        let mut target = SheetsView::blank(Kind::Sheets);
        crate::excel_mut(&mut source.doc).date_system = from;
        crate::excel_mut(&mut target.doc).date_system = to;
        let a1 = CellRef::new(0, 0);
        let a2 = CellRef::new(1, 0);
        let a3 = CellRef::new(2, 0);
        for (at, value, format) in [(a1, serial, "yyyy-mm-dd hh:mm"), (a2, 1.5, "[h]:mm"), (a3, 0.5, "hh:mm")] {
            source.set_input(0, at, &value.to_string());
            source.format(0, Range { start: at, end: at }, &FormatPatch { number_format: Some(format.into()), ..Default::default() });
        }
        let range = Range { start: a1, end: a3 };
        let (text, clipboard) = source.copy(0, range, Some("cut"));
        source.cut(0, range, "cut".into());
        target.paste(0, Range { start: a1, end: a1 }, &text, clipboard);
        assert_eq!(target.cell_info(0, a1).display, "2023-03-15 12:00");
        assert_eq!(target.sheets()[0].value(a1), &Value::Number(expected));
        assert_eq!(target.sheets()[0].value(a2), &Value::Number(1.5));
        assert_eq!(target.sheets()[0].value(a3), &Value::Number(0.5));
    }
}

#[test]
fn cutting_a_shared_formula_leaves_its_followers_working() {
    let mut v = SheetsView::blank(Kind::Sheets);
    put_shared(&mut v, CellRef::new(0, 1), Value::Number(0.0), Some("A1+1"), "0", Some("B1:B2"));
    put_shared(&mut v, CellRef::new(1, 1), Value::Number(0.0), None, "0", None);
    let leader = Range::parse("B1").unwrap();
    let (text, clipboard) = v.copy(0, leader, Some("cut"));
    v.cut(0, leader, "cut".into());
    v.paste(0, Range::parse("D1").unwrap(), &text, clipboard);
    let follower = CellRef::parse("B2").unwrap();
    assert_eq!(formula_text(&v, follower).as_deref(), Some("A2+1"));
    v.set_input(0, CellRef::parse("A2").unwrap(), "3");
    assert_eq!(v.cell_info(0, follower).display, "4");
}

#[test]
fn clipboard_copies_shared_formula_followers_without_the_leader() {
    let mut v = SheetsView::blank(Kind::Sheets);
    put_shared(&mut v, CellRef::new(0, 1), Value::Number(0.0), Some("A1+1"), "0", Some("B1:B3"));
    put_shared(&mut v, CellRef::new(1, 1), Value::Number(0.0), None, "0", None);
    put_shared(&mut v, CellRef::new(2, 1), Value::Number(0.0), None, "0", None);
    let dest = CellRef::new(1, 3);
    let (text, clipboard) = v.copy(0, Range { start: CellRef::new(1, 1), end: CellRef::new(2, 1) }, None);
    v.paste(0, Range { start: dest, end: dest }, &text, clipboard);
    assert_eq!(formula_text(&v, dest).as_deref(), Some("C2+1"));
    assert_eq!(formula_text(&v, CellRef::new(2, 3)).as_deref(), Some("C3+1"));
}

#[test]
fn typed_currency_uses_that_symbol_and_impossible_dates_stay_text() {
    let mut v = SheetsView::blank(Kind::Sheets);
    v.set_input(0, CellRef::new(0, 0), "€5");
    v.set_input(0, CellRef::new(0, 1), "£2.5");
    v.set_input(0, CellRef::new(0, 2), "$9");
    v.set_input(0, CellRef::new(1, 0), "-€5");
    v.set_input(0, CellRef::new(1, 1), "(£4)");
    assert_eq!(v.cell_info(0, CellRef::new(0, 0)).display, "€5.00");
    assert_eq!(v.cell_info(0, CellRef::new(0, 1)).display, "£2.50");
    assert_eq!(v.cell_info(0, CellRef::new(0, 2)).display, "$9.00");
    assert_eq!(v.cell_info(0, CellRef::new(1, 0)).display, "-€5.00");
    assert_eq!(v.cell_info(0, CellRef::new(1, 1)).display, "-£4.00");
    assert_eq!(crate::format_of(&v.doc, 0, CellRef::new(0, 0)).number_format, "\"€\"#,##0.00");
    assert_eq!(crate::format_of(&v.doc, 0, CellRef::new(0, 1)).number_format, "\"£\"#,##0.00");
    assert_eq!(crate::format_of(&v.doc, 0, CellRef::new(0, 2)).number_format, "\"$\"#,##0.00");

    v.set_input(0, CellRef::new(2, 0), "2/31/2026");
    v.set_input(0, CellRef::new(2, 1), "1900-02-29");
    v.set_input(0, CellRef::new(2, 2), "2024-02-29");
    v.set_input(0, CellRef::new(3, 0), "2026-02-29");
    v.set_input(0, CellRef::new(3, 1), "4/31/2026");
    assert_eq!(v.sheets()[0].value(CellRef::new(2, 0)), &Value::Text("2/31/2026".into()));
    assert_eq!(v.sheets()[0].value(CellRef::new(2, 1)), &Value::Text("1900-02-29".into()));
    assert_eq!(v.sheets()[0].value(CellRef::new(3, 0)), &Value::Text("2026-02-29".into()));
    assert_eq!(v.sheets()[0].value(CellRef::new(3, 1)), &Value::Text("4/31/2026".into()));
    assert!(matches!(v.sheets()[0].value(CellRef::new(2, 2)), Value::Number(_)));
    assert_eq!(v.cell_info(0, CellRef::new(2, 2)).display, "2024-02-29");
}

#[test]
fn fit_columns_truncates_on_character_boundaries() {
    let sample = crate::numfmt::format(&Value::Number(123.45), "0.00 \"€\"", crate::DateSystem::default()).text;
    assert_eq!(sample, "123.45 €");
    assert!(sample.len() > 9 && !sample.is_char_boundary(9), "byte 9 sits inside the euro sign");

    let mut v = SheetsView::blank(Kind::Sheets);
    let wide = CellRef::new(0, 0);
    let euro = CellRef::new(0, 1);
    v.sheets_mut()[0].set(wide, Value::Number(1_234_567.89));
    v.sheets_mut()[0].set(euro, Value::Number(123.45));
    for at in [wide, euro] {
        let mut f = crate::format_of(&v.doc, 0, at);
        f.number_format = "0.00 \"€\"".into();
        crate::set_format(&mut v.doc, 0, at, &f);
    }
    let mut host = FixedMeasurer::default();
    v.fit_columns(&mut host, 0);
    assert!(v.grid(0).width(0) > v.grid(0).default_width);
}

fn put_shared(v: &mut SheetsView, at: CellRef, value: Value, formula: Option<&str>, si: &str, range: Option<&str>) {
    let cell = v.sheets_mut()[0].cell_mut(at);
    cell.value = value;
    cell.formula = formula.map(str::to_string);
    let mut f = Element::new(ns::X, "f").with_attr("", "t", "shared").with_attr("", "si", si);
    if let Some(range) = range {
        f.set_attr("", "ref", range);
    }
    let f = match formula {
        Some(text) => f.with_text(text),
        None => f,
    };
    cell.origin = Origin::new(Element::new(ns::X, "c").with_child(f));
}

fn put_formula(v: &mut SheetsView, at: CellRef, formula: &str) {
    let cell = v.sheets_mut()[0].cell_mut(at);
    cell.formula = Some(formula.to_string());
    cell.value = Value::Number(0.0);
}

fn formula_text(v: &SheetsView, at: CellRef) -> Option<String> {
    v.sheets()[0].cell(at).and_then(|c| c.formula.clone())
}

fn sheet_xml(bytes: &[u8], part: &str) -> String {
    String::from_utf8(Package::read(bytes).unwrap().part(part).unwrap().to_vec()).unwrap()
}

#[test]
fn shift_rows_keeps_shared_formulas_and_moves_references() {
    let mut v = SheetsView::blank(Kind::Sheets);
    v.add_sheet();
    v.rename_sheet(1, "Other");
    v.sheets_mut()[1].set(CellRef::new(0, 0), Value::Number(3.0));
    v.sheets_mut()[0].set(CellRef::new(0, 1), Value::Number(10.0));
    v.sheets_mut()[0].set(CellRef::new(1, 1), Value::Number(20.0));
    v.sheets_mut()[0].set(CellRef::new(9, 0), Value::Number(5.0));
    v.sheets_mut()[0].set(CellRef::new(10, 0), Value::Number(6.0));
    v.sheets_mut()[0].set(CellRef::new(11, 0), Value::Number(7.0));
    v.sheets_mut()[0].set(CellRef::new(0, 4), Value::Number(100.0));
    v.sheets_mut()[0].set(CellRef::new(5, 25), Value::Number(1.0));
    v.sheets_mut()[0].set(CellRef::new(6, 25), Value::Number(2.0));
    v.sheets_mut()[0].set(CellRef::new(7, 25), Value::Number(4.0));
    // Above the insertion, so the group and its references stay shared.
    put_shared(&mut v, CellRef::new(0, 0), Value::Number(0.0), Some("B1+1"), "0", Some("A1:A2"));
    put_shared(&mut v, CellRef::new(1, 0), Value::Number(0.0), None, "0", None);
    // Below the insertion: every cell of the group has to become its own formula.
    put_shared(&mut v, CellRef::new(9, 2), Value::Number(0.0), Some("A10+1"), "1", Some("C10:C12"));
    put_shared(&mut v, CellRef::new(10, 2), Value::Number(0.0), None, "1", None);
    put_shared(&mut v, CellRef::new(11, 2), Value::Number(0.0), None, "1", None);
    put_formula(&mut v, CellRef::new(1, 6), "SUM(Z6:Z8)");
    put_formula(&mut v, CellRef::new(9, 3), "$E$1+A10");
    put_formula(&mut v, CellRef::new(9, 5), "Other!A1+A10");

    v.shift_rows(0, 5, 1);

    assert_eq!(v.cell_info(0, CellRef::new(0, 0)).input, "=B1+1");
    assert_eq!(v.cell_info(0, CellRef::new(1, 0)).input, "=B2+1");
    assert_eq!(v.cell_info(0, CellRef::new(0, 0)).display, "11");
    assert_eq!(v.cell_info(0, CellRef::new(1, 0)).display, "21");
    assert_eq!(formula_text(&v, CellRef::new(10, 2)).as_deref(), Some("A11+1"));
    assert_eq!(formula_text(&v, CellRef::new(11, 2)).as_deref(), Some("A12+1"));
    assert_eq!(formula_text(&v, CellRef::new(12, 2)).as_deref(), Some("A13+1"));
    assert_eq!(v.cell_info(0, CellRef::new(10, 2)).display, "6");
    assert_eq!(v.cell_info(0, CellRef::new(11, 2)).display, "7");
    assert_eq!(v.cell_info(0, CellRef::new(12, 2)).display, "8");
    assert_eq!(formula_text(&v, CellRef::new(1, 6)).as_deref(), Some("SUM(Z7:Z9)"));
    assert_eq!(v.cell_info(0, CellRef::new(1, 6)).display, "7");
    assert_eq!(formula_text(&v, CellRef::new(10, 3)).as_deref(), Some("$E$1+A11"));
    assert_eq!(v.cell_info(0, CellRef::new(10, 3)).display, "105");
    assert_eq!(formula_text(&v, CellRef::new(10, 5)).as_deref(), Some("Other!A1+A11"));
    assert_eq!(v.cell_info(0, CellRef::new(10, 5)).display, "8");

    let part = v.sheets()[0].part.clone().unwrap();
    let bytes = v.save().unwrap();
    let xml = sheet_xml(&bytes, &part);
    assert!(xml.contains(r#"t="shared""#), "{xml}");
    assert!(xml.contains("<f>A11+1</f>"), "{xml}");
    assert!(xml.contains("<f>A13+1</f>"), "{xml}");
    assert!(!xml.contains(r#"si="1""#), "the moved group is no longer shared\n{xml}");

    let mut back = SheetsView::open(&bytes, "Book.xlsx").unwrap();
    crate::recalculate(&mut back.doc, None);
    assert_eq!(back.cell_info(0, CellRef::new(1, 0)).input, "=B2+1");
    assert_eq!(back.cell_info(0, CellRef::new(1, 0)).display, "21");
    assert_eq!(formula_text(&back, CellRef::new(10, 2)).as_deref(), Some("A11+1"));
    assert_eq!(back.cell_info(0, CellRef::new(10, 2)).display, "6");
    assert_eq!(back.cell_info(0, CellRef::new(12, 2)).display, "8");
    assert_eq!(back.cell_info(0, CellRef::new(10, 5)).input, "=Other!A1+A11");
    assert_eq!(back.cell_info(0, CellRef::new(10, 5)).display, "8");
}

#[test]
fn deleting_a_shared_leader_keeps_follower_formulas() {
    let mut v = SheetsView::blank(Kind::Sheets);
    v.sheets_mut()[0].set(CellRef::new(9, 0), Value::Number(100.0));
    v.sheets_mut()[0].set(CellRef::new(10, 0), Value::Number(200.0));
    v.sheets_mut()[0].set(CellRef::new(11, 0), Value::Number(300.0));
    put_shared(&mut v, CellRef::new(0, 1), Value::Number(0.0), Some("A10*2"), "0", Some("B1:B3"));
    put_shared(&mut v, CellRef::new(1, 1), Value::Number(0.0), None, "0", None);
    put_shared(&mut v, CellRef::new(2, 1), Value::Number(0.0), None, "0", None);

    v.shift_rows(0, 0, -1);

    assert!(v.sheets()[0].cell(CellRef::new(0, 1)).unwrap().formula.is_some(), "the follower that lost its leader still has a formula");
    assert_eq!(formula_text(&v, CellRef::new(0, 1)).as_deref(), Some("A10*2"));
    assert_eq!(formula_text(&v, CellRef::new(1, 1)).as_deref(), Some("A11*2"));
    assert_eq!(v.cell_info(0, CellRef::new(0, 1)).display, "400");
    assert_eq!(v.cell_info(0, CellRef::new(1, 1)).display, "600");

    let part = v.sheets()[0].part.clone().unwrap();
    let bytes = v.save().unwrap();
    let xml = sheet_xml(&bytes, &part);
    assert!(xml.contains("<f>A10*2</f>"), "{xml}");
    assert!(xml.contains("<f>A11*2</f>"), "{xml}");
    assert!(!xml.contains(r#"t="shared""#), "{xml}");

    let mut back = SheetsView::open(&bytes, "Book.xlsx").unwrap();
    crate::recalculate(&mut back.doc, None);
    assert_eq!(formula_text(&back, CellRef::new(0, 1)).as_deref(), Some("A10*2"));
    assert_eq!(formula_text(&back, CellRef::new(1, 1)).as_deref(), Some("A11*2"));
    assert_eq!(back.cell_info(0, CellRef::new(0, 1)).display, "400");
    assert_eq!(back.cell_info(0, CellRef::new(1, 1)).display, "600");
}

#[test]
fn shift_cols_keeps_shared_formulas_and_moves_references() {
    let mut v = SheetsView::blank(Kind::Sheets);
    v.sheets_mut()[0].set(CellRef::new(20, 0), Value::Number(3.0));
    put_shared(&mut v, CellRef::new(20, 1), Value::Number(0.0), Some("A21*10"), "0", Some("B21:D21"));
    put_shared(&mut v, CellRef::new(20, 2), Value::Number(0.0), None, "0", None);
    put_shared(&mut v, CellRef::new(20, 3), Value::Number(0.0), None, "0", None);

    v.shift_cols(0, 1, 1);

    assert_eq!(formula_text(&v, CellRef::new(20, 2)).as_deref(), Some("A21*10"));
    assert_eq!(formula_text(&v, CellRef::new(20, 3)).as_deref(), Some("C21*10"));
    assert_eq!(formula_text(&v, CellRef::new(20, 4)).as_deref(), Some("D21*10"));
    assert_eq!(v.cell_info(0, CellRef::new(20, 2)).display, "30");
    assert_eq!(v.cell_info(0, CellRef::new(20, 3)).display, "300");
    assert_eq!(v.cell_info(0, CellRef::new(20, 4)).display, "3000");

    let bytes = v.save().unwrap();
    let mut back = SheetsView::open(&bytes, "Book.xlsx").unwrap();
    crate::recalculate(&mut back.doc, None);
    assert_eq!(formula_text(&back, CellRef::new(20, 2)).as_deref(), Some("A21*10"));
    assert_eq!(formula_text(&back, CellRef::new(20, 3)).as_deref(), Some("C21*10"));
    assert_eq!(formula_text(&back, CellRef::new(20, 4)).as_deref(), Some("D21*10"));
    assert_eq!(back.cell_info(0, CellRef::new(20, 4)).display, "3000");
}

#[test]
fn shared_follower_range_expands_when_the_leader_does_not() {
    // A1's range ends above the insertion; A2's copied range includes it. Keeping them shared
    // would leave A2 one row short.
    let mut v = SheetsView::blank(Kind::Sheets);
    for (row, n) in [1.0, 10.0, 100.0, 1_000.0, 10_000.0].into_iter().enumerate() {
        v.sheets_mut()[0].set(CellRef::new(row as u32, 25), Value::Number(n));
    }
    put_shared(&mut v, CellRef::new(0, 0), Value::Number(0.0), Some("SUM(Z1:Z4)"), "0", Some("A1:A2"));
    put_shared(&mut v, CellRef::new(1, 0), Value::Number(0.0), None, "0", None);

    v.shift_rows(0, 4, 1);

    assert_eq!(formula_text(&v, CellRef::new(0, 0)).as_deref(), Some("SUM(Z1:Z4)"));
    assert_eq!(formula_text(&v, CellRef::new(1, 0)).as_deref(), Some("SUM(Z2:Z6)"));
    assert_eq!(v.cell_info(0, CellRef::new(0, 0)).display, "1111");
    assert_eq!(v.cell_info(0, CellRef::new(1, 0)).display, "11110");

    let bytes = v.save().unwrap();
    let mut back = SheetsView::open(&bytes, "Book.xlsx").unwrap();
    crate::recalculate(&mut back.doc, None);
    assert_eq!(formula_text(&back, CellRef::new(1, 0)).as_deref(), Some("SUM(Z2:Z6)"));
    assert_eq!(back.cell_info(0, CellRef::new(1, 0)).display, "11110");
}

#[test]
fn sheet_size_is_kept_in_a_hidden_name_and_follows_the_sheet() {
    let mut v = SheetsView::blank(Kind::Sheets);
    v.set_input(0, CellRef::new(0, 0), "a");
    // Without a size, room past what's used.
    assert_eq!((v.grid(0).rows, v.grid(0).cols), (201, 52));
    v.set_sheet_size(0, 30, 8);
    assert_eq!((v.grid(0).rows, v.grid(0).cols), (30, 8));
    v.undo();
    assert_eq!(v.sheets()[0].size, None);
    v.redo();
    // Never fewer than what's there.
    v.set_input(0, CellRef::new(39, 9), "far");
    assert_eq!((v.grid(0).rows, v.grid(0).cols), (40, 10));
    // Inserting and deleting rows and columns carry the size along.
    v.shift_rows(0, 0, 2);
    v.shift_cols(0, 0, -1);
    assert_eq!(v.sheets()[0].size, Some((32, 7)));

    v.add_sheet();
    v.set_sheet_size(1, 5000, 40);
    v.rename_sheet(1, "Big one");
    let bytes = v.save().unwrap();
    let workbook = sheet_xml(&bytes, "xl/workbook.xml");
    assert!(workbook.contains(r#"<definedName name="Writan.SheetSize" localSheetId="1" hidden="1">{5000,40}</definedName>"#), "{workbook}");
    let mut back = SheetsView::open(&bytes, "Book.xlsx").unwrap();
    assert_eq!(back.sheets()[0].size, Some((32, 7)));
    assert_eq!((back.grid(1).rows, back.grid(1).cols), (5000, 40));

    // Deleting the first sheet moves the second's size to its new place; saving again keeps one name.
    back.delete_sheet(0);
    let again = back.save().unwrap();
    let workbook = sheet_xml(&again, "xl/workbook.xml");
    assert_eq!(workbook.matches("Writan.SheetSize").count(), 1, "{workbook}");
    assert!(workbook.contains(r#"localSheetId="0" hidden="1">{5000,40}"#), "{workbook}");
    let back = SheetsView::open(&again, "Book.xlsx").unwrap();
    assert_eq!(back.sheets()[0].size, Some((5000, 40)));
}

#[test]
fn a_row_numbered_past_excel_stops_at_its_last_row() {
    // A damaged sheet's row 2147483648 once asked for 16 GB of row positions.
    let bytes = SheetsView::blank(Kind::Sheets).save().unwrap();
    let mut pkg = Package::read(&bytes).unwrap();
    let xml = sheet_xml(&bytes, "xl/worksheets/sheet1.xml");
    let rows = r#"<sheetData><row r="2147483648"><c r="A1"><v>1</v></c></row><row><c r="ZZZ2"><v>2</v></c></row></sheetData>"#;
    let xml = xml.replace("<sheetData></sheetData>", rows);
    assert!(xml.contains("2147483648"), "{xml}");
    pkg.replace("xl/worksheets/sheet1.xml", xml.into_bytes()).unwrap();
    let mut v = SheetsView::open(&pkg.to_bytes().unwrap(), "Book.xlsx").unwrap();
    let g = v.grid(0);
    assert_eq!((g.rows, g.cols), (crate::MAX_ROWS, crate::MAX_COLS));
    assert!(g.content.0 <= g.rows && g.content.1 <= g.cols);
    let mut host = FixedMeasurer::default();
    v.paint(&mut host, 0, 0, 20, 0, 10);
}

#[test]
fn other_kinds_of_file_are_an_error_not_a_crash() {
    let word = office_core::Compiler::new().write(&Document::new(Kind::Word), Kind::Word).unwrap();
    let e = SheetsView::open(&word, "renamed.xlsx").err().expect("a Word file isn't a workbook");
    assert_eq!(e.to_string(), "this is a Word document, not an Excel workbook");
}

#[test]
fn a_circular_reference_is_reported_and_saves_zero() {
    let a1 = CellRef::new(0, 0);
    let mut v = SheetsView::blank(Kind::Sheets);
    assert!(v.circular().is_empty());
    v.set_input(0, a1, "=A1+1");
    assert_eq!(v.cell_info(0, a1).display, "0", "not what one step round the loop gives");
    assert_eq!(v.circular(), &[(0, a1)]);

    let part = v.sheets()[0].part.clone().unwrap();
    let bytes = v.save().unwrap();
    let xml = sheet_xml(&bytes, &part);
    assert!(xml.contains("<f>A1+1</f><v>0</v>"), "{xml}");

    // Reopened, it's still reported, before any edit.
    let mut back = SheetsView::open(&bytes, "Book.xlsx").unwrap();
    assert_eq!(back.circular(), &[(0, a1)]);
    assert_eq!(back.cell_info(0, a1).display, "0");

    back.set_input(0, a1, "=5");
    assert!(back.circular().is_empty());
    assert_eq!(back.cell_info(0, a1).display, "5");
    back.undo();
    assert_eq!(back.circular(), &[(0, a1)]);
}

#[test]
fn a_circular_reference_saved_with_a_result_opens_as_zero() {
    let a1 = CellRef::new(0, 0);
    let mut v = SheetsView::blank(Kind::Sheets);
    let b1 = CellRef::new(0, 1);
    v.set_input(0, a1, "=A1+1");
    v.set_input(0, b1, "=A1+1");
    // As 0.10.10 saved it: one step round the loop, and B1 with no result yet.
    if let Body::Workbook { sheets } = &mut v.doc.body {
        sheets[0].cell_mut(a1).value = Value::Number(1.0);
        sheets[0].cell_mut(b1).value = Value::Empty;
    }
    let bytes = v.save().unwrap();
    let mut back = SheetsView::open(&bytes, "Book.xlsx").unwrap();
    back.set_now(46000.0);
    assert_eq!(back.circular(), &[(0, a1)]);
    assert_eq!(back.cell_info(0, a1).display, "0");
    assert_eq!(back.cell_info(0, b1).display, "1", "worked out from the loop at 0");

    // Only circular without the clock: it keeps its result.
    let mut v = SheetsView::blank(Kind::Sheets);
    v.set_now(46000.0);
    v.set_input(0, a1, "=IFERROR(NOW(),A1)");
    let bytes = v.save().unwrap();
    let mut back = SheetsView::open(&bytes, "Book.xlsx").unwrap();
    back.set_now(46001.0);
    assert!(back.circular().is_empty());
    assert_ne!(back.cell_info(0, a1).display, "0");
}
