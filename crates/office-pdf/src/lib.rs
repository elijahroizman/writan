mod color;
mod fonts;
mod image_pdf;
pub mod model;
mod ocr_layer;
mod svg_path;
mod writer;

pub use image_pdf::images_to_pdf;
pub use model::*;
pub use ocr_layer::{inject_ocr_text_layer, inspect_pdf};
pub use writer::render_document_to_pdf;

use std::collections::HashMap;
use std::io::Write;

pub fn unpack_blobs(mut buf: &[u8]) -> Result<HashMap<String, Vec<u8>>, String> {
    fn read_len(buf: &mut &[u8]) -> Result<usize, String> {
        let (len, rest) = buf.split_at_checked(4).ok_or("Truncated blob pack length")?;
        *buf = rest;
        Ok(u32::from_le_bytes(len.try_into().unwrap()) as usize)
    }
    fn read_bytes<'a>(buf: &mut &'a [u8]) -> Result<&'a [u8], String> {
        let len = read_len(buf)?;
        let (bytes, rest) = buf.split_at_checked(len).ok_or("Truncated blob pack entry")?;
        *buf = rest;
        Ok(bytes)
    }
    let count = read_len(&mut buf)?;
    if count > buf.len() / 8 {
        return Err("Truncated blob pack entries".into());
    }
    let mut blobs = HashMap::new();
    for _ in 0..count {
        let key = std::str::from_utf8(read_bytes(&mut buf)?).map_err(|_| "Invalid UTF-8 blob key")?;
        let bytes = read_bytes(&mut buf)?;
        if blobs.insert(key.to_owned(), bytes.to_vec()).is_some() {
            return Err(format!("Duplicate blob key {key}"));
        }
    }
    if !buf.is_empty() {
        return Err("Trailing data in blob pack".into());
    }
    Ok(blobs)
}

fn flate(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).map_err(|e| e.to_string())?;
    encoder.finish().map_err(|e| e.to_string())
}
