//! Legacy binary PowerPoint (`.ppt`) conversion. This uses `office_oxide`, whose readers and
//! writers are much larger than the editing engine's own code, so it is a separate WebAssembly
//! module loaded only when a `.ppt` file is opened or converted.

use wasm_bindgen::prelude::*;

/// Converts a legacy binary PowerPoint file (`.ppt`, a compound document) to `.pptx` bytes.
#[wasm_bindgen(js_name = convertLegacyPpt)]
pub fn convert_legacy_ppt(bytes: &[u8]) -> Result<Vec<u8>, JsError> {
    convert_inner(bytes).map_err(|e| JsError::new(&e))
}

fn convert_inner(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let doc = office_oxide::Document::from_reader(std::io::Cursor::new(bytes.to_vec()), office_oxide::DocumentFormat::Ppt)
        .map_err(|e| format!("couldn't read this PowerPoint file: {e}"))?;
    if doc.format() != office_oxide::DocumentFormat::Ppt {
        return Err("this isn't a PowerPoint file".into());
    }
    let ir = doc.to_ir();
    let mut out = std::io::Cursor::new(Vec::new());
    office_oxide::create::create_from_ir_to_writer(&ir, office_oxide::DocumentFormat::Pptx, &mut out)
        .map_err(|e| format!("couldn't convert this PowerPoint file to .pptx: {e}"))?;
    Ok(out.into_inner())
}
