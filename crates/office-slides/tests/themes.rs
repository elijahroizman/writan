use office_core::opc::{content_types::types, rels::kind, Package};
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

#[test]
fn saving_a_rethemed_deck_keeps_theme_parts_referenced_with_different_case() {
    let mut view = SlidesView::blank();
    view.apply_theme("Copper").unwrap();
    let pkg = view.doc.package.as_mut().unwrap();
    let main = pkg.main_part().unwrap();
    let master = &pkg.related(&main, kind::SLIDE_MASTER).unwrap()[0].1;
    let theme = &pkg.related(master, kind::THEME).unwrap()[0].1;
    for owner in [&main, master] {
        let mut rels = pkg.rels(owner).unwrap();
        for rel in rels.0.iter_mut().filter(|r| r.is(kind::THEME)) {
            rel.target.make_ascii_uppercase();
        }
        pkg.put_rels(owner, &rels);
    }
    let saved = view.save().unwrap();
    assert!(Package::read(&saved).unwrap().has(theme));
    assert_eq!(SlidesView::open(&saved).unwrap().doc.theme.unwrap().name, "Copper");
}
