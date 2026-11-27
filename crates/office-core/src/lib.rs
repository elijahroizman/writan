//! The shared core of the office compiler.
//!
//! Front ends read Word (.docx), PowerPoint (.pptx), Excel (.xlsx) and CSV into one model,
//! layers add what only one format has, and back ends write the model out again. The core works
//! on bytes only: no file system and no platform code, so it also builds for WebAssembly.
//!
//! ```
//! let csv = b"name,qty\r\nApple,3\r\n";
//! let doc = office_core::read(csv, Some("fruit.csv")).unwrap();
//! assert_eq!(doc.kind, office_core::Kind::Csv);
//! assert_eq!(office_core::write(&doc, office_core::Kind::Csv).unwrap(), csv);
//! ```
//!
//! See `docs/internals/office-compiler.md` for the design.

pub mod convert;
pub mod drawing;
mod error;
pub mod formats;
pub mod layer;
pub mod model;
pub mod opc;
pub mod units;
pub mod xml;

pub use error::{Error, Result};
pub use layer::Layer;
pub use model::{Document, Kind};

use std::borrow::Cow;
use std::sync::Arc;

use formats::Output;
use opc::Package;

/// Reads and writes documents, running the layers it has been given.
#[derive(Default)]
pub struct Compiler {
    layers: Vec<Box<dyn Layer>>,
}

impl Compiler {
    pub fn new() -> Compiler {
        Compiler::default()
    }

    pub fn with_layer(mut self, layer: impl Layer + 'static) -> Compiler {
        self.layers.push(Box::new(layer));
        self
    }

    /// Reads a document. `name` is the file's name, if known; it helps tell CSV from other text
    /// and names a CSV file's sheet.
    pub fn read(&self, bytes: &[u8], name: Option<&str>) -> Result<Document> {
        // A zip is read once, both to tell its kind and for the front end.
        let doc = if is_zip(bytes) {
            let pkg = Package::read(bytes)?;
            let kind = package_kind(&pkg)?;
            formats::read_package(pkg, kind)?
        } else {
            formats::read(bytes, detect(bytes, name)?, name)?
        };
        self.after_read(doc)
    }

    /// Reads a document whose bytes are already shared, without copying the zip again.
    pub fn read_arc(&self, bytes: Arc<[u8]>, name: Option<&str>) -> Result<Document> {
        let doc = if is_zip(&bytes) {
            let pkg = Package::read_arc(bytes)?;
            let kind = package_kind(&pkg)?;
            formats::read_package(pkg, kind)?
        } else {
            formats::read(&bytes, detect(&bytes, name)?, name)?
        };
        self.after_read(doc)
    }

    fn after_read(&self, mut doc: Document) -> Result<Document> {
        for layer in self.layers_for(doc.kind) {
            layer.after_read(&mut doc).map_err(|e| layer_error(layer, e))?;
        }
        Ok(doc)
    }

    /// Writes a document as `kind`: its own kind to save it, another to convert it (see
    /// [`convert::to_kind`]).
    pub fn write(&self, doc: &Document, kind: Kind) -> Result<Vec<u8>> {
        let mut doc = Cow::Borrowed(doc);
        if !convert::fits(&doc, kind) || (doc.kind != kind && doc.package.is_some()) {
            doc = Cow::Owned(convert::to_kind(&doc, kind)?);
        }
        // Layers edit a copy, made only when one does: copying a big document is a good share
        // of saving it.
        for layer in self.layers_for(kind).filter(|l| l.edits_before_write(kind)) {
            layer.before_write(doc.to_mut(), kind).map_err(|e| layer_error(layer, e))?;
        }
        let doc = &*doc;
        match formats::write(doc, kind)? {
            Output::Bytes(bytes) => Ok(bytes),
            Output::Package(mut pkg) => {
                for layer in self.layers_for(kind) {
                    layer.write_parts(doc, &mut pkg, kind).map_err(|e| layer_error(layer, e))?;
                }
                pkg.to_bytes()
            }
        }
    }

    fn layers_for(&self, kind: Kind) -> impl Iterator<Item = &dyn Layer> {
        self.layers.iter().map(|l| l.as_ref()).filter(move |l| l.kinds().contains(&kind))
    }
}

fn layer_error(layer: &dyn Layer, e: Error) -> Error {
    match e {
        Error::Layer { .. } => e,
        e => Error::Layer { layer: layer.name(), message: e.to_string() },
    }
}

/// Reads a document with no layers.
pub fn read(bytes: &[u8], name: Option<&str>) -> Result<Document> {
    Compiler::new().read(bytes, name)
}

/// Writes a document with no layers.
pub fn write(doc: &Document, kind: Kind) -> Result<Vec<u8>> {
    Compiler::new().write(doc, kind)
}

/// What kind of document `bytes` is. A zip is told apart by its main part's content type; other
/// bytes are CSV if the name says so, or if there's no name and they look like text.
pub fn detect(bytes: &[u8], name: Option<&str>) -> Result<Kind> {
    if is_zip(bytes) {
        return package_kind(&Package::read(bytes)?);
    }
    if bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]) {
        // Office saves a password-protected .docx, .pptx or .xlsx as a compound file too, holding
        // the encrypted zip in a stream named EncryptedPackage.
        if is_encrypted_package(bytes) {
            return Err(Error::Invalid(
                "this file is protected with a password, which isn't supported: remove the password in Office and open it again".into(),
            ));
        }
        return Err(Error::Invalid("this is an old binary Office file (.doc, .ppt or .xls), which isn't supported".into()));
    }
    let ext = name.and_then(|n| n.rsplit_once('.')).map(|(_, e)| e);
    match ext.and_then(Kind::from_extension) {
        Some(Kind::Csv) => Ok(Kind::Csv),
        Some(_) => Err(Error::Zip("the file isn't a zip, so it's damaged or not really an Office file".into())),
        None if name.is_none() && looks_like_text(bytes) => Ok(Kind::Csv),
        None => Err(Error::UnknownFormat),
    }
}

fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
}

/// What kind of document a package is, by its main part's content type.
fn package_kind(pkg: &Package) -> Result<Kind> {
    let main = pkg.main_part()?;
    pkg.content_type(&main).and_then(Kind::from_main_content_type).ok_or(Error::UnknownFormat)
}

/// Whether a compound file has the stream an encrypted Office package is kept in: its name in
/// the file's directory, which is UTF-16.
fn is_encrypted_package(bytes: &[u8]) -> bool {
    let name: Vec<u8> = "EncryptedPackage".encode_utf16().flat_map(u16::to_le_bytes).collect();
    bytes.windows(name.len()).any(|w| w == name)
}

fn looks_like_text(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(4096)];
    sample.starts_with(&[0xFF, 0xFE]) || sample.starts_with(&[0xFE, 0xFF]) || !sample.contains(&0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opc::READS;

    fn reads_while(f: impl FnOnce()) -> usize {
        READS.with(|n| n.set(0));
        f();
        READS.with(|n| n.get())
    }

    #[test]
    fn opening_reads_the_zip_once_and_tells_its_kind() {
        for kind in [Kind::Word, Kind::Slides, Kind::Sheets] {
            let bytes = write(&Document::new(kind), kind).unwrap();
            for name in [None, Some("file.bin"), Some(if kind == Kind::Word { "x.xlsx" } else { "x.docx" })] {
                let mut doc = None;
                assert_eq!(reads_while(|| doc = Some(read(&bytes, name).unwrap())), 1, "{kind:?} named {name:?}");
                let doc = doc.unwrap();
                // The zip's content decides, not its name, as before.
                assert_eq!(doc.kind, kind);
                assert_eq!(detect(&bytes, name).unwrap(), kind);
                assert_eq!(doc.body, formats::read(&bytes, kind, name).unwrap().body);
            }
        }
        // Text isn't a zip, and is told by its name or, with none, by looking like text.
        for (bytes, name) in [(&b"a,b\n1,2\n"[..], Some("list.csv")), (b"a\tb\n", Some("x.tsv")), (b"a,b\n", None)] {
            let mut doc = None;
            assert_eq!(reads_while(|| doc = Some(read(bytes, name).unwrap())), 0);
            assert_eq!(doc.unwrap().kind, Kind::Csv);
        }
        assert!(matches!(read(b"plain", Some("report.docx")), Err(Error::Zip(_))));
        assert!(matches!(read(b"\x00\x01binary", None), Err(Error::UnknownFormat)));
        // A zip that isn't an Office file.
        let mut pkg = Package::new();
        pkg.put("hello.txt", b"hi".to_vec(), "text/plain");
        let zip = pkg.to_bytes().unwrap();
        assert!(read(&zip, Some("hello.zip")).is_err());
        assert_eq!(detect(&zip, None).map_err(|e| e.to_string()).err(), read(&zip, None).map_err(|e| e.to_string()).err());
    }
}
