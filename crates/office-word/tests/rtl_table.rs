//! Right-to-left (`w:bidiVisual`) Word tables: the first grid column is drawn at the right,
//! and the table's indent measures from the right edge (CORPUS-011).

use office_engine::display::Item;
use office_engine::measure::FixedMeasurer;
use office_word::view::WordView;

const SAMPLE: &[u8] = include_bytes!("../../../tests/stress/samples/rtl-table.docx");

/// Each painted text item's (text, x, width).
fn texts(view: &mut WordView, host: &mut FixedMeasurer) -> Vec<(String, f64, f64)> {
    view.layout(host);
    let mut out = Vec::new();
    for page in 0..view.page_count() {
        for item in view.paint(host, page).items {
            if let Item::Text { text, x, w, .. } = item {
                out.push((text, x, w));
            }
        }
    }
    out
}

fn at(items: &[(String, f64, f64)], text: &str) -> (f64, f64) {
    let (_, x, w) = items.iter().find(|i| i.0 == text).unwrap_or_else(|| panic!("{text} in {items:?}"));
    (*x, *w)
}

fn assert_column_order(view: &mut WordView, host: &mut FixedMeasurer) {
    let items = texts(view, host);
    let (page_w, _) = view.page_size(0);
    // bidiVisual: each row's second cell is drawn left of its first.
    assert!(at(&items, "second").0 < at(&items, "first").0, "{items:?}");
    assert!(at(&items, "beta").0 < at(&items, "alpha").0, "{items:?}");
    // The narrow first column (2000 twips ≈ 133 px) sits against the table's right edge: the
    // 720-twip (48 px) indent measures from the right, inside the 1-inch page margin.
    let (x, w) = at(&items, "first");
    // The ordinary table keeps its left-to-right order.
    assert!(at(&items, "west").0 < at(&items, "east").0, "{items:?}");
    // And its first column starts at the left margin, well left of the RTL table's start.
    assert!(at(&items, "west").0 < at(&items, "second").0, "{items:?}");
    assert!(x + w > page_w / 2.0, "{items:?} on {page_w}");
}

#[test]
fn rtl_table_columns_are_mirrored_and_survive_saving() {
    let mut host = FixedMeasurer::default();
    let mut view = WordView::open(SAMPLE).unwrap();
    for _ in 0..2 {
        assert_column_order(&mut view, &mut host);
        let saved = view.save().unwrap();
        let pkg = office_core::opc::Package::read(&saved).unwrap();
        let body = String::from_utf8(pkg.part("word/document.xml").unwrap().to_vec()).unwrap();
        assert!(body.contains("<w:bidiVisual/>"), "{body}");
        view = WordView::open(&saved).unwrap();
    }
}

#[test]
fn original_corpus_rtl_table_columns_are_mirrored() {
    let Some(dir) = std::env::var_os("CORPUS_FILES") else { return };
    let bytes = std::fs::read(std::path::Path::new(&dir).join("lo-table-rtl.docx")).unwrap();
    let mut host = FixedMeasurer::default();
    let mut view = WordView::open(&bytes).unwrap();
    let items = texts(&mut view, &mut host);
    assert!(at(&items, "B1").0 < at(&items, "A1").0, "{items:?}");
    assert!(at(&items, "B2").0 < at(&items, "A2").0, "{items:?}");
}
