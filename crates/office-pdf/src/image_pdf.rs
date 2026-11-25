use std::collections::HashMap;
use std::io::Cursor;

use jpeg_decoder::{Decoder, PixelFormat};
use pdf_writer::{Filter, Name, Pdf, Ref};
use png::{ColorType, Transformations};

use crate::flate;
use crate::model::{ImagePdfPageInput, PdfMetadata};
use crate::ocr_layer::write_words;
use crate::svg_path::Matrix;
use crate::writer::{Engine, PageDrawing};

pub fn images_to_pdf(
    pages: &[ImagePdfPageInput],
    media: &HashMap<String, Vec<u8>>,
    fonts: &HashMap<String, Vec<u8>>,
) -> Result<Vec<u8>, String> {
    let mut engine = Engine::new(media, fonts);
    for page in pages {
        let mut drawing = PageDrawing::new(page.width_pt, page.height_pt)?;
        let bytes = media.get(&page.media_key).ok_or_else(|| format!("Missing image: {}", page.media_key))?;
        let mime = page.mime.split(';').next().unwrap_or("").trim();
        let valid_mime = match mime {
            "image/jpeg" | "image/jpg" => bytes.starts_with(&[0xff, 0xd8]),
            "image/png" => is_png(bytes),
            _ => false,
        };
        if !valid_mime {
            return Err(format!("Unsupported or mismatched image MIME type: {mime}"));
        }
        Matrix([1., 0., 0., -1., 0., page.height_pt]).apply(&mut drawing.content);
        engine.draw_image(&mut drawing, &page.media_key, [0., 0., page.width_pt, page.height_pt], None)?;
        if let Some(layer) = &page.ocr_layer {
            let transform = crate::ocr_layer::layer_scale(layer, page.width_pt, page.height_pt)?;
            transform.apply(&mut drawing.content);
            write_words(&mut drawing.content, layer, &mut engine.fonts, &engine.font_names)?;
        }
        engine.add_page(drawing)?;
    }
    engine.finish(&PdfMetadata::default())
}

fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
}

pub(crate) fn embed(pdf: &mut Pdf, next: &mut Ref, bytes: &[u8]) -> Result<(Ref, u8), String> {
    if is_png(bytes) {
        Ok((embed_png(pdf, next, bytes)?, 1))
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        Ok((embed_jpeg(pdf, next, bytes)?, jpeg_exif_orientation(bytes)))
    } else {
        Err("PDF image must be PNG or JPEG".into())
    }
}

fn jpeg_exif_orientation(mut bytes: &[u8]) -> u8 {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return 1;
    }
    bytes = &bytes[2..];
    while let Some(rest) = bytes.strip_prefix(&[0xff]) {
        bytes = rest;
        while let Some(rest) = bytes.strip_prefix(&[0xff]) {
            bytes = rest;
        }
        let Some((&marker, tail)) = bytes.split_first() else {
            break;
        };
        if marker == 0xda || marker == 0xd9 {
            break;
        }
        let Some((len_bytes, _)) = tail.split_at_checked(2) else {
            break;
        };
        let len = u16::from_be_bytes([len_bytes[0], len_bytes[1]]) as usize;
        let Some((segment, remaining)) = tail.split_at_checked(len) else {
            break;
        };
        if marker == 0xe1 && segment.get(2..8) == Some(b"Exif\0\0") {
            let tiff = &segment[8..];
            if tiff.len() >= 8 {
                let le = match &tiff[0..2] {
                    b"II" => true,
                    b"MM" => false,
                    _ => return 1,
                };
                let r16 = |off: usize| -> Option<u16> {
                    let end = off.checked_add(2)?;
                    let b = tiff.get(off..end)?;
                    Some(if le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) })
                };
                let r32 = |off: usize| -> Option<u32> {
                    let end = off.checked_add(4)?;
                    let b = tiff.get(off..end)?;
                    Some(if le { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { u32::from_be_bytes([b[0], b[1], b[2], b[3]]) })
                };
                if r16(2) == Some(42) {
                    if let Some(ifd) = r32(4).and_then(|v| usize::try_from(v).ok()) {
                        if let Some(count) = r16(ifd).map(|c| c.min(128) as usize) {
                            let entries_start = ifd.checked_add(2);
                            if let Some(base) = entries_start {
                                for i in 0..count {
                                    let Some(entry) = i.checked_mul(12).and_then(|delta| base.checked_add(delta)) else {
                                        break;
                                    };
                                    let (Some(tag), Some(ty), Some(cnt)) =
                                        (r16(entry), entry.checked_add(2).and_then(&r16), entry.checked_add(4).and_then(&r32))
                                    else {
                                        break;
                                    };
                                    if tag == 0x0112 && ty == 3 && cnt == 1 {
                                        if let Some(val) = entry.checked_add(8).and_then(&r16) {
                                            if (1..=8).contains(&val) {
                                                return val as u8;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if len < 2 {
            break;
        }
        bytes = remaining;
    }
    1
}

fn embed_jpeg(pdf: &mut Pdf, next: &mut Ref, bytes: &[u8]) -> Result<Ref, String> {
    let mut decoder = Decoder::new(Cursor::new(bytes));
    decoder.read_info().map_err(|e| format!("Invalid JPEG: {e}"))?;
    let info = decoder.info().ok_or("Missing JPEG dimensions")?;
    let color_space = match info.pixel_format {
        PixelFormat::L8 => Name(b"DeviceGray"),
        PixelFormat::RGB24 => Name(b"DeviceRGB"),
        PixelFormat::CMYK32 => Name(b"DeviceCMYK"),
        PixelFormat::L16 => return Err("PDF JPEG requires 8-bit components".into()),
    };
    let id = next.bump();
    let mut image = pdf.image_xobject(id, bytes);
    image.filter(Filter::DctDecode);
    image.width(info.width.into()).height(info.height.into()).bits_per_component(8).color_space_name(color_space);
    if matches!(info.pixel_format, PixelFormat::CMYK32) && adobe_jpeg(bytes) {
        image.decode([1., 0., 1., 0., 1., 0., 1., 0.]);
    }
    Ok(id)
}

fn adobe_jpeg(mut bytes: &[u8]) -> bool {
    bytes = &bytes[2..];
    while let Some(rest) = bytes.strip_prefix(&[0xff]) {
        bytes = rest;
        while let Some(rest) = bytes.strip_prefix(&[0xff]) {
            bytes = rest;
        }
        let Some((&marker, tail)) = bytes.split_first() else {
            break;
        };
        if marker == 0xda || marker == 0xd9 {
            break;
        }
        let Some((len, _)) = tail.split_at_checked(2) else {
            break;
        };
        let len = u16::from_be_bytes([len[0], len[1]]) as usize;
        let Some((segment, remaining)) = tail.split_at_checked(len) else {
            break;
        };
        if marker == 0xee && segment.get(2..7) == Some(b"Adobe") {
            return true;
        }
        if len < 2 {
            break;
        }
        bytes = remaining;
    }
    false
}

fn embed_png(pdf: &mut Pdf, next: &mut Ref, bytes: &[u8]) -> Result<Ref, String> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(|e| format!("Invalid PNG: {e}"))?;
    let len = reader.output_buffer_size().filter(|n| *n <= 128 * 1024 * 1024).ok_or("PNG is too large for PDF export")?;
    let mut pixels = vec![0; len];
    let info = reader.next_frame(&mut pixels).map_err(|e| format!("Invalid PNG: {e}"))?;
    let (width, height) = (info.width as i32, info.height as i32);
    let (bits, sample_size) = if info.bit_depth == png::BitDepth::Sixteen { (16, 2) } else { (8, 1) };
    let (color_space, channels, alpha) = match info.color_type {
        ColorType::Rgb => (Name(b"DeviceRGB"), 3, false),
        ColorType::Rgba => (Name(b"DeviceRGB"), 3, true),
        ColorType::Grayscale => (Name(b"DeviceGray"), 1, false),
        ColorType::GrayscaleAlpha => (Name(b"DeviceGray"), 1, true),
        ColorType::Indexed => return Err("Could not expand PNG palette".into()),
    };
    let pixels = &pixels[..info.buffer_size()];
    let mut mask = None;
    let color_data = if alpha {
        let pixel_size = (channels + 1) * sample_size;
        let mut colors = Vec::with_capacity(pixels.len() / pixel_size * channels * sample_size);
        let mut alphas = Vec::with_capacity(pixels.len() / pixel_size * sample_size);
        for pixel in pixels.chunks_exact(pixel_size) {
            let (color, alpha) = pixel.split_at(channels * sample_size);
            colors.extend_from_slice(color);
            alphas.extend_from_slice(alpha);
        }
        if alphas.iter().any(|&byte| byte != 255) {
            let compressed = flate(&alphas)?;
            let id = next.bump();
            let mut image = pdf.image_xobject(id, &compressed);
            image.filter(Filter::FlateDecode);
            image.width(width).height(height).bits_per_component(bits).color_space().device_gray();
            mask = Some(id);
        }
        flate(&colors)?
    } else {
        flate(pixels)?
    };
    let id = next.bump();
    let mut image = pdf.image_xobject(id, &color_data);
    image.filter(Filter::FlateDecode);
    image.width(width).height(height).bits_per_component(bits).color_space_name(color_space);
    if let Some(mask) = mask {
        image.s_mask(mask);
    }
    Ok(id)
}
