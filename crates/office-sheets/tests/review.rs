use office_core::model::{Body, Origin, Value};
use office_core::xml::{self, ns};
use office_core::{Document, Kind};
use office_sheets::{
    eval::{self, Values, V},
    formula, numfmt, DateSystem,
};
#[test]
fn excel_single_cell_sum_coercion() {
    let mut b = Values::default();
    b.cells.insert((0, 0), V::Text("3".into()));
    b.cells.insert((0, 1), V::Bool(true));
    assert_eq!(eval::eval(&formula::parse("SUM(A1,B1)").unwrap(), &mut b), V::Num(0.));
    b.names.insert("TextRef".into(), formula::parse("A1").unwrap());
    for (f, expected) in
        [("SUM(TextRef)", 0.), ("SUM(\"3\",TRUE)", 4.), ("COUNT(A1,B1)", 0.), ("COUNT(\"3\",TRUE)", 2.), ("SUM(A1+0,B1+0)", 4.)]
    {
        assert_eq!(eval::eval(&formula::parse(f).unwrap(), &mut b), V::Num(expected), "{f}");
    }
}

#[test]
fn excel_isnumber_error() {
    for (f, expected) in [
        ("ISNUMBER(1/0)", false),
        ("ISBLANK(1/0)", false),
        ("ISTEXT(1/0)", false),
        ("ISLOGICAL(1/0)", false),
        ("ISNONTEXT(1/0)", true),
        ("ISERROR(1/0)", true),
        ("ISERR(1/0)", true),
        ("ISNA(1/0)", false),
        ("ISNA(NA())", true),
        ("ISERR(NA())", false),
    ] {
        assert_eq!(eval::eval(&formula::parse(f).unwrap(), &mut Values::default()), V::Bool(expected), "{f}");
    }
}

#[test]
fn excel_sumproduct_error() {
    assert_eq!(eval::eval(&formula::parse("SUMPRODUCT({1,#N/A},{2,3})").unwrap(), &mut Values::default()), V::Err("#N/A"));
}

#[test]
fn excel_jagged_array_no_panic() {
    assert!(formula::parse("SUMPRODUCT({1,2;3},{1,2;3})").is_err());
}

#[test]
fn excel_sheet_name_looks_like_cell() {
    assert!(formula::parse("Q1!A1").is_ok());
}

#[test]
fn excel_sheet_ref_shift() {
    assert_eq!(formula::shift("Q1!A1", 1, 0).unwrap(), "Q1!A2");
}

#[test]
fn excel_unicode_comparison_no_panic() {
    assert!(std::panic::catch_unwind(|| formula::parse("1<é")).is_ok());
}

#[test]
fn excel_rounding_numfmt() {
    for (n, code, expected) in
        [(2.5, "0", "3"), (-2.5, "0", "-3"), (1.005, "0.00", "1.01"), (-1.005, "0.00", "-1.01"), (9.995, "0.00", "10.00")]
    {
        assert_eq!(numfmt::format(&Value::Number(n), code, DateSystem::From1900).text, expected);
    }
}

#[test]
fn excel_column_width_retains_metadata() {
    let mut d = Document::new(Kind::Sheets);
    if let Body::Workbook { sheets } = &mut d.body {
        sheets[0].origin=Origin::new(xml::parse_element(&format!(r#"<worksheet xmlns="{}"><cols><col min="1" max="3" width="8" hidden="1" style="2" outlineLevel="1"/></cols><sheetData/></worksheet>"#,ns::X)).unwrap());
    }
    office_sheets::set_column_width(&mut d, 0, 1, Some(20.));
    let Body::Workbook { sheets } = &d.body else { unreachable!() };
    let col = sheets[0]
        .origin
        .get()
        .unwrap()
        .child(ns::X, "cols")
        .unwrap()
        .children_named(ns::X, "col")
        .find(|x| x.attr("min") == Some("2"))
        .unwrap();
    assert_eq!((col.attr("hidden"), col.attr("style"), col.attr("outlineLevel")), (Some("1"), Some("2"), Some("1")));
}

#[test]
fn excel_quoted_sheet_shared_shift() {
    let shifted = formula::shift("'Q1'!A1", 1, 0).unwrap();
    assert_eq!(shifted, "'Q1'!A2");
    assert!(formula::parse(&shifted).is_ok(), "{shifted}");
}

#[test]
fn excel_general_negative_section() {
    let got = numfmt::format(&Value::Number(-5.), "General;[Red](0)", DateSystem::From1900);
    assert_eq!(got, numfmt::Formatted { text: "(5)".into(), color: Some("red".into()) });
}

#[test]
fn excel_large_date_no_panic() {
    for f in ["DATE(1E100,1,1)", "DATE(2020,1E100,1)", "DATE(2020,1,-1E100)", "DATE(10000,1,1)", "DATE(-1,1,1)"] {
        assert_eq!(eval::eval(&formula::parse(f).unwrap(), &mut Values::default()), V::Err("#NUM!"));
    }
}

#[test]
fn excel_fixed_denominator_ten() {
    assert_eq!(numfmt::format(&Value::Number(0.3), "# ?/10", DateSystem::From1900).text, "3/10");
}

#[test]
fn excel_intersection_operator() {
    let mut b = Values::default();
    for r in 0..3 {
        for c in 0..3 {
            b.cells.insert((r, c), V::Num((1 + r * 3 + c) as f64));
        }
    }
    assert_eq!(eval::eval(&formula::parse("SUM(A1:B2 B2:C3)").unwrap(), &mut b), V::Num(5.));
    assert_eq!(eval::eval(&formula::parse("SUM(A1:B2 C3)").unwrap(), &mut b), V::Err("#NULL!"));
    assert_eq!(eval::eval(&formula::parse("SUM(A1:B2 B2:C3,A1)").unwrap(), &mut b), V::Num(6.));
}

#[test]
fn excel_union_operator() {
    let mut b = Values::default();
    for r in 0..2 {
        for c in 0..3 {
            b.cells.insert((r, c), V::Num((1 + r * 3 + c) as f64));
        }
    }
    assert_eq!(eval::eval(&formula::parse("SUM((A1:A2,C1:C2))").unwrap(), &mut b), V::Num(14.));
    assert_eq!(eval::eval(&formula::parse("SUM((A1:A2,C1:C2) A2:C2)").unwrap(), &mut b), V::Num(10.));
    assert_eq!(eval::eval(&formula::parse("SUM((A1:A2,C1:C2),2)").unwrap(), &mut b), V::Num(16.));
}

#[test]
fn excel_sum_overflow_is_error() {
    for f in ["SUM(9E307,9E307)", "PRODUCT(9E307,2)", "AVERAGE(9E307,9E307)", "1E999", "VALUE(\"inf\")"] {
        assert_eq!(eval::eval(&formula::parse(f).unwrap(), &mut Values::default()), V::Err("#NUM!"));
    }
    for n in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
        assert_eq!(numfmt::general(n), "#NUM!");
        assert_eq!(numfmt::format(&Value::Number(n), "0.00", DateSystem::From1900).text, "#NUM!");
    }
}

#[test]
fn excel_defined_name_cycle() {
    let mut b = Values::default();
    b.names.insert("Loop".into(), formula::parse("Loop").unwrap());
    assert_eq!(eval::eval(&formula::parse("Loop").unwrap(), &mut b), V::Err("#REF!"));
    b.names.insert("Loop".into(), formula::parse("Other").unwrap());
    b.names.insert("Other".into(), formula::parse("loop").unwrap());
    assert_eq!(eval::eval(&formula::parse("Loop").unwrap(), &mut b), V::Err("#REF!"));
    b.names.clear();
    for i in 0..1000 {
        b.names.insert(format!("Name_{i}"), formula::parse(&format!("Name_{}", i + 1)).unwrap());
    }
    assert_eq!(eval::eval(&formula::parse("Name_0").unwrap(), &mut b), V::Err("#REF!"));
    assert_eq!(eval::eval(&formula::parse("1+2").unwrap(), &mut b), V::Num(3.));
}
#[test]
fn excel_unknown_unicode_rejected() {
    assert!(formula::parse("💥").is_err());
}
#[test]
fn excel_large_fraction_denominator() {
    assert_eq!(numfmt::format(&Value::Number(0.3), "# ?/????????????", DateSystem::From1900).text, "3/10");
}
#[test]
fn excel_wildcard_adversarial() {
    let f = format!("SEARCH(\"{}b\",\"{}\")", "*a".repeat(28), "a".repeat(56));
    assert_eq!(eval::eval(&formula::parse(&f).unwrap(), &mut Values::default()), V::Err("#VALUE!"));
}

#[test]
fn excel_text_format_shows_numbers() {
    // A number under the Text format (`@`) shows its digits, in the file and after a round trip.
    use office_core::model::{Body, CellRef};
    let compiler = office_core::Compiler::new().with_layer(office_sheets::Sheets);
    let mut doc = Document::new(Kind::Sheets);
    let Body::Workbook { sheets } = &mut doc.body else { unreachable!() };
    sheets[0].set(CellRef::new(0, 0), Value::Number(42.5));
    let mut format = office_sheets::format_of(&doc, 0, CellRef::new(0, 0));
    format.number_format = "@".into();
    office_sheets::set_format(&mut doc, 0, CellRef::new(0, 0), &format);
    assert_eq!(office_sheets::display(&doc, 0, CellRef::new(0, 0)), "42.5");
    let saved = compiler.write(&doc, Kind::Sheets).unwrap();
    let back = compiler.read(&saved, Some("book.xlsx")).unwrap();
    assert_eq!(office_sheets::display(&back, 0, CellRef::new(0, 0)), "42.5");
}
