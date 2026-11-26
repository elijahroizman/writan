use super::content_types::types;
use super::rels::kind;
use super::*;
use crate::xml::{ns, Element};

/// A small Word package, built the way the back ends will build one.
fn sample() -> Package {
    let mut pkg = Package::new();
    let body = Element::new(ns::W, "body").with_child(Element::new(ns::W, "p"));
    pkg.put_xml("word/document.xml", &XmlDoc::new(Element::new(ns::W, "document").with_child(body)), types::DOCX_MAIN);
    pkg.relate("", kind::OFFICE_DOCUMENT, "word/document.xml").unwrap();
    pkg.put("word/media/image1.png", vec![0x89, b'P', b'N', b'G', 1, 2, 3], "image/png");
    pkg.relate("word/document.xml", kind::IMAGE, "word/media/image1.png").unwrap();
    pkg
}

#[test]
fn builds_writes_and_reads_back() {
    let bytes = sample().to_bytes().unwrap();
    let pkg = Package::read(&bytes).unwrap();
    assert_eq!(pkg.main_part().unwrap(), "word/document.xml");
    assert_eq!(pkg.content_type("word/document.xml"), Some(types::DOCX_MAIN));
    assert_eq!(pkg.content_type("word/media/image1.png"), Some("image/png"));
    assert!(pkg.content_types().defaults.iter().any(|(e, _)| e == "png"), "pictures get a default by extension");
    let images = pkg.related("word/document.xml", kind::IMAGE).unwrap();
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].1, "word/media/image1.png");
    assert_eq!(images[0].0.target, "media/image1.png");
    assert_eq!(pkg.part("word/media/image1.png").unwrap(), &[0x89, b'P', b'N', b'G', 1, 2, 3]);
    assert!(pkg.xml("word/document.xml").unwrap().root.descendant(ns::W, "p").is_some());

    // [Content_Types].xml is the first entry.
    let archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    assert_eq!(archive.name_for_index(0), Some(content_types::PART));
}

#[test]
fn shared_part_bytes_do_not_stay_in_the_package() {
    let pkg = Package::read(&sample().to_bytes().unwrap()).unwrap();
    let name = "word/media/image1.png";
    let first = pkg.part_arc(name).unwrap();
    let index = pkg.find(name).unwrap();
    assert!(pkg.parts[index].data.get().is_none(), "reading shared bytes must not inflate the part into the package");

    let second = pkg.part_arc(name).unwrap();
    assert_eq!(&*first, &*second);
    assert!(!Arc::ptr_eq(&first, &second), "unchanged source bytes are decompressed for each caller, then dropped");

    assert_eq!(pkg.part(name).unwrap(), &*first);
    assert!(pkg.parts[index].data.get().is_some(), "the borrowing read keeps its historic cache");
}

#[test]
fn shared_package_bytes_are_not_copied() {
    let bytes: Arc<[u8]> = Arc::from(sample().to_bytes().unwrap());
    let pkg = Package::read_arc(bytes.clone()).unwrap();
    assert!(Arc::strong_count(&bytes) > 1, "the package keeps the shared bytes themselves");
    assert_eq!(pkg.part("word/media/image1.png").unwrap(), &[0x89, b'P', b'N', b'G', 1, 2, 3]);
}

#[test]
fn unchanged_round_trip_is_byte_identical() {
    let first = sample().to_bytes().unwrap();
    let second = Package::read(&first).unwrap().to_bytes().unwrap();
    assert_eq!(first, second);
}

#[test]
fn only_replaced_parts_change() {
    let original = sample().to_bytes().unwrap();
    let mut pkg = Package::read(&original).unwrap();
    pkg.replace("word/media/image1.png", vec![9, 9, 9]).unwrap();
    let saved = Package::read(&pkg.to_bytes().unwrap()).unwrap();
    let before = Package::read(&original).unwrap();
    assert_eq!(saved.part("word/media/image1.png").unwrap(), &[9, 9, 9]);
    assert_eq!(saved.part("word/document.xml").unwrap(), before.part("word/document.xml").unwrap());
    assert_eq!(saved.part_names().collect::<Vec<_>>(), before.part_names().collect::<Vec<_>>());
}

#[test]
fn case_insensitive_names_and_leading_slashes() {
    let pkg = Package::read(&sample().to_bytes().unwrap()).unwrap();
    assert!(pkg.has("/word/document.xml"));
    assert!(pkg.has("Word/Document.XML"));
    assert!(matches!(pkg.part("word/missing.xml"), Err(crate::Error::MissingPart(_))));
}

#[test]
fn removing_a_part_takes_its_relationships_and_override() {
    let mut pkg = sample();
    pkg.remove("word/document.xml");
    assert!(!pkg.has("word/_rels/document.xml.rels"));
    assert_eq!(pkg.content_type("word/document.xml"), Some(types::XML), "back to the xml default");
}

#[test]
fn relate_reuses_and_numbers_ids() {
    let mut pkg = sample();
    let a = pkg.relate("word/document.xml", kind::IMAGE, "word/media/image1.png").unwrap();
    assert_eq!(a, "rId1");
    let b = pkg.relate_external("word/document.xml", kind::HYPERLINK, "https://example.com").unwrap();
    assert_eq!(b, "rId2");
    assert_eq!(pkg.relate_external("word/document.xml", kind::HYPERLINK, "https://example.com").unwrap(), "rId2");
    assert_eq!(pkg.unused_name("word/media/image{}.png"), "word/media/image2.png");
}

#[test]
fn rejects_what_isnt_a_package() {
    assert!(matches!(Package::read(b"not a zip"), Err(crate::Error::Zip(_))));
    // A zip without [Content_Types].xml isn't an Office file.
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("hello.txt", SimpleFileOptions::default()).unwrap();
    zip.write_all(b"hi").unwrap();
    let bytes = zip.finish().unwrap().into_inner();
    assert!(matches!(Package::read(&bytes), Err(crate::Error::MissingPart(_))));
}

#[test]
fn properties_round_trip() {
    let mut pkg = sample();
    assert_eq!(Properties::read(&pkg).unwrap(), Properties::default());
    let props = Properties {
        title: Some("Q3 & Q4 plan".into()),
        creator: Some("Ada".into()),
        created: Some("2026-10-20T09:30:00Z".into()),
        application: Some("Writan".into()),
        ..Default::default()
    };
    props.write(&mut pkg).unwrap();
    let pkg = Package::read(&pkg.to_bytes().unwrap()).unwrap();
    assert_eq!(Properties::read(&pkg).unwrap(), props);
    let core = String::from_utf8(pkg.part("docProps/core.xml").unwrap().to_vec()).unwrap();
    assert!(core.contains(r#"<dcterms:created xsi:type="dcterms:W3CDTF">2026-10-20T09:30:00Z</dcterms:created>"#), "{core}");
    assert_eq!(pkg.content_type("docProps/core.xml"), Some(types::CORE_PROPERTIES));
    assert!(pkg.rels("").unwrap().first(kind::CORE_PROPERTIES).is_some());
}

#[test]
fn properties_keep_what_the_model_doesnt_cover() {
    let mut pkg = sample();
    let mut root = Element::new(ns::CP, "coreProperties");
    root.declare(Some("cp"), ns::CP);
    root.push(Element::new(ns::DC, "language").with_text("en-GB"));
    root.push(Element::new(ns::DC, "title").with_text("Old"));
    pkg.put_xml("docProps/core.xml", &XmlDoc::new(root), types::CORE_PROPERTIES);
    pkg.relate("", kind::CORE_PROPERTIES, "docProps/core.xml").unwrap();

    let mut props = Properties::read(&pkg).unwrap();
    assert_eq!(props.title.as_deref(), Some("Old"));
    props.title = Some("New".into());
    props.write(&mut pkg).unwrap();
    let doc = pkg.xml("docProps/core.xml").unwrap();
    assert_eq!(doc.root.child(ns::DC, "language").unwrap().text(), "en-GB");
    assert_eq!(doc.root.child(ns::DC, "title").unwrap().text(), "New");

    // Writing the same properties again leaves the part alone.
    let before = pkg.to_bytes().unwrap();
    let mut again = Package::read(&before).unwrap();
    props.write(&mut again).unwrap();
    assert_eq!(again.to_bytes().unwrap(), before);
}

#[test]
fn a_part_bigger_than_it_says_is_refused() {
    let mut pkg = sample();
    pkg.put("word/big.bin", vec![0; 4096], "application/octet-stream");
    let mut bytes = pkg.to_bytes().unwrap();
    // Say the part is 16 bytes, in its local header and in the central directory.
    let name = b"word/big.bin";
    let find = |bytes: &[u8], sig: &[u8], name_at: usize| {
        (0..bytes.len() - name_at - name.len()).find(|&i| bytes[i..].starts_with(sig) && bytes[i + name_at..].starts_with(name)).unwrap()
    };
    let local = find(&bytes, b"PK\x03\x04", 30);
    bytes[local + 22..local + 26].copy_from_slice(&16u32.to_le_bytes());
    let central = find(&bytes, b"PK\x01\x02", 46);
    bytes[central + 24..central + 28].copy_from_slice(&16u32.to_le_bytes());
    let pkg = Package::read(&bytes).unwrap();
    assert!(pkg.part("word/document.xml").is_ok());
    assert!(pkg.part("word/big.bin").is_err());
}

#[test]
fn backslashes_in_entry_names_are_folders() {
    // Some writers name entries `xl\workbook.xml`, and `[content_types].xml` in lower case.
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let o = SimpleFileOptions::default();
    let src = sample();
    for name in std::iter::once(content_types::PART).chain(src.part_names()) {
        let data = if name == content_types::PART { src.content_types.to_xml().to_bytes() } else { src.part(name).unwrap().to_vec() };
        let written = if name == content_types::PART { "[content_types].xml".to_string() } else { name.replace('/', "\\") };
        zip.start_file(written, o).unwrap();
        zip.write_all(&data).unwrap();
    }
    let bytes = zip.finish().unwrap().into_inner();
    let pkg = Package::read(&bytes).unwrap();
    assert_eq!(pkg.main_part().unwrap(), "word/document.xml");
    assert_eq!(pkg.related("word/document.xml", kind::IMAGE).unwrap()[0].1, "word/media/image1.png");
    // Saved with `/`, and the content types under their proper name.
    let saved = pkg.to_bytes().unwrap();
    let mut names: Vec<String> = ZipArchive::new(Cursor::new(saved)).unwrap().file_names().map(str::to_string).collect();
    names.sort();
    assert_eq!(names, ["[Content_Types].xml", "_rels/.rels", "word/_rels/document.xml.rels", "word/document.xml", "word/media/image1.png"]);
}

#[test]
fn oversized_xml_is_refused_before_it_is_inflated() {
    for name in [content_types::PART, "word/document.xml"] {
        let mut bytes = sample().to_bytes().unwrap();
        let name_bytes = name.as_bytes();
        let find = |sig: &[u8], name_at: usize| {
            (0..bytes.len() - name_at - name_bytes.len())
                .find(|&i| bytes[i..].starts_with(sig) && bytes[i + name_at..].starts_with(name_bytes))
                .unwrap()
        };
        let local = find(b"PK\x03\x04", 30);
        let central = find(b"PK\x01\x02", 46);
        let size = (MAX_XML_SIZE as u32 + 1).to_le_bytes();
        bytes[local + 22..local + 26].copy_from_slice(&size);
        bytes[central + 24..central + 28].copy_from_slice(&size);
        let error = if name == content_types::PART {
            Package::read(&bytes).unwrap_err().to_string()
        } else {
            let package = Package::read(&bytes).unwrap();
            let error = package.xml_bytes(name).unwrap_err().to_string();
            assert!(package.xml(name).unwrap_err().to_string().contains("too large"));
            assert!(package.xml_shared(name).unwrap_err().to_string().contains("too large"));
            // Unopened large parts can still pass through without being inflated.
            assert!(package.to_bytes().is_ok());
            error
        };
        assert!(error.contains("too large to open") && error.contains("128 MB"), "{error}");
    }
}
