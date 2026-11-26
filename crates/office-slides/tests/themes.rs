use office_core::opc::{content_types::types, Package};
use office_slides::view::SlidesView;

#[test]
fn saving_a_rethemed_deck_omits_unused_masters_layouts_and_themes() {
    let mut view = SlidesView::blank();
    for name in ["Harbor", "Default", "Graphite", "Copper"] {
        assert!(view.apply_theme(name).unwrap());
    }
    let saved = view.save().unwrap();
    let pkg = Package::read(&saved).unwrap();
    let count = |kind| pkg.part_names().filter(|part| pkg.content_type(part) == Some(kind)).count();
    assert_eq!(count(types::PPTX_MASTER), 1);
    assert_eq!(count(types::PPTX_LAYOUT), 9);
    assert_eq!(count(types::THEME), 1);
    let reopened = SlidesView::open(&saved).unwrap();
    assert_eq!(reopened.doc.theme.as_ref().unwrap().name, "Copper");
    assert_eq!(reopened.slides().len(), 2);
}
