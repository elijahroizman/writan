//! Charts: made from cells, saved as Excel chart parts, read back, moved, changed and deleted.

use office_core::model::{CellRef, ChartKind, LegendPosition, Range, Value};
use office_core::opc::Package;
use office_core::xml::{ns, Element, Node};
use office_core::Kind;
use office_sheets::view::{ChartPatch, SheetsView};

fn range(s: &str) -> Range {
    Range::parse(s).unwrap()
}

fn book() -> SheetsView {
    let mut v = SheetsView::blank(Kind::Sheets);
    let rows = [["Month", "Sales", "Costs"], ["Jan", "10", "4"], ["Feb", "12", "5"], ["Mar", "9", "7"]];
    for (r, row) in rows.iter().enumerate() {
        for (c, text) in row.iter().enumerate() {
            v.set_input(0, CellRef::new(r as u32, c as u32), text);
        }
    }
    v
}

fn part(bytes: &[u8], name: &str) -> Option<String> {
    let pkg = Package::read(bytes).unwrap();
    pkg.has(name).then(|| String::from_utf8(pkg.part(name).unwrap().to_vec()).unwrap())
}

#[test]
fn poi_monthly_budget_recalculates_totals_and_chart() {
    // Apache POI: test-data/spreadsheet/simple-monthly-budget.xlsx
    let mut v = SheetsView::open(include_bytes!("poi-simple-monthly-budget.xlsx"), "budget.xlsx").unwrap();
    let at = |s: &str| CellRef::parse(s).unwrap();
    assert_eq!(v.sheets()[0].value(at("E9")), &Value::Number(3750.0));
    let original_chart = v.charts(0);
    assert!(!original_chart.is_empty());
    v.set_input(0, at("C5"), "2600");
    let check = |v: &mut SheetsView| {
        let sheet = &v.sheets()[0];
        for (cell, expected) in [("E4", 2336.0), ("H4", 2336.0 / 3850.0), ("E9", 3850.0), ("F9", 2336.0), ("G9", 1514.0)] {
            assert_eq!(sheet.value(at(cell)), &Value::Number(expected), "{cell}");
        }
        assert_eq!(sheet.cell(at("E9")).unwrap().formula.as_deref(), Some("TotalMonthlyIncome"));
        let chart = v.charts(0);
        assert_eq!(chart[0].series[0].values, [Some(3850.0), Some(2336.0)]);
    };
    check(&mut v);
    let bytes = v.save().unwrap();
    let workbook = part(&bytes, "xl/workbook.xml").unwrap();
    assert!(workbook.contains("SUM(tblIncome[Amount])"));
    let original = include_bytes!("poi-simple-monthly-budget.xlsx");
    for name in ["xl/tables/table1.xml", "xl/tables/table2.xml"] {
        assert_eq!(part(&bytes, name), part(original, name));
    }
    check(&mut SheetsView::open(&bytes, "budget.xlsx").unwrap());
}

#[test]
fn poi_monthly_budget_tables_follow_inserted_and_deleted_rows() {
    let mut v = SheetsView::open(include_bytes!("poi-simple-monthly-budget.xlsx"), "budget.xlsx").unwrap();
    let at = |s: &str| CellRef::parse(s).unwrap();
    v.set_input(0, at("C5"), "2600");
    v.add_sheet();
    v.set_input(1, at("A1"), "=SUM(tblIncome[Amount])");
    v.shift_rows(0, at("C6").row, 1);
    v.set_input(0, at("C6"), "100");
    let check = |v: &SheetsView, summary: &str, amount| {
        assert_eq!(v.sheets()[0].value(at(summary)), &Value::Number(amount));
        assert_eq!(v.sheets()[1].value(at("A1")), &Value::Number(amount));
        assert_eq!(v.sheets()[1].cell(at("A1")).unwrap().formula.as_deref(), Some("SUM(tblIncome[Amount])"));
    };
    check(&v, "E10", 3950.0);
    let inserted = v.save().unwrap();
    assert!(part(&inserted, "xl/tables/table1.xml").unwrap().contains("ref=\"B4:C8\""));
    assert!(part(&inserted, "xl/tables/table2.xml").unwrap().contains("ref=\"B11:C24\""));
    v.shift_rows(0, at("C5").row, -1);
    check(&v, "E9", 1350.0);
    assert!(v.undo());
    check(&v, "E10", 3950.0);
    assert!(v.redo());
    check(&v, "E9", 1350.0);
    let deleted = v.save().unwrap();
    let pkg = Package::read(&deleted).unwrap();
    for (name, coords) in [("table1", "B4:C7"), ("table2", "B10:C23")] {
        let table = pkg.xml(&format!("xl/tables/{name}.xml")).unwrap().root;
        assert_eq!(table.attr("ref"), Some(coords));
        assert_eq!(table.child(ns::X, "autoFilter").unwrap().attr("ref"), Some(coords));
    }
    assert!(part(&deleted, "xl/workbook.xml").unwrap().contains("SUM(tblIncome[Amount])"));
    let mut reopened = SheetsView::open(&deleted, "budget.xlsx").unwrap();
    check(&reopened, "E9", 1350.0);
    reopened.set_input(0, at("C5"), "200");
    check(&reopened, "E9", 1450.0);
}

#[test]
fn poi_monthly_budget_structured_reference_selectors() {
    let mut pkg = Package::read(include_bytes!("poi-simple-monthly-budget.xlsx")).unwrap();
    let table = "xl/tables/table1.xml";
    let mut xml = pkg.xml(table).unwrap();
    xml.root.set_attr("", "totalsRowCount", "1");
    xml.root.child_mut(ns::X, "tableColumns").unwrap().elements_mut().next().unwrap().set_attr("", "name", "A5]#");
    pkg.replace(table, xml.to_bytes()).unwrap();
    let mut v = SheetsView::open(&pkg.to_bytes().unwrap(), "budget.xlsx").unwrap();
    let at = |s: &str| CellRef::parse(s).unwrap();
    let check = |v: &mut SheetsView, cell, formula, expected| {
        v.set_input(0, at(cell), formula);
        assert_eq!(v.sheets()[0].value(at(cell)), &expected, "{formula}");
    };
    for (cell, formula, expected) in [
        ("K4", "=SUM(TBLincome)", 3500.0),
        ("K5", "=SUM(tblIncome[[#All],[Amount]])", 3750.0),
        ("K6", "=SUM(tblIncome[[#Data],[A5']'#]:[Amount]])", 3500.0),
        ("K7", "=SUM(tblIncome[[#Totals],[Amount]])", 250.0),
        ("K8", "=SUM(tblIncome[[#Data],[#Totals],[Amount]])", 3750.0),
        ("K9", "=SUM(tblIncome[Amount] C6:C7)", 1000.0),
    ] {
        check(&mut v, cell, formula, Value::Number(expected));
    }
    check(&mut v, "K10", "=tblIncome[@[Amount]]", Value::Error("#VALUE!".into()));
    check(&mut v, "K11", "=tblIncome[[#Headers],[Amount]]", Value::Text("Amount".into()));
    check(&mut v, "B5", "=[@Amount]", Value::Number(2500.0));
    check(&mut v, "B6", "=tblIncome[[#This Row],[Amount]]", Value::Number(1000.0));
    v.shift_rows(0, at("C5").row, 1);
    assert_eq!(v.sheets()[0].cell(at("K7")).unwrap().formula.as_deref(), Some("SUM(tblIncome[[#Data],[A5']'#]:[Amount]])"));
    assert_eq!(v.sheets()[0].value(at("B6")), &Value::Number(2500.0));
}

#[test]
fn a_chart_from_a_block_of_cells() {
    let mut v = book();
    // From one cell, the whole block around it.
    assert_eq!(v.add_chart(0, range("B2"), ChartKind::Column), Some(0));
    let c = &v.sheets()[0].charts[0];
    assert_eq!(c.series.len(), 2);
    assert_eq!(c.series[0].name_ref.as_deref(), Some("Sheet1!$B$1"));
    assert_eq!(c.series[0].categories.as_deref(), Some("Sheet1!$A$2:$A$4"));
    assert_eq!(c.series[1].values, "Sheet1!$C$2:$C$4");
    assert_eq!(c.legend, Some(LegendPosition::Bottom));
    let shown = v.charts(0);
    assert_eq!(shown[0].categories, ["Jan", "Feb", "Mar"]);
    assert_eq!(shown[0].series[0].name, "Sales");
    assert_eq!(shown[0].series[1].values, [Some(4.0), Some(5.0), Some(7.0)]);
    assert_eq!(shown[0].range.as_deref(), Some("A1:C4"));
    assert!(shown[0].x > 200.0 && shown[0].w > 400.0, "{:?}", (shown[0].x, shown[0].w));

    let bytes = v.save().unwrap();
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml").unwrap();
    assert!(sheet.contains("<drawing r:id="), "{sheet}");
    let chart = part(&bytes, "xl/charts/chart1.xml").unwrap();
    assert!(chart.contains("<c:barDir val=\"col\"/>") && chart.contains("<c:f>Sheet1!$B$2:$B$4</c:f>"), "{chart}");
    assert!(part(&bytes, "xl/drawings/drawing1.xml").unwrap().contains("r:id=\"rId1\""));
    let pkg = Package::read(&bytes).unwrap();
    assert_eq!(pkg.content_type("xl/charts/chart1.xml"), Some("application/vnd.openxmlformats-officedocument.drawingml.chart+xml"));
    assert_eq!(pkg.content_type("xl/drawings/drawing1.xml"), Some("application/vnd.openxmlformats-officedocument.drawing+xml"));

    // Read back: the same chart, which saves again without changes.
    let mut back = SheetsView::open(&bytes, "book.xlsx").unwrap();
    let c2 = back.sheets()[0].charts[0].clone();
    let refs = |c: &office_core::model::Chart| {
        c.series.iter().map(|s| (s.name_ref.clone(), s.categories.clone(), s.values.clone())).collect::<Vec<_>>()
    };
    assert_eq!(refs(&c2), refs(&v.sheets()[0].charts[0]));
    assert_eq!(c2.series[1].cache.values, [Some(4.0), Some(5.0), Some(7.0)]);
    assert_eq!(c2.anchor, v.sheets()[0].charts[0].anchor);
    assert!(c2.untouched());
    let again = back.save().unwrap();
    assert_eq!(part(&again, "xl/charts/chart1.xml"), Some(chart.clone()));

    // Moved: the chart part stays, the anchor moves.
    back.move_chart(0, 0, 10.0, 200.0, 300.0, 200.0);
    let moved = back.save().unwrap();
    assert_eq!(part(&moved, "xl/charts/chart1.xml"), Some(chart));
    let reread = SheetsView::open(&moved, "book.xlsx").unwrap();
    assert_eq!(reread.sheets()[0].charts[0].anchor.from.col, 0);

    // Changed: written fresh.
    back.set_chart(0, 0, &ChartPatch { kind: Some(ChartKind::Line), title: Some("Money".into()), ..Default::default() });
    let changed = back.save().unwrap();
    let xml = part(&changed, "xl/charts/chart1.xml").unwrap();
    assert!(xml.contains("<c:lineChart>") && xml.contains("Money"), "{xml}");
    let reread = SheetsView::open(&changed, "book.xlsx").unwrap();
    assert_eq!(reread.sheets()[0].charts[0].kind, ChartKind::Line);
    assert_eq!(reread.sheets()[0].charts[0].title.as_deref(), Some("Money"));

    // Deleted: the drawing goes, and the sheet stops pointing at it.
    back.delete_chart(0, 0);
    let gone = back.save().unwrap();
    assert!(part(&gone, "xl/charts/chart1.xml").is_none());
    assert!(part(&gone, "xl/drawings/drawing1.xml").is_none());
    assert!(!part(&gone, "xl/worksheets/sheet1.xml").unwrap().contains("<drawing"));
}

#[test]
fn axis_titles_read_and_write() {
    let mut v = book();
    v.add_chart(0, range("A1:C4"), ChartKind::Column).unwrap();
    v.set_chart(0, 0, &ChartPatch { cat_title: Some("Month".into()), val_title: Some("Sales ($)".into()), ..Default::default() });
    let shown = v.charts(0);
    assert_eq!(shown[0].cat_title.as_deref(), Some("Month"));
    assert_eq!(shown[0].val_title.as_deref(), Some("Sales ($)"));

    // Written: each axis carries its title, the vertical one turned.
    let bytes = v.save().unwrap();
    let xml = part(&bytes, "xl/charts/chart1.xml").unwrap();
    assert!(xml.contains("<c:catAx>") && xml.contains(">Month<") && xml.contains(">Sales ($)<"), "{xml}");
    let at = |s: &str| xml.find(s).unwrap_or(usize::MAX);
    assert!(at("<c:catAx>") < at(">Month<") && at(">Month<") < at("<c:valAx>") && at("<c:valAx>") < at(">Sales ($)<"), "{xml}");
    assert_eq!(xml.matches("rot=\"-5400000\"").count(), 1, "{xml}");
    assert!(at("<c:valAx>") < at("rot=\"-5400000\"") && at("rot=\"-5400000\"") < at(">Sales ($)<"), "{xml}");

    // Read back: the same titles, and an untouched save keeps the part byte for byte.
    let mut back = SheetsView::open(&bytes, "book.xlsx").unwrap();
    let c = &back.sheets()[0].charts[0];
    assert_eq!(c.cat_title.as_deref(), Some("Month"));
    assert_eq!(c.val_title.as_deref(), Some("Sales ($)"));
    assert!(c.untouched());
    let again = back.save().unwrap();
    assert_eq!(part(&again, "xl/charts/chart1.xml"), Some(xml.clone()));

    // A group that lists its value axis first still reads each title by its axis' kind.
    let mut pkg = Package::read(&bytes).unwrap();
    let chart = "xl/charts/chart1.xml";
    let xml = String::from_utf8(pkg.part(chart).unwrap().to_vec()).unwrap();
    let swapped =
        xml.replacen("<c:axId val=\"500000001\"/><c:axId val=\"500000002\"/>", "<c:axId val=\"500000002\"/><c:axId val=\"500000001\"/>", 1);
    assert_ne!(swapped, xml);
    pkg.replace(chart, swapped.into_bytes()).unwrap();
    let mut back = SheetsView::open(&pkg.to_bytes().unwrap(), "book.xlsx").unwrap();
    let c = &back.sheets()[0].charts[0];
    assert_eq!(c.cat_title.as_deref(), Some("Month"));
    assert_eq!(c.val_title.as_deref(), Some("Sales ($)"));

    // A bar chart's axes swap sides: the value title lies along the bottom.
    back.set_chart(0, 0, &ChartPatch { kind: Some(ChartKind::Bar), ..Default::default() });
    let xml = part(&back.save().unwrap(), "xl/charts/chart1.xml").unwrap();
    assert!(xml.contains(">Month<") && xml.contains(">Sales ($)<"), "{xml}");
    let at = |s: &str| xml.find(s).unwrap_or(usize::MAX);
    assert!(at("<c:catAx>") < at("rot=\"-5400000\"") && at("rot=\"-5400000\"") < at(">Month<"), "{xml}");
    assert!(at("<c:valAx>") < at(">Sales ($)<"), "{xml}");

    // A scatter chart's x axis is a valAx too; its title still reads back as the category one.
    back.set_chart(0, 0, &ChartPatch { kind: Some(ChartKind::Scatter), ..Default::default() });
    let bytes = back.save().unwrap();
    let xml = part(&bytes, "xl/charts/chart1.xml").unwrap();
    assert!(!xml.contains("<c:catAx>"), "{xml}");
    let back = SheetsView::open(&bytes, "book.xlsx").unwrap();
    let c = &back.sheets()[0].charts[0];
    assert_eq!(c.cat_title.as_deref(), Some("Month"), "{xml}");
    assert_eq!(c.val_title.as_deref(), Some("Sales ($)"), "{xml}");

    // Empty clears a title.
    let mut back = SheetsView::open(&bytes, "book.xlsx").unwrap();
    back.set_chart(0, 0, &ChartPatch { cat_title: Some(String::new()), ..Default::default() });
    assert_eq!(back.charts(0)[0].cat_title, None);
    let xml = part(&back.save().unwrap(), "xl/charts/chart1.xml").unwrap();
    assert!(!xml.contains(">Month<"), "{xml}");
}

#[test]
fn charts_follow_rows_names_and_undo() {
    let mut v = book();
    v.add_chart(0, range("A1:C4"), ChartKind::Pie).unwrap();
    v.shift_rows(0, 0, 2);
    assert_eq!(v.sheets()[0].charts[0].series[0].values, "Sheet1!$B$4:$B$6");
    assert_eq!(v.sheets()[0].charts[0].anchor.from.row, 2);
    v.rename_sheet(0, "Q 1");
    assert_eq!(v.sheets()[0].charts[0].series[0].values, "'Q 1'!$B$4:$B$6");
    assert_eq!(v.charts(0)[0].series[0].values, [Some(10.0), Some(12.0), Some(9.0)]);
    v.undo();
    v.undo();
    v.undo();
    assert!(v.sheets()[0].charts.is_empty());
    v.redo();
    assert_eq!(v.sheets()[0].charts.len(), 1);
}

#[test]
fn scatter_plots_x_against_y() {
    let mut v = SheetsView::blank(Kind::Sheets);
    for (r, (x, y)) in [(1, 2), (2, 4), (3, 9)].iter().enumerate() {
        v.set_input(0, CellRef::new(r as u32, 0), &x.to_string());
        v.set_input(0, CellRef::new(r as u32, 1), &y.to_string());
    }
    v.add_chart(0, range("A1:B3"), ChartKind::Scatter).unwrap();
    let c = &v.charts(0)[0];
    assert_eq!(c.series.len(), 1);
    assert_eq!(c.series[0].x, Some(vec![Some(1.0), Some(2.0), Some(3.0)]));
    let bytes = v.save().unwrap();
    let xml = part(&bytes, "xl/charts/chart1.xml").unwrap();
    assert!(xml.contains("<c:xVal><c:numRef><c:f>Sheet1!$A$1:$A$3</c:f>"), "{xml}");
}

/// A workbook from another app with charts (set CHART_SAMPLES to a folder of .xlsx files).
#[test]
fn other_apps_charts_survive() {
    let Ok(dir) = std::env::var("CHART_SAMPLES") else { return };
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "xlsx") {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let mut v = SheetsView::open(&bytes, "x.xlsx").unwrap();
        let count: usize = v.sheets().iter().map(|s| s.charts.len()).sum();
        eprintln!("{}: {count} charts", path.display());
        for (i, s) in v.sheets().to_vec().iter().enumerate() {
            for c in v.charts(i) {
                eprintln!(
                    "  {} {:?} {:?} {} series, {} categories, {:?}",
                    s.name,
                    c.kind,
                    c.title,
                    c.series.len(),
                    c.categories.len(),
                    c.range
                );
            }
        }
        let pkg = Package::read(&bytes).unwrap();
        let charts: Vec<String> = pkg.part_names().filter(|n| n.starts_with("xl/charts/chart")).map(str::to_string).collect();
        let saved = v.save().unwrap();
        for c in &charts {
            assert_eq!(part(&saved, c), part(&bytes, c), "{c} changed on an unchanged save");
        }
        // Move every chart: parts stay, the file reads back with as many charts.
        for i in 0..v.sheets().len() {
            for (k, c) in v.charts(i).iter().enumerate() {
                v.move_chart(i, k, c.x + 20.0, c.y + 20.0, c.w, c.h);
            }
        }
        let saved = v.save().unwrap();
        let back = SheetsView::open(&saved, "x.xlsx").unwrap();
        assert_eq!(back.sheets().iter().map(|s| s.charts.len()).sum::<usize>(), count);
        // Change every chart's title: written fresh, still read back.
        for i in 0..v.sheets().len() {
            for k in 0..v.sheets()[i].charts.len() {
                v.set_chart(i, k, &ChartPatch { title: Some(format!("T{k}")), ..Default::default() });
            }
        }
        let saved = v.save().unwrap();
        let back = SheetsView::open(&saved, "x.xlsx").unwrap();
        assert_eq!(back.sheets().iter().map(|s| s.charts.len()).sum::<usize>(), count);
        std::fs::write(format!("/tmp/chart-samples/out-{}", path.file_name().unwrap().to_string_lossy()), &saved).ok();
    }
}

/// Writes a workbook with one chart of every kind into CHART_OUT, for checking in other apps.
#[test]
fn every_kind_for_other_apps() {
    let Ok(dir) = std::env::var("CHART_OUT") else { return };
    let mut v = book();
    let kinds =
        [ChartKind::Column, ChartKind::Bar, ChartKind::Line, ChartKind::Area, ChartKind::Pie, ChartKind::Doughnut, ChartKind::Scatter];
    for (i, k) in kinds.iter().enumerate() {
        let index = v.add_chart(0, range("A1:C4"), *k).unwrap();
        v.move_chart(0, index, 300.0 + (i % 2) as f64 * 500.0, (i / 2) as f64 * 320.0, 480.0, 300.0);
    }
    v.set_chart(
        0,
        0,
        &ChartPatch {
            grouping: Some(office_core::model::ChartGrouping::Stacked),
            title: Some("Stacked & <titled>".into()),
            cat_title: Some("Month".into()),
            val_title: Some("Amount".into()),
            color: Some((1, "AA3355".into())),
            ..Default::default()
        },
    );
    std::fs::write(format!("{dir}/every-kind.xlsx"), v.save().unwrap()).unwrap();
}

/// Adding the first chart to a saved workbook rewrites that sheet's XML; the other sheets' text
/// must still point at the right shared strings.
#[test]
fn a_new_chart_keeps_other_sheets_text() {
    let mut v = book();
    v.add_sheet();
    v.set_input(1, CellRef::new(0, 0), "Only on the second sheet");
    let mut v = SheetsView::open(&v.save().unwrap(), "a.xlsx").unwrap();
    v.add_chart(0, range("A1:C4"), ChartKind::Column).unwrap();
    let back = SheetsView::open(&v.save().unwrap(), "a.xlsx").unwrap();
    assert_eq!(back.cell_info(1, CellRef::new(0, 0)).display, "Only on the second sheet");
    assert_eq!(back.cell_info(0, CellRef::new(1, 0)).display, "Jan");
    assert_eq!(back.sheets()[0].charts.len(), 1);
}

/// A chart on one sheet drawing another sheet's cells follows rows inserted there, and undo puts
/// it back.
#[test]
fn charts_on_other_sheets_follow_inserted_rows() {
    let mut v = book();
    v.add_sheet();
    v.set_input(1, CellRef::new(0, 0), "1");
    v.add_chart(1, range("A1:A2"), ChartKind::Column).unwrap();
    v.set_chart(1, 0, &ChartPatch { range: Some("Sheet1!A1:C4".into()), ..Default::default() });
    let before = v.sheets()[1].charts[0].series[0].values.clone();
    assert_eq!(before, "Sheet1!$B$2:$B$4");
    v.shift_rows(0, 0, 1);
    assert_eq!(v.sheets()[1].charts[0].series[0].values, "Sheet1!$B$3:$B$5");
    assert_eq!(v.charts(1)[0].series[0].values, [Some(10.0), Some(12.0), Some(9.0)]);
    v.undo();
    assert_eq!(v.sheets()[1].charts[0].series[0].values, before);
}

/// Other apps anchor charts to one cell with a size; rows inserted above move it, in the file too.
#[test]
fn one_cell_anchors_move_with_rows() {
    let mut v = book();
    v.add_chart(0, range("A1:C4"), ChartKind::Column).unwrap();
    let mut pkg = Package::read(&v.save().unwrap()).unwrap();
    let mut doc = pkg.xml("xl/drawings/drawing1.xml").unwrap();
    let a = doc.root.children.iter_mut().find_map(|n| if let Node::Element(e) = n { Some(e) } else { None }).unwrap();
    a.local = "oneCellAnchor".into();
    a.remove_attr("", "editAs");
    a.remove_children(ns::XDR, "to");
    a.children.insert(1, Node::Element(Element::new(ns::XDR, "ext").with_attr("", "cx", "4572000").with_attr("", "cy", "2743200")));
    pkg.put_xml("xl/drawings/drawing1.xml", &doc, "application/vnd.openxmlformats-officedocument.drawing+xml");
    let mut v = SheetsView::open(&pkg.to_bytes().unwrap(), "a.xlsx").unwrap();
    let row = v.sheets()[0].charts[0].anchor.from.row;
    v.shift_rows(0, 0, 3);
    let back = SheetsView::open(&v.save().unwrap(), "a.xlsx").unwrap();
    assert_eq!(back.sheets()[0].charts[0].anchor.from.row, row + 3);
    assert_eq!(back.sheets()[0].charts[0].anchor.size, Some((4572000, 2743200)));
}
