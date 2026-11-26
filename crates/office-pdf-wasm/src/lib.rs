use office_pdf::{unpack_blob_slices, ImagePdfPageInput, OcrPageLayer, PdfDocumentInput};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(js_name = renderDocumentToPdf)]
pub fn render_document_to_pdf(doc_json: &str, media_pack: &[u8], font_pack: &[u8]) -> Result<Vec<u8>, JsError> {
    let doc: PdfDocumentInput = serde_json::from_str(doc_json)?;
    let media = unpack_blob_slices(media_pack).map_err(|e| JsError::new(&e))?;
    let fonts = unpack_blob_slices(font_pack).map_err(|e| JsError::new(&e))?;
    office_pdf::render_document_to_pdf_slices(&doc, &media, &fonts).map_err(|e| JsError::new(&e))
}

#[wasm_bindgen(js_name = imagesToPdf)]
pub fn images_to_pdf(pages_json: &str, media_pack: &[u8], font_pack: &[u8]) -> Result<Vec<u8>, JsError> {
    let pages: Vec<ImagePdfPageInput> = serde_json::from_str(pages_json)?;
    let media = unpack_blob_slices(media_pack).map_err(|e| JsError::new(&e))?;
    let fonts = unpack_blob_slices(font_pack).map_err(|e| JsError::new(&e))?;
    office_pdf::images_to_pdf_slices(&pages, &media, &fonts).map_err(|e| JsError::new(&e))
}

#[wasm_bindgen(js_name = inspectPdf)]
pub fn inspect_pdf_wasm(pdf_bytes: &[u8]) -> Result<String, JsError> {
    let info = office_pdf::inspect_pdf(pdf_bytes).map_err(|e| JsError::new(&e))?;
    Ok(serde_json::to_string(&info)?)
}

#[wasm_bindgen(js_name = injectOcrTextLayer)]
pub fn inject_ocr_text_layer_wasm(
    pdf_bytes: &[u8],
    layers_json: &str,
    font_pack: &[u8],
    allow_signed: Option<bool>,
) -> Result<Vec<u8>, JsError> {
    let layers: Vec<OcrPageLayer> = serde_json::from_str(layers_json)?;
    let fonts = unpack_blob_slices(font_pack).map_err(|e| JsError::new(&e))?;
    office_pdf::inject_ocr_text_layer_slices(pdf_bytes, &layers, &fonts, allow_signed.unwrap_or(false)).map_err(|e| JsError::new(&e))
}
