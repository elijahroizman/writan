//! The document engine as the app uses it: WebAssembly classes for each kind of document, which
//! open bytes, draw pages as display lists (JSON), take editing commands and save.
//!
//! Text is measured by the app (`Host`), with the same fonts the page draws in, so what the
//! engine lays out is exactly what shows.

use office_engine::measure::{Font, Measurer, Metrics};
use wasm_bindgen::prelude::*;

mod convert;
mod ocr_docx;
mod sheets;
mod slides;
mod word;

pub use convert::{convert_document, to_markdown};
pub use sheets::SheetsDoc;
pub use slides::SlidesDoc;
pub use word::WordDoc;

/// Reconstructs an editable Word document from structured OCR results.
#[wasm_bindgen(js_name = ocrToDocx)]
pub fn ocr_to_docx(json: &str) -> Result<Vec<u8>, JsError> {
    ocr_docx::ocr_document_to_docx(json).map_err(err)
}

#[wasm_bindgen]
extern "C" {
    /// The app's side: measures text with a canvas.
    pub type Host;

    /// The advance width of `text` in the CSS font `font`.
    #[wasm_bindgen(method)]
    fn width(this: &Host, font: &str, text: &str) -> f64;

    /// The font's ascent, descent and line gap, as `[ascent, descent, gap]`.
    #[wasm_bindgen(method)]
    fn metrics(this: &Host, font: &str) -> Vec<f64>;
}

/// The host as the engine's measurer.
pub(crate) struct JsMeasurer<'a>(pub &'a Host);

impl Measurer for JsMeasurer<'_> {
    fn width(&mut self, font: &Font, text: &str) -> f64 {
        self.0.width(&font.css(), text)
    }

    fn metrics(&mut self, font: &Font) -> Metrics {
        let m = self.0.metrics(&font.css());
        Metrics {
            ascent: m.first().copied().unwrap_or(font.px() * 0.8),
            descent: m.get(1).copied().unwrap_or(font.px() * 0.2),
            gap: m.get(2).copied().unwrap_or(0.0),
        }
    }
}

pub(crate) fn json<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "null".into())
}

pub(crate) fn err(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

/// What kind of document some bytes are: `word`, `slides`, `sheets` or `csv`.
#[wasm_bindgen]
pub fn detect(bytes: &[u8], name: &str) -> Option<String> {
    office_core::detect(bytes, Some(name)).ok().map(|k| match k {
        office_core::Kind::Word => "word".into(),
        office_core::Kind::Slides => "slides".into(),
        office_core::Kind::Sheets => "sheets".into(),
        office_core::Kind::Csv => "csv".into(),
    })
}
