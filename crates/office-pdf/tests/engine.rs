use std::collections::HashMap;

use lopdf::{content::Operation, dictionary, Dictionary, Document, Object, ObjectId, Stream};
use office_pdf::{
    images_to_pdf, inject_ocr_text_layer, inspect_pdf, render_document_to_pdf, unpack_blobs, ImagePdfPageInput, OcrPageLayer,
    PdfDocumentInput,
};
use serde_json::{json, Value};

fn fonts() -> HashMap<String, Vec<u8>> {
    macro_rules! font {
        ($file:literal) => {
            include_bytes!(concat!("../../../src/engine/fonts/", $file)).to_vec()
        };
    }
    HashMap::from([
        ("carlito:regular".into(), font!("Carlito-Regular.ttf")),
        ("carlito:bold-italic".into(), font!("Carlito-BoldItalic.ttf")),
        ("liberation-sans:regular".into(), font!("LiberationSans-Regular.ttf")),
        ("noto-arabic:regular".into(), font!("pdf/NotoSansArabic-Regular.ttf")),
        ("noto-math:regular".into(), font!("pdf/NotoSansMath-Regular.ttf")),
        ("noto-sans:regular".into(), font!("pdf/NotoSans-Regular.ttf")),
    ])
}

fn page(items: Value) -> Value {
    json!({"widthPt": 240, "heightPt": 180, "scaleX": 1.25, "scaleY": 0.75, "offsetXPt": 4, "offsetYPt": 8,
        "clipRectPx": [0,0,180,220], "items": items})
}

fn doc(pages: Vec<Value>, styles: Value) -> PdfDocumentInput {
    serde_json::from_value(json!({"pages": pages, "styles": styles,
        "metadata": {"title": "Office Ω", "author": "Writan", "subject": "Vector export", "creator": "Writan Test"}}))
    .unwrap()
}

fn text(x: f64, y: f64, s: usize, text: &str) -> Value {
    json!({"t": "text", "x": x, "y": y, "w": 65, "s": s, "text": text})
}

fn loaded(bytes: &[u8]) -> Document {
    Document::load_mem(bytes).unwrap()
}

fn res(doc: &Document, id: ObjectId) -> &Dictionary {
    doc.get_dictionary(id).unwrap().get_deref(b"Resources", doc).unwrap().as_dict().unwrap()
}

fn ops(doc: &Document, page: ObjectId) -> Vec<Operation> {
    doc.get_and_decode_page_content(page).unwrap().operations
}

fn png() -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(&[40, 80, 120, 255, 1, 2, 3, 80]).unwrap();
    bytes
}

const JPEG: &[u8] = include_bytes!("fixtures/pixels.jpg");

#[test]
fn blob_pack_bounds_and_utf8() {
    assert!(unpack_blobs(&[]).is_err());
    assert!(unpack_blobs(&u32::MAX.to_le_bytes()).is_err());
    assert_eq!(unpack_blobs(&[0; 4]).unwrap().len(), 0);
    let pack = [1u32.to_le_bytes().as_slice(), &2u32.to_le_bytes(), "á".as_bytes(), &3u32.to_le_bytes(), &[5, 6, 7]].concat();
    assert_eq!(unpack_blobs(&pack).unwrap()["á"], [5, 6, 7]);
    for end in 0..pack.len() {
        assert!(unpack_blobs(&pack[..end]).is_err());
    }
    let mut invalid = pack.clone();
    invalid[8] = 255;
    assert!(unpack_blobs(&invalid).is_err());
    invalid = pack.clone();
    invalid.push(0);
    assert!(unpack_blobs(&invalid).is_err());
    let repeated = [2u32.to_le_bytes().as_slice(), &pack[4..], &pack[4..]].concat();
    assert!(unpack_blobs(&repeated).is_err());
}

#[test]
fn word_pages_and_transformed_safe_links() {
    let mut links: Vec<_> =
        ["https://example.test/part", "mailto:help@example.test", "javascript:alert(1)", "file:///private", "https://", "data:,x"]
            .iter()
            .enumerate()
            .map(|(i, link)| {
                let mut item = text(1., 12. + i as f64 * 15., 0, "Link");
                item["link"] = json!(link);
                item
            })
            .collect();
    links.push(text(6., 105., 1, "Office straße"));
    let input = doc(
        vec![
            page(json!([{"t":"group","x":12,"y":20,"w":80,"h":115,"rot":90,"fh":true,"clip":true,"id":"hf-dim","items":links}])),
            page(json!([text(10., 20., 0, "Next page"), text(10., 42., 2, "Struck and dashed"), text(10., 64., 3, "Dotted")])),
        ],
        json!([
            {"font":"bold italic 14.67px \"Calibri\", \"Carlito\", sans-serif","color":"#123","ascent":8,"descent":2},
            {"font":"16px Calibri","color":"#222", "smallCaps":true,"shift":2,"decoration":"underline wavy", "background":"yellow"},
            {"font":"12px Calibri","color":"black","decoration":"line-through underline dashed"},
            {"font":"12px Calibri","color":"black","decoration":"underline dotted"},
        ]),
    );
    let bytes = render_document_to_pdf(&input, &HashMap::new(), &fonts()).unwrap();
    let pdf = loaded(&bytes);
    assert_eq!(pdf.version, "1.7");
    let info = pdf.trailer.get_deref(b"Info", &pdf).unwrap().as_dict().unwrap();
    assert_eq!(lopdf::decode_text_string(info.get(b"Title").unwrap()).unwrap(), "Office Ω");
    assert_eq!(lopdf::decode_text_string(info.get(b"Producer").unwrap()).unwrap(), "Writan PDF Engine");
    let pages = pdf.get_pages();
    assert_eq!(pages.len(), 2);
    let first = pages[&1];
    let operations = ops(&pdf, first);
    assert!(!operations.iter().any(|o| o.operator == "gs"), "opaque header/footer");
    assert!(operations.iter().any(|o| o.operator == "c"), "wavy underline");
    assert!(operations.iter().any(|o| o.operator == "BDC"), "small caps preserve original casing");
    assert!(operations.iter().any(|o| o.operator == "Tf" && (o.operands[1].as_float().unwrap() - 12.48).abs() < 0.001));
    assert!(ops(&pdf, pages[&2]).iter().any(|o| o.operator == "d"));
    let page = pdf.get_dictionary(first).unwrap();
    let annotations = page.get(b"Annots").unwrap().as_array().unwrap();
    assert_eq!(annotations.len(), 2);
    for (object, uri) in annotations.iter().zip(["https://example.test/part", "mailto:help@example.test"]) {
        let link = pdf.get_dictionary(object.as_reference().unwrap()).unwrap();
        assert_eq!(link.get(b"Subtype").unwrap().as_name().unwrap(), b"Link");
        assert_eq!(link.get(b"Border").unwrap().as_array().unwrap(), &[0.into(), 0.into(), 0.into()]);
        let action = link.get(b"A").unwrap().as_dict().unwrap();
        assert_eq!(action.get(b"S").unwrap().as_name().unwrap(), b"URI");
        assert_eq!(action.get(b"URI").unwrap().as_str().unwrap(), uri.as_bytes());
    }
    let link = pdf.get_dictionary(annotations[0].as_reference().unwrap()).unwrap();
    let coords: Vec<_> = link.get(b"Rect").unwrap().as_array().unwrap().iter().map(|o| o.as_float().unwrap()).collect();
    assert_eq!(coords, [123.375, 84.625, 135.875, 133.375]);
    assert_eq!(inspect_pdf(&bytes).unwrap().annotation_count, 2);
}

#[test]
fn table_gradients_and_svg_vectors() {
    let mut p = page(json!([
        {"t":"rect","x":0,"y":0,"w":150,"h":24,"fill":"linear-gradient(90deg, rgba(0, 140, 255, .3) 0%, #aaffff 35%, #fff 100%)"},
        {"t":"rect","x":0,"y":24,"w":150,"h":24,"fill":"#eee"},
        {"t":"line","x1":0,"y1":0,"x2":150,"y2":0,"w":1,"color":"black","dash":"dashed"},
        {"t":"line","x1":0,"y1":48,"x2":150,"y2":48,"w":3,"color":"#005","dash":"double"},
        {"t":"path","x":5,"y":24,"d":"M0 0 l5 0 h5 v3 C10 8 15 2 20 5 s8 5 10 1 Q35 0 40 5 t10 0 A5 5 0 0 1 55 10 z","fill":"red","stroke":"blue","sw":1,"fillRule":"evenodd"}
    ]));
    p["svgs"] = json!([
        {"xPx":4,"yPx":52,"wPx":110,"hPx":35,"svg":r##"<svg viewBox="0 -10 50 20" style="color:navy"><defs><path id="m" d="M0 0 L10 0 L10 4 Z"/></defs><g fill="currentColor" transform="scale(1,-1) skewX(12)"><use xlink:href="#m" x="10"/><use href="#m" transform="matrix(1,0,0,1,22,0)"/><line x1="2" x2="37" y1="8" y2="8" stroke="currentColor"/></g></svg>"##},
        {"xPx":5,"yPx":94,"wPx":130,"hPx":70,"svg":r##"<svg viewBox="0 0 130 70" font-family="Calibri" font-size="10"><rect x=".5" y=".5" width="129" height="69" rx="4" fill="white" stroke="#242424"/><circle cx="80" cy="30" r="7" fill="#ffc400"/><path d="M90 30 A8 8 0 1 1 90 46 A8 8 0 1 1 90 30 Z M90 34 A4 4 0 1 0 90 42 A4 4 0 1 0 90 34 Z" fill="red" fill-rule="evenodd" fill-opacity=".5"/><text x="65" y="14" text-anchor="middle">Revenue &amp; Costs</text><text transform="translate(10 50) rotate(-90)" text-anchor="middle">Axis</text></svg>"##}
    ]);
    let input = doc(vec![p], json!([]));
    let bytes = render_document_to_pdf(&input, &HashMap::new(), &fonts()).unwrap();
    let pdf = loaded(&bytes);
    let first = pdf.get_pages()[&1];
    let resources = res(&pdf, first);
    assert!(!resources.has(b"XObject"));
    let shade = resources.get(b"Shading").unwrap().as_dict().unwrap().iter().next().unwrap().1;
    let shade = pdf.get_dictionary(shade.as_reference().unwrap()).unwrap();
    assert_eq!(shade.get(b"ShadingType").unwrap().as_i64().unwrap(), 2);
    let function = shade.get_deref(b"Function", &pdf).unwrap().as_dict().unwrap();
    assert_eq!(function.get(b"Functions").unwrap().as_array().unwrap().len(), 2);
    assert!(pdf.objects.values().any(|o| o.as_dict().is_ok_and(|d| d.has(b"SMask"))));
    let operations = ops(&pdf, first);
    for operator in ["W", "cm", "sh", "c", "B*", "f*", "BT", "Tm", "Tj", "gs", "d"] {
        assert!(operations.iter().any(|op| op.operator == operator), "missing {operator}");
    }
    let extracted = pdf.extract_text(&[1]).unwrap();
    assert!(extracted.contains("Revenue & Costs"), "{extracted:?}");
    assert!(extracted.contains("Axis"), "{extracted:?}");
    maybe_save("vectors.pdf", &bytes);
    let mut malformed = page(json!([]));
    malformed["svgs"] = json!([{"xPx":0,"yPx":0,"wPx":5,"hPx":5,"svg":"<svg><g></svg>"}]);
    assert!(render_document_to_pdf(&doc(vec![malformed], json!([])), &HashMap::new(), &fonts()).is_err());
}

fn as_hex(s: &str) -> String {
    s.encode_utf16().map(|u| format!("{u:04X}")).collect()
}

#[test]
fn logical_unicode_and_subset_glyphs() {
    let strings = ["שלום עולם", "مرحبا بالعالم", "ABC שלום 123 نهاية", "שָׁלוֹם", "لا", "A⟰B"];
    let mut items: Vec<_> = strings
        .iter()
        .enumerate()
        .map(|(i, value)| {
            let mut item = text(20., 25. + i as f64 * 23., 0, value);
            item["rtl"] = json!(i < 5);
            item
        })
        .collect();
    items.last_mut().unwrap()["ws"] = json!(1.5);
    let stretched = page(json!(items));
    let mut uniform = stretched.clone();
    uniform["scaleX"] = json!(0.75);
    let input = doc(vec![stretched, uniform], json!([{"font":"18px Calibri","color":"black"}]));
    let bytes = render_document_to_pdf(&input, &HashMap::new(), &fonts()).unwrap();
    let pdf = loaded(&bytes);
    let page = pdf.get_pages()[&1];
    let embedded = res(&pdf, page).get_deref(b"Font", &pdf).unwrap().as_dict().unwrap();
    assert!(embedded.len() >= 3);
    let mut cmaps = String::new();
    for (_, reference) in embedded.iter() {
        let font = pdf.get_dictionary(reference.as_reference().unwrap()).unwrap();
        let cmap = font.get_deref(b"ToUnicode", &pdf).unwrap().as_stream().unwrap().decompressed_content().unwrap();
        cmaps.push_str(&String::from_utf8(cmap).unwrap().to_ascii_uppercase());
        let descendant = font.get(b"DescendantFonts").unwrap().as_array().unwrap()[0].as_reference().unwrap();
        let descendant = pdf.get_dictionary(descendant).unwrap();
        let file = descendant
            .get_deref(b"FontDescriptor", &pdf)
            .unwrap()
            .as_dict()
            .unwrap()
            .get_deref(b"FontFile2", &pdf)
            .unwrap()
            .as_stream()
            .unwrap()
            .decompressed_content()
            .unwrap();
        let face = ttf_parser::Face::parse(&file, 0).unwrap();
        assert!(face.number_of_glyphs() < 80);
        assert_eq!(descendant.get(b"CIDToGIDMap").unwrap().as_name().unwrap(), b"Identity");
        let widths = descendant.get(b"W").unwrap().as_array().unwrap();
        assert_eq!(widths[1].as_array().unwrap().len(), face.number_of_glyphs() as usize);
    }
    for ch in ['ש', 'ם', 'م', 'ب', '⟰'] {
        assert!(cmaps.contains(&format!("<{}>", as_hex(&ch.to_string()))), "{ch}: {cmaps}");
    }
    // Emits HarfBuzz's native visual-LTR batched runs so PDFium, MuPDF, and desktop PDF
    // viewers reconstruct logical Unicode without fragmenting RTL runs; lopdf concatenates raw
    // stream order without BiDi reordering (verified end-to-end in pdf-vector-export.mjs).
    let text = pdf.extract_text(&[1]).unwrap().replace('\u{200b}', "");
    let visual = |s: &str| s.chars().rev().collect::<String>();
    for expected in [visual("שלום עולם"), visual("مرحبا بالعالم"), "ABC".into(), "123".into(), "A⟰B".into()] {
        assert!(text.contains(&expected), "missing {expected} in {text:?}");
    }
    let operations = ops(&pdf, page);
    let begin = operations.iter().position(|o| o.operator == "BT").unwrap();
    let end = operations.iter().position(|o| o.operator == "ET").unwrap();
    let tm_count = operations[begin..end].iter().filter(|o| o.operator == "Tm").count();
    assert_eq!(tm_count, 1, "expected batched visual-LTR run for Hebrew phrase, got {tm_count} Tm ops");
    assert!(operations[begin..end].iter().any(|o| o.operator == "Tj" || o.operator == "TJ"));
    let marked: Vec<_> = operations
        .iter()
        .filter(|o| o.operator == "BDC")
        .map(|o| lopdf::decode_text_string(o.operands[1].as_dict().unwrap().get(b"ActualText").unwrap()).unwrap())
        .collect();
    assert!(marked.iter().any(|span| span.contains("שָׁ")));
    maybe_save("rtl.pdf", &bytes);
}

#[test]
fn jpeg_and_png_are_lossless_with_crop_and_ocr() {
    let png = png();
    let media = HashMap::from([("png".into(), png), ("jpeg".into(), JPEG.to_vec())]);
    let pages: Vec<ImagePdfPageInput> = serde_json::from_value(json!([
        {"mediaKey":"jpeg","mime":"image/jpeg","widthPt":240,"heightPt":180,"ocrLayer":{
            "pageIndex":0,"pageWidthPt":240,"pageHeightPt":180,
            "words":[{"text":"searchable","xPt":20,"yPt":30,"wPt":65,"hPt":13,"baselineYPt":40}]
        }},
        {"mediaKey":"png","mime":"image/png","widthPt":240,"heightPt":180}
    ]))
    .unwrap();
    let bytes = images_to_pdf(&pages, &media, &fonts()).unwrap();
    let pdf = loaded(&bytes);
    let streams: Vec<_> = pdf
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .filter(|s| s.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Image"))
        .collect();
    let jpeg = streams.iter().find(|s| s.dict.get(b"Filter").and_then(Object::as_name).unwrap() == b"DCTDecode").unwrap();
    assert_eq!(jpeg.content, JPEG);
    let color = streams.iter().find(|s| s.dict.has(b"SMask")).unwrap();
    let alpha = color.dict.get_deref(b"SMask", &pdf).unwrap().as_stream().unwrap();
    assert_eq!(color.decompressed_content().unwrap(), [40, 80, 120, 1, 2, 3]);
    assert_eq!(alpha.decompressed_content().unwrap(), [255, 80]);
    let operations = ops(&pdf, pdf.get_pages()[&1]);
    assert!(operations.iter().any(|o| o.operator == "Tr" && o.operands == [3.into()]));
    assert!(pdf.extract_text(&[1]).unwrap().contains("searchable"));
    let input = doc(vec![page(json!([{"t":"image","x":20,"y":30,"w":100,"h":25,"media":"png","crop":[0.25,0.25,0.25,0.25]}]))], json!([]));
    let cropped = loaded(&render_document_to_pdf(&input, &media, &HashMap::new()).unwrap());
    let operations = ops(&cropped, cropped.get_pages()[&1]);
    assert!(operations.iter().any(|o| o.operator == "re" && o.operands == [20.into(), 30.into(), 100.into(), 25.into()]));
    assert!(operations
        .iter()
        .any(|o| o.operator == "cm" && o.operands == [200.into(), 0.into(), 0.into(), (-50).into(), (-30).into(), 67.5f32.into()]));
}

fn original_pdf(with_signature: bool, xref_stream: bool) -> Vec<u8> {
    let mut pdf = Document::with_version("1.7");
    if !xref_stream {
        pdf.reference_table.cross_reference_type = lopdf::xref::XrefType::CrossReferenceTable;
    }
    let pages = pdf.new_object_id();
    let form = if with_signature {
        let signature = pdf.add_object(dictionary! {"Type" => "Sig", "ByteRange" => vec![0.into(),100.into(),150.into(),50.into()]});
        let field = pdf.add_object(dictionary! {"FT" => "Sig", "T" => Object::string_literal("Approval"), "V" => signature});
        pdf.add_object(dictionary! {"Fields" => vec![Object::Reference(field)]})
    } else {
        pdf.add_object(dictionary! {"Fields" => Vec::<Object>::new()})
    };
    let bookmark = pdf.add_object(dictionary! {"Type" => "Outlines", "Count" => 0});
    let annotation =
        pdf.add_object(dictionary! {"Type" => "Annot", "Subtype" => "Text", "Rect" => vec![10.into(),10.into(),20.into(),20.into()]});
    let data = pdf.add_object(Stream::new(
        dictionary! {"Type" => "XObject", "Subtype" => "Image", "ColorSpace" => "DeviceRGB",
        "BitsPerComponent" => 8, "Width" => 2, "Height" => 2, "Filter" => "DCTDecode"},
        JPEG.to_vec(),
    ));
    let existing_font = pdf.add_object(dictionary! {"Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica"});
    let fonts = pdf.add_object(dictionary! {"F_WOCR_0" => existing_font});
    let resources = pdf.add_object(dictionary! {"XObject" => dictionary! {"Photo" => data}, "Font" => fonts});
    let content = pdf.add_object(Stream::new(Dictionary::new(), b"70 0 0 70 0 0 cm /Photo Do".to_vec()));
    let mut children = Vec::new();
    for angle in [0, 90, 180, 270, 0] {
        let sub = pdf.new_object_id();
        let page = pdf.add_object(dictionary! {"Type" => "Page", "Parent" => sub, "Annots" => vec![Object::Reference(annotation)],
        "Contents" => if angle == 90 { Object::Array(vec![Object::Reference(content)]) } else { Object::Reference(content) }});
        pdf.set_object(
            sub,
            dictionary! {"Type" => "Pages", "Parent" => pages, "Kids" => vec![Object::Reference(page)], "Count" => 1, "Rotate" => angle},
        );
        children.push(Object::Reference(sub));
    }
    pdf.set_object(
        pages,
        dictionary! {"Type" => "Pages", "Kids" => children, "Count" => 5, "MediaBox" => vec![0.into(),0.into(),250.into(),200.into()],
        "CropBox" => vec![13.into(),27.into(),213.into(),127.into()], "Resources" => resources},
    );
    let root = pdf.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages, "Outlines" => bookmark, "AcroForm" => form});
    let info = pdf.add_object(dictionary! {"Title" => Object::string_literal("Original title")});
    pdf.trailer.set("Root", root);
    pdf.trailer.set("Info", info);
    pdf.trailer.set("ID", vec![Object::string_literal("pdf-test-id"), Object::string_literal("pdf-test-id")]);
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).unwrap();
    bytes
}

#[test]
fn inspection_detects_forms_outlines_annotations_signatures_and_encryption() {
    let pdf = original_pdf(true, false);
    let inspection = inspect_pdf(&pdf).unwrap();
    assert_eq!(inspection.page_count, 5);
    assert!(inspection.has_forms && inspection.has_outlines && inspection.has_digital_signature);
    assert_eq!(inspection.signature_fields, ["Approval"]);
    assert_eq!(inspection.annotation_count, 5);
    assert!(inject_ocr_text_layer(&pdf, &[], &fonts(), false).unwrap_err().contains("signed"));
    let allowed = inject_ocr_text_layer(&pdf, &[], &fonts(), true).unwrap();
    assert!(!inspect_pdf(&allowed).unwrap().has_digital_signature);
    assert!(inspect_pdf(b"%PDF-1.7\ntruncated").is_err());
    let mut encrypted = loaded(&original_pdf(false, false));
    let state = lopdf::EncryptionState::try_from(lopdf::EncryptionVersion::V1 {
        document: &encrypted,
        owner_password: "owner",
        user_password: "",
        permissions: lopdf::Permissions::all(),
    })
    .unwrap();
    encrypted.encrypt(&state).unwrap();
    let mut bytes = Vec::new();
    encrypted.save_to(&mut bytes).unwrap();
    let err = inspect_pdf(&bytes).unwrap_err();
    assert!(err.contains("Encrypted"), "{err}");
}

#[test]
fn ocr_updates_all_rotations_and_preserves_original_pdf() {
    let layers: Vec<OcrPageLayer> = [(200, 100), (100, 200), (200, 100), (100, 200)]
        .iter()
        .enumerate()
        .map(|(i, &(w, h))| {
            serde_json::from_value(json!({
                "pageIndex":i, "pageWidthPt":w, "pageHeightPt":h,
                "words":[{"text":"OCR","xPt":10,"yPt":11,"wPt":32,"hPt":10,"baselineYPt":19},
                         {"text":"שלום עולם","xPt":45,"yPt":11,"wPt":50,"hPt":10,"rtl":true}]
            }))
            .unwrap()
        })
        .collect();
    for xref_stream in [false, true] {
        let src = original_pdf(false, xref_stream);
        let bytes = inject_ocr_text_layer(&src, &layers, &fonts(), false).unwrap();
        assert!(bytes.starts_with(&src), "the original PDF and image stream survive byte for byte");
        let old = loaded(&src);
        let updated = loaded(&bytes);
        assert_eq!(inspect_pdf(&bytes).unwrap().annotation_count, 5);
        for key in [b"AcroForm".as_slice(), b"Outlines"] {
            assert_eq!(updated.catalog().unwrap().get(key).unwrap(), old.catalog().unwrap().get(key).unwrap());
        }
        assert_eq!(updated.trailer.get(b"Info").unwrap(), old.trailer.get(b"Info").unwrap());
        let rotations = [[1, 0, 0, -1, 13, 127], [0, 1, 1, 0, 13, 27], [-1, 0, 0, 1, 213, 27], [0, -1, -1, 0, 213, 127]];
        for (i, (n, id)) in updated.get_pages().into_iter().enumerate() {
            let page = updated.get_dictionary(id).unwrap();
            assert_eq!(page.get(b"Annots").unwrap(), old.get_dictionary(id).unwrap().get(b"Annots").unwrap());
            if n == 5 {
                assert!(!page.has(b"Resources"));
                assert_eq!(old.get_page_contents(id), updated.get_page_contents(id));
                continue;
            }
            let streams = updated.get_page_contents(id);
            assert_eq!(streams.len(), 3, "page {n}");
            assert_eq!(streams[1], old.get_page_contents(id)[0]);
            let prefix = updated.get_object(streams[0]).unwrap().as_stream().unwrap();
            assert_eq!(&prefix.content, b"q\n");
            let suffix = updated.get_object(streams[2]).unwrap().as_stream().unwrap().decompressed_content().unwrap();
            assert!(suffix.starts_with(b"\nQ\nq\n"));
            let ops = ops(&updated, id);
            let matrix = rotations[i].into_iter().map(Object::from).collect::<Vec<_>>();
            assert!(ops.iter().any(|o| o.operator == "cm" && o.operands == matrix));
            assert!(ops.iter().any(|o| o.operator == "Tr" && o.operands == [3.into()]));
            assert!(ops.iter().any(|o| o.operator == "Tz"));
            let fonts = res(&updated, id).get_deref(b"Font", &updated).unwrap().as_dict().unwrap();
            assert!(fonts.has(b"F_WOCR_0") && fonts.len() >= 2);
            let extracted = updated.extract_text(&[n]).unwrap();
            assert!(extracted.contains("OCR"));
            assert!(extracted.contains(&"שלום עולם".chars().rev().collect::<String>()));
        }
        if xref_stream {
            maybe_save("ocr-original.pdf", &src);
            maybe_save("ocr.pdf", &bytes);
        }
    }
}

#[test]
fn signed_ocr_copy_strips_signature_and_keeps_annotations_and_outlines() {
    let pdf = original_pdf(true, false);
    let layers: Vec<OcrPageLayer> = vec![serde_json::from_value(json!({
        "pageIndex": 0, "pageWidthPt": 200, "pageHeightPt": 100,
        "words": [{"text": "Verified", "xPt": 10, "yPt": 11, "wPt": 45, "hPt": 10, "baselineYPt": 19}]
    }))
    .unwrap()];
    assert!(inject_ocr_text_layer(&pdf, &layers, &fonts(), false).unwrap_err().contains("signed"));
    let allowed = inject_ocr_text_layer(&pdf, &layers, &fonts(), true).unwrap();
    let inspected = inspect_pdf(&allowed).unwrap();
    assert!(!inspected.has_digital_signature);
    assert!(inspected.signature_fields.is_empty());
    assert!(inspected.has_outlines);
    assert_eq!(inspected.annotation_count, 5);
    assert!(loaded(&allowed).extract_text(&[1]).unwrap().contains("Verified"));
}

#[test]
fn kerned_latin_emits_tj_adjustments_from_nominal_widths() {
    let input = doc(vec![page(json!([text(10., 40., 0, "AVATAR To Wave")]))], json!([{"font":"24px Calibri","color":"black"}]));
    let bytes = render_document_to_pdf(&input, &HashMap::new(), &fonts()).unwrap();
    let pdf = loaded(&bytes);
    let page_id = pdf.get_pages()[&1];
    let page_ops = ops(&pdf, page_id);
    let has_tj_kerning = page_ops.iter().any(|op| {
        op.operator == "TJ"
            && op.operands.first().and_then(|o| o.as_array().ok()).is_some_and(|arr| {
                arr.iter().any(|el| matches!(el, Object::Real(r) if r.abs() >= 0.5) || matches!(el, Object::Integer(i) if *i != 0))
            })
    });
    assert!(has_tj_kerning, "expected non-zero TJ kerning adjustment in AVATAR To Wave");
    assert!(pdf.extract_text(&[1]).unwrap().contains("AVATAR To Wave"));
}

#[test]
fn jpeg_exif_orientation_6_preserves_dct_and_rotates_transform() {
    let app1: &[u8] = &[
        0xff, 0xe1, 0x00, 0x1e, b'E', b'x', b'i', b'f', 0x00, 0x00, b'M', b'M', 0x00, 0x2a, 0x00, 0x00, 0x00, 0x08, 0x00, 0x01, 0x01, 0x12,
        0x00, 0x03, 0x00, 0x00, 0x00, 0x01, 0x00, 0x06, 0x00, 0x00,
    ];
    let mut exif_jpeg = Vec::with_capacity(JPEG.len() + app1.len());
    exif_jpeg.extend_from_slice(&JPEG[..2]);
    exif_jpeg.extend_from_slice(app1);
    exif_jpeg.extend_from_slice(&JPEG[2..]);
    let pages =
        vec![ImagePdfPageInput { media_key: "photo".into(), mime: "image/jpeg".into(), width_pt: 200., height_pt: 100., ocr_layer: None }];
    let bytes = images_to_pdf(&pages, &HashMap::from([("photo".into(), exif_jpeg.clone())]), &fonts()).unwrap();
    let pdf = loaded(&bytes);
    let xobjs = res(&pdf, pdf.get_pages()[&1]).get_deref(b"XObject", &pdf).unwrap().as_dict().unwrap();
    let img_ref = xobjs.iter().next().unwrap().1.as_reference().unwrap();
    let img_stream = pdf.get_object(img_ref).unwrap().as_stream().unwrap();
    assert_eq!(img_stream.dict.get(b"Filter").unwrap().as_name().unwrap(), b"DCTDecode");
    assert_eq!(img_stream.content, exif_jpeg);
    assert!(ops(&pdf, pdf.get_pages()[&1])
        .iter()
        .any(|op| op.operator == "cm" && op.operands == [0.into(), 100.into(), 200.into(), 0.into(), 0.into(), 0.into()]));
}

#[test]
fn oriented_ocr_word_emits_rotated_matrix_and_extracts_text() {
    let src = original_pdf(false, false);
    let layers: Vec<OcrPageLayer> = vec![serde_json::from_value(json!({
        "pageIndex": 0,
        "pageWidthPt": 200,
        "pageHeightPt": 100,
        "words": [
            {
                "text": "VERTICAL",
                "xPt": 34, "yPt": 20, "wPt": 12, "hPt": 60,
                "baselineX0Pt": 40, "baselineY0Pt": 20,
                "baselineX1Pt": 40, "baselineY1Pt": 80,
                "orientedHPt": 12
            },
            {
                "text": "SKEWED",
                "xPt": 90, "yPt": 40, "wPt": 60, "hPt": 18,
                "baselineX0Pt": 90, "baselineY0Pt": 55,
                "baselineX1Pt": 150, "baselineY1Pt": 45,
                "orientedHPt": 10
            }
        ]
    }))
    .unwrap()];
    let bytes = inject_ocr_text_layer(&src, &layers, &fonts(), false).unwrap();
    let pdf = loaded(&bytes);
    let page_ops = ops(&pdf, pdf.get_pages()[&1]);
    assert!(page_ops
        .iter()
        .any(|op| op.operator == "cm" && op.operands == [0.into(), 1.into(), (-1).into(), 0.into(), 40.into(), 20.into()]));
    let extracted = pdf.extract_text(&[1]).unwrap();
    assert!(extracted.contains("VERTICAL"), "{extracted:?}");
    assert!(extracted.contains("SKEWED"), "{extracted:?}");
}

#[test]
fn signed_form_widget_kids_are_stripped_while_regular_fields_survive() {
    let mut pdf = Document::with_version("1.7");
    let pages = pdf.new_object_id();
    let page = pdf.new_object_id();
    let sig_val = pdf.add_object(dictionary! {"Type" => "Sig", "ByteRange" => vec![0.into(), 100.into(), 150.into(), 50.into()]});
    let sig_field = pdf.new_object_id();
    let sig_ap = pdf.add_object(Stream::new(
        dictionary! {"Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 50.into(), 20.into()]},
        b"q Q".to_vec(),
    ));
    let sig_widget = pdf.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "Parent" => sig_field,
        "Rect" => vec![10.into(), 10.into(), 60.into(), 30.into()],
        "AP" => dictionary! {"N" => sig_ap}
    });
    pdf.set_object(
        sig_field,
        dictionary! {"FT" => "Sig", "T" => Object::string_literal("Signer"), "V" => sig_val, "Kids" => vec![Object::Reference(sig_widget)]},
    );
    let tx_widget = pdf.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "T" => Object::string_literal("Notes"),
        "Rect" => vec![10.into(), 40.into(), 120.into(), 60.into()], "V" => Object::string_literal("ok")
    });
    let form = pdf.add_object(dictionary! {"SigFlags" => 3, "Fields" => vec![Object::Reference(sig_field), Object::Reference(tx_widget)]});
    let content = pdf.add_object(Stream::new(Dictionary::new(), b"q Q".to_vec()));
    pdf.set_object(
        page,
        dictionary! {"Type" => "Page", "Parent" => pages, "MediaBox" => vec![0.into(), 0.into(), 200.into(), 100.into()],
        "Annots" => vec![Object::Reference(sig_widget), Object::Reference(tx_widget)], "Contents" => content},
    );
    pdf.set_object(pages, dictionary! {"Type" => "Pages", "Kids" => vec![Object::Reference(page)], "Count" => 1});
    let root = pdf.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages, "AcroForm" => form});
    pdf.trailer.set("Root", root);
    let mut src = Vec::new();
    pdf.save_to(&mut src).unwrap();

    let layers: Vec<OcrPageLayer> = vec![serde_json::from_value(json!({
        "pageIndex": 0, "pageWidthPt": 200, "pageHeightPt": 100,
        "words": [{"text": "Clean", "xPt": 10, "yPt": 20, "wPt": 30, "hPt": 10}]
    }))
    .unwrap()];
    let out = inject_ocr_text_layer(&src, &layers, &fonts(), true).unwrap();
    let inspected = inspect_pdf(&out).unwrap();
    assert!(!inspected.has_digital_signature);
    assert!(inspected.has_forms);
    assert_eq!(inspected.annotation_count, 1);
    let cleaned = loaded(&out);
    assert!(!cleaned.objects.contains_key(&sig_widget));
    let remaining_form = cleaned.catalog().unwrap().get_deref(b"AcroForm", &cleaned).unwrap().as_dict().unwrap();
    assert!(!remaining_form.has(b"SigFlags"));
    assert_eq!(remaining_form.get_deref(b"Fields", &cleaned).unwrap().as_array().unwrap().len(), 1);
}

fn maybe_save(filename: &str, bytes: &[u8]) {
    if let Some(dir) = std::env::var_os("WRITAN_PDF_TEST_OUTPUT") {
        std::fs::write(std::path::PathBuf::from(dir).join(filename), bytes).unwrap();
    }
}
