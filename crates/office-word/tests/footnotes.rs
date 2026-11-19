use office_engine::display::Item;
use office_engine::measure::FixedMeasurer;
use office_word::view::WordView;

fn painted_text(view: &mut WordView) -> Vec<String> {
    let mut host = FixedMeasurer::default();
    view.layout(&mut host);
    let mut out = Vec::new();
    for page in 0..view.page_count() {
        for item in view.paint(&mut host, page).items {
            if let Item::Text { text, .. } = item {
                out.push(text.trim().to_string());
            }
        }
    }
    out
}

#[test]
fn original_corpus_footnote_body_is_painted_and_survives_saving() {
    let Some(dir) = std::env::var_os("CORPUS_FILES") else { return };
    let bytes = std::fs::read(std::path::Path::new(&dir).join("lo-footnote.docx")).unwrap();
    let mut view = WordView::open(&bytes).unwrap();
    for _ in 0..2 {
        assert_eq!(view.page_count().max(1), 1);
        let text = painted_text(&mut view);
        assert!(text.iter().any(|t| t == "Foo"), "{text:?}");
        assert!(text.iter().any(|t| t == "bar"), "the note body must appear in the display list: {text:?}");
        assert_eq!(text.iter().filter(|t| *t == "1").count(), 2, "the reference and the note's own mark: {text:?}");
        view = WordView::open(&view.save().unwrap()).unwrap();
    }
}
