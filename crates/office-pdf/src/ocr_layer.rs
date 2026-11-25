use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use lopdf::{decode_text_string, Dictionary, Document, IncrementalDocument, Object, ObjectId, Stream};
use pdf_writer::{types::TextRenderingMode, Content, Name, Pdf, Ref};

use crate::flate;
use crate::fonts::{FontId, FontSpec, Fonts};
use crate::model::{OcrPageLayer, PdfInspection};
use crate::svg_path::Matrix;

fn load_pdf(pdf_bytes: &[u8]) -> Result<Document, String> {
    let doc = Document::load_mem(pdf_bytes).map_err(|e| format!("Could not read PDF: {e}"))?;
    if doc.is_encrypted() || doc.encryption_state.is_some() {
        return Err("Encrypted PDFs are unsupported".into());
    }
    Ok(doc)
}

pub fn inspect_pdf(pdf_bytes: &[u8]) -> Result<PdfInspection, String> {
    let doc = load_pdf(pdf_bytes)?;
    let catalog = doc.catalog().map_err(|e| format!("Invalid PDF catalog: {e}"))?;
    let pages = page_ids(&doc)?;
    let mut annotation_count = 0;
    for page in &pages {
        let page = doc.get_dictionary(*page).map_err(|e| e.to_string())?;
        if page.has(b"Annots") {
            let annots = page.get_deref(b"Annots", &doc).and_then(Object::as_array).map_err(|e| format!("Invalid PDF annotations: {e}"))?;
            annotation_count += annots.len();
        }
    }
    let (has_digital_signature, signature_fields) = signatures(&doc);
    Ok(PdfInspection {
        page_count: pages.len(),
        encrypted: false,
        has_digital_signature,
        signature_fields,
        has_forms: catalog.has(b"AcroForm"),
        has_outlines: catalog.has(b"Outlines"),
        annotation_count,
    })
}

fn signatures(doc: &Document) -> (bool, Vec<String>) {
    let mut names = BTreeSet::new();
    let mut found = false;
    let mut objects: Vec<_> = doc.objects.values().collect();
    while let Some(object) = objects.pop() {
        let dict = match object {
            Object::Array(items) => {
                objects.extend(items);
                continue;
            }
            Object::Dictionary(dict) => dict,
            Object::Stream(stream) => &stream.dict,
            _ => continue,
        };
        objects.extend(dict.iter().map(|(_, value)| value));
        let has_name = |key| dict.get_deref(key, doc).and_then(Object::as_name).ok() == Some(b"Sig");
        if has_name(b"Type") || has_name(b"FT") || dict.has(b"ByteRange") {
            found = true;
            let mut seen = HashSet::new();
            let mut parts = Vec::new();
            let mut field = dict;
            loop {
                if let Ok(name) = field.get_deref(b"T", doc).and_then(decode_text_string) {
                    parts.push(name);
                }
                let Ok(id) = field.get(b"Parent").and_then(Object::as_reference) else {
                    break;
                };
                if !seen.insert(id) {
                    break;
                }
                let Ok(parent) = doc.get_dictionary(id) else {
                    break;
                };
                field = parent;
            }
            if !parts.is_empty() {
                parts.reverse();
                names.insert(parts.join("."));
            }
        }
    }
    (found, names.into_iter().collect())
}

fn page_ids(doc: &Document) -> Result<Vec<ObjectId>, String> {
    let catalog = doc.catalog().map_err(|e| format!("Invalid PDF catalog: {e}"))?;
    let root = catalog.get(b"Pages").and_then(Object::as_reference).map_err(|e| format!("Invalid PDF page tree: {e}"))?;
    let mut pending = vec![root];
    let mut seen = HashSet::new();
    let mut pages = Vec::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            return Err("Cyclic or repeated node in PDF page tree".into());
        }
        let node = doc.get_dictionary(id).map_err(|e| format!("Invalid PDF page tree: {e}"))?;
        match node.get(b"Type").and_then(Object::as_name) {
            Ok(b"Page") => pages.push(id),
            Ok(b"Pages") => {
                let children =
                    node.get_deref(b"Kids", doc).and_then(Object::as_array).map_err(|e| format!("Invalid PDF page children: {e}"))?;
                for child in children.iter().rev() {
                    pending.push(child.as_reference().map_err(|e| format!("Invalid PDF child page: {e}"))?);
                }
            }
            _ => return Err("Invalid PDF page tree node type".into()),
        }
    }
    Ok(pages)
}

fn inherited<'a>(doc: &'a Document, mut id: ObjectId, key: &[u8]) -> Result<Option<&'a Object>, String> {
    let mut seen = HashSet::new();
    loop {
        if !seen.insert(id) {
            return Err("Cyclic PDF page inheritance".into());
        }
        let dict = doc.get_dictionary(id).map_err(|e| format!("Invalid PDF page parent: {e}"))?;
        if let Ok(value) = dict.get(key) {
            let (_, value) = doc.dereference(value).map_err(|e| format!("Invalid inherited PDF property: {e}"))?;
            if !matches!(value, Object::Null) {
                return Ok(Some(value));
            }
        }
        let Ok(parent) = dict.get(b"Parent") else {
            return Ok(None);
        };
        id = parent.as_reference().map_err(|e| format!("Invalid PDF parent reference: {e}"))?;
    }
}

fn page_box(doc: &Document, value: &Object) -> Result<[f64; 4], String> {
    let values = value.as_array().map_err(|e| format!("Invalid PDF page box: {e}"))?;
    if values.len() != 4 {
        return Err("PDF page box must have four coordinates".into());
    }
    let mut coords = [0.; 4];
    for (target, value) in coords.iter_mut().zip(values) {
        let (_, value) = doc.dereference(value).map_err(|e| e.to_string())?;
        *target = match value {
            Object::Integer(int) => *int as f64,
            Object::Real(real) if real.is_finite() => f64::from(*real),
            _ => return Err("Invalid PDF page box coordinate".into()),
        };
    }
    let [x, y, x2, y2] = coords;
    Ok([x.min(x2), y.min(y2), x.max(x2), y.max(y2)])
}

fn display_transform(doc: &Document, id: ObjectId, layer: &OcrPageLayer) -> Result<Matrix, String> {
    let media = inherited(doc, id, b"MediaBox")?.ok_or("Missing PDF MediaBox")?;
    let [mut x, mut y, mut r, mut t] = page_box(doc, media)?;
    if let Some(crop) = inherited(doc, id, b"CropBox")? {
        let [cx, cy, cr, ct] = page_box(doc, crop)?;
        x = x.max(cx);
        y = y.max(cy);
        r = r.min(cr);
        t = t.min(ct);
    }
    if r <= x || t <= y {
        return Err("Empty PDF display box".into());
    }
    let rotate = inherited(doc, id, b"Rotate")?
        .map(Object::as_i64)
        .transpose()
        .map_err(|e| format!("Invalid PDF rotation: {e}"))?
        .unwrap_or(0)
        .rem_euclid(360);
    let (transform, width, height) = match rotate {
        0 => (Matrix([1., 0., 0., -1., x, t]), r - x, t - y),
        90 => (Matrix([0., 1., 1., 0., x, y]), t - y, r - x),
        180 => (Matrix([-1., 0., 0., 1., r, y]), r - x, t - y),
        270 => (Matrix([0., -1., -1., 0., r, t]), t - y, r - x),
        _ => return Err(format!("Unsupported PDF page rotation {rotate}")),
    };
    Ok(transform.then_local(layer_scale(layer, width, height)?))
}

pub(crate) fn layer_scale(layer: &OcrPageLayer, width: f64, height: f64) -> Result<Matrix, String> {
    let [w, h] = [layer.page_width_pt, layer.page_height_pt];
    let [sx, sy] = [width / w, height / h];
    if ![sx, sy].iter().all(|&scale| scale.is_finite() && scale > 0. && scale <= 1e12) {
        return Err("Invalid OCR layer dimensions".into());
    }
    Ok(Matrix::scale(sx, sy))
}

fn resources(doc: &Document, id: ObjectId) -> Result<(Dictionary, Dictionary), String> {
    let resources = inherited(doc, id, b"Resources")?
        .map(Object::as_dict)
        .transpose()
        .map_err(|e| format!("Invalid PDF resource dictionary: {e}"))?
        .cloned()
        .unwrap_or_default();
    let existing_fonts = if resources.has(b"Font") {
        resources.get_deref(b"Font", doc).and_then(Object::as_dict).map_err(|e| format!("Invalid PDF font dictionary: {e}"))?.clone()
    } else {
        Dictionary::new()
    };
    Ok((resources, existing_fonts))
}

fn original_contents(doc: &Document, id: ObjectId) -> Result<Vec<Object>, String> {
    let page = doc.get_dictionary(id).map_err(|e| e.to_string())?;
    let Ok(initial) = page.get(b"Contents") else {
        return Ok(Vec::new());
    };
    let (id, contents) = doc.dereference(initial).map_err(|e| format!("Invalid PDF content reference: {e}"))?;
    match contents {
        Object::Null => Ok(Vec::new()),
        Object::Stream(_) => Ok(vec![Object::Reference(id.ok_or("PDF page content stream must be indirect")?)]),
        Object::Array(items) => {
            for item in items {
                let (id, stream) = doc.dereference(item).map_err(|e| format!("Invalid PDF stream reference: {e}"))?;
                if id.is_none() || !matches!(stream, Object::Stream(_)) {
                    return Err("PDF content array must contain indirect streams".into());
                }
            }
            Ok(items.clone())
        }
        _ => Err("Invalid PDF page contents".into()),
    }
}

struct Update {
    page_id: ObjectId,
    resources: Dictionary,
    fonts: Dictionary,
    stream: Vec<u8>,
    contents: Vec<Object>,
}

pub fn inject_ocr_text_layer(
    pdf_bytes: &[u8],
    layers: &[OcrPageLayer],
    blobs: &HashMap<String, Vec<u8>>,
    allow_signed: bool,
) -> Result<Vec<u8>, String> {
    let doc = load_pdf(pdf_bytes)?;
    if !allow_signed && signatures(&doc).0 {
        return Err("OCR cannot modify a digitally signed PDF".into());
    }
    let pages = page_ids(&doc)?;
    let mut by_page = BTreeMap::<ObjectId, Vec<&OcrPageLayer>>::new();
    for layer in layers {
        let id = *pages.get(layer.page_index).ok_or_else(|| format!("OCR page {} is out of range", layer.page_index))?;
        if !layer.words.is_empty() {
            by_page.entry(id).or_default().push(layer);
        }
    }
    let mut occupied = HashSet::new();
    let mut page_resources = BTreeMap::new();
    for &page_id in by_page.keys() {
        let (resources, fonts) = resources(&doc, page_id)?;
        occupied.extend(fonts.iter().map(|(name, _)| name.clone()));
        page_resources.insert(page_id, (resources, fonts));
    }
    let mut fonts = Fonts::new(blobs);
    let names = (0..)
        .map(|i| format!("F_WOCR_{i}"))
        .filter(|name| {
            !occupied
                .iter()
                .any(|existing| existing.strip_prefix(name.as_bytes()).is_some_and(|suffix| suffix.is_empty() || suffix.starts_with(b"_")))
        })
        .take(fonts.len())
        .enumerate()
        .collect();
    let mut updates = Vec::new();
    for (page_id, layers) in by_page {
        let mut content = Content::new();
        for layer in layers {
            content.save_state();
            display_transform(&doc, page_id, layer)?.apply(&mut content);
            write_words(&mut content, layer, &mut fonts, &names)?;
            content.restore_state();
        }
        let (resources, fonts) = page_resources.remove(&page_id).ok_or("Missing OCR page resources")?;
        let raw = [b"\nQ\n", content.finish().as_slice()].concat();
        updates.push(Update { page_id, resources, fonts, stream: flate(&raw)?, contents: original_contents(&doc, page_id)? });
    }
    if fonts.used().next().is_none() {
        return Ok(pdf_bytes.to_vec());
    }
    let font_objects = fonts.used().map(|(_, aliases)| 3 + 2 * aliases as u64).sum::<u64>();
    if u64::from(doc.max_id) + font_objects + updates.len() as u64 + 6 >= u64::from(u32::MAX) {
        return Err("PDF object numbering is exhausted".into());
    }
    let mut updated = IncrementalDocument::create_from(pdf_bytes.to_vec(), doc);
    let refs = append_fonts(&mut updated.new_document, &fonts)?;
    let prefix = updated.new_document.add_object(Stream::new(Dictionary::new(), b"q\n".to_vec()));
    for mut update in updates {
        for (&index, &id) in &refs {
            update.fonts.set(index.resource_name(&names).as_bytes(), Object::Reference(id));
        }
        update.resources.set("Font", update.fonts);
        let mut dict = Dictionary::new();
        dict.set("Filter", Object::Name(b"FlateDecode".to_vec()));
        let suffix = updated.new_document.add_object(Stream::new(dict, update.stream));
        update.contents.insert(0, Object::Reference(prefix));
        update.contents.push(Object::Reference(suffix));
        let page_id = update.page_id;
        updated.opt_clone_object_to_new_document(page_id).map_err(|e| e.to_string())?;
        let page = updated.new_document.get_object_mut(page_id).and_then(Object::as_dict_mut).map_err(|e| e.to_string())?;
        page.set("Resources", update.resources);
        page.set("Contents", Object::Array(update.contents));
    }
    let mut output = Vec::new();
    updated.save_to(&mut output).map_err(|e| format!("Could not save OCR layer: {e}"))?;
    Ok(output)
}

// Transfer only the new font objects from the shared PDF writer into the incremental update.
fn append_fonts(doc: &mut Document, fonts: &Fonts<'_>) -> Result<BTreeMap<FontId, ObjectId>, String> {
    let mut temporary = Pdf::new();
    temporary.catalog(Ref::new(1)).pages(Ref::new(2));
    let refs = fonts.embed(&mut temporary, &mut Ref::new(4))?;
    temporary.pages(Ref::new(2)).kids([Ref::new(3)]).count(1);
    let mut page = temporary.page(Ref::new(3));
    page.parent(Ref::new(2));
    let mut resources = page.resources();
    let mut font_dict = resources.fonts();
    for (index, id) in refs.values().enumerate() {
        font_dict.pair(Name(format!("F{index}").as_bytes()), *id);
    }
    drop(font_dict);
    drop(resources);
    drop(page);
    let mut objects = Document::load_mem(&temporary.finish()).map_err(|e| format!("Could not read PDF font objects: {e}"))?;
    let offset = doc.max_id;
    objects.renumber_objects_with(offset + 1);
    doc.max_id = objects.max_id;
    doc.objects.extend(objects.objects.split_off(&(offset + 4, 0)));
    Ok(refs.into_iter().map(|(index, id)| (index, (offset + id.get() as u32, 0))).collect())
}

pub(crate) fn write_words(
    content: &mut Content,
    layer: &OcrPageLayer,
    fonts: &mut Fonts<'_>,
    names: &BTreeMap<usize, String>,
) -> Result<(), String> {
    content.begin_text().set_text_rendering_mode(TextRenderingMode::Invisible);
    for word in &layer.words {
        if word.text.trim().is_empty() {
            continue;
        }
        if word.w_pt <= 0.
            || word.h_pt <= 0.
            || ![word.x_pt, word.y_pt, word.w_pt, word.h_pt, word.baseline_y_pt.unwrap_or(0.)]
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 1e12)
        {
            return Err("Invalid OCR word box".into());
        }
        let spec = FontSpec::from_family("Liberation Sans", word.h_pt, false, false);
        let shaped = fonts.shape(&word.text, &spec, Some(word.rtl), false, 0.)?;
        if shaped.width <= 0. {
            continue;
        }
        let scale_y = word.h_pt / (shaped.ascent + shaped.descent).max(f64::EPSILON);
        let baseline = word.baseline_y_pt.unwrap_or(word.y_pt + shaped.ascent * scale_y);
        shaped.write(content, names, [word.x_pt, baseline], [word.w_pt / shaped.width, scale_y]);
    }
    content.end_text();
    Ok(())
}
