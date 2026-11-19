use office_core::model::CellRef;
use office_core::opc::Package;
use office_core::xml::{ns, Element, Node};
use office_sheets::view::SheetsView;

const FIXTURE: &[u8] = include_bytes!("v111-table-row-base.xlsx");
const TABLE: &str = "xl/tables/table1.xml";

fn table_column(table: &Element) -> &Element {
    table.child(ns::X, "tableColumns").unwrap().children_named(ns::X, "tableColumn").nth(1).unwrap()
}

#[test]
fn calculated_column_matches_worksheet_after_renaming_and_inserting_a_row() {
    let mut view = SheetsView::open(FIXTURE, "table.xlsx").unwrap();
    let reference = CellRef::parse("H2").unwrap();
    let check = |view: &mut SheetsView, expected: &str| {
        let saved = view.save().unwrap();
        let reopened = SheetsView::open(&saved, "table.xlsx").unwrap();
        assert_eq!(reopened.sheets()[1].cell(reference).unwrap().formula.as_deref(), Some(expected));
        let table = Package::read(&saved).unwrap().xml(TABLE).unwrap().root;
        assert_eq!(table.attr("ref"), Some("G1:H3"));
        assert_eq!(table_column(&table).child(ns::X, "calculatedColumnFormula").unwrap().text(), expected);
    };
    view.rename_sheet(1, "Bob's Data");
    check(&mut view, "'Bob''s Data'!$A$5");
    view.shift_rows(1, 4, 1);
    check(&mut view, "'Bob''s Data'!$A$6");
    assert!(view.undo());
    check(&mut view, "'Bob''s Data'!$A$5");
    assert!(view.redo());
    check(&mut view, "'Bob''s Data'!$A$6");
    view.shift_rows(1, 5, -1);
    check(&mut view, "#REF!");
}

#[test]
fn table_totals_follow_references_to_another_sheet_without_shifting_local_references() {
    let mut pkg = Package::read(FIXTURE).unwrap();
    let mut xml = pkg.xml(TABLE).unwrap();
    let column = xml.root.child_mut(ns::X, "tableColumns").unwrap().elements_mut().nth(1).unwrap();
    let original = "SUM(Summary!$B$2,$B$2,RefTable[Item],[1]Summary!$B$2)";
    column.push(Element::new(ns::X, "totalsRowFormula").with_text(original));
    column.child_mut(ns::X, "calculatedColumnFormula").unwrap().children = vec![Node::Text("$A$5".into())];
    pkg.replace(TABLE, xml.to_bytes()).unwrap();
    let mut view = SheetsView::open(&pkg.to_bytes().unwrap(), "table.xlsx").unwrap();
    let check = |view: &mut SheetsView, calculated, totals| {
        let saved = view.save().unwrap();
        let table = Package::read(&saved).unwrap().xml(TABLE).unwrap().root;
        assert_eq!(table.attr("ref"), Some("G1:H3"));
        let column = table_column(&table);
        assert_eq!(column.child(ns::X, "calculatedColumnFormula").unwrap().text(), calculated);
        assert_eq!(column.child(ns::X, "totalsRowFormula").unwrap().text(), totals);
    };
    view.shift_rows(0, 1, 1);
    check(&mut view, "$A$5", "SUM(Summary!$B$3,$B$2,RefTable[Item],[1]Summary!$B$2)");
    view.shift_cols(0, 1, 1);
    check(&mut view, "$A$5", "SUM(Summary!$C$3,$B$2,RefTable[Item],[1]Summary!$B$2)");
    view.shift_cols(0, 2, -1);
    check(&mut view, "$A$5", "SUM(#REF!,$B$2,RefTable[Item],[1]Summary!$B$2)");
    assert!(view.undo());
    check(&mut view, "$A$5", "SUM(Summary!$C$3,$B$2,RefTable[Item],[1]Summary!$B$2)");
    view.shift_rows(1, 4, 1);
    check(&mut view, "$A$6", "SUM(Summary!$C$3,$B$2,RefTable[Item],[1]Summary!$B$2)");
}
