//! The contract the format layers (Word, PowerPoint, Excel) implement.
//!
//! The core reads what the formats share. A layer adds what only its format has: it reads its
//! own parts from the document's package, keeps typed data in [`Document::extensions`], can turn
//! the core's raw nodes into its own data, and writes all of it back. The core never depends on
//! a layer.

use crate::model::{Document, Kind};
use crate::opc::Package;
use crate::Result;

pub trait Layer: Send + Sync {
    /// For error messages: "Word", "PowerPoint".
    fn name(&self) -> &'static str;

    /// The kinds of document this layer applies to.
    fn kinds(&self) -> &[Kind];

    /// Runs after the core has read a document: parse the layer's parts, resolve inheritance
    /// (styles, placeholders), put its data in the document's extensions.
    fn after_read(&self, _doc: &mut Document) -> Result<()> {
        Ok(())
    }

    /// Runs on a copy of the document before the core writes it as `to`: fold the layer's data
    /// back into the model (raw nodes, origins) where the core will write it.
    fn before_write(&self, _doc: &mut Document, _to: Kind) -> Result<()> {
        Ok(())
    }

    /// Whether [`Layer::before_write`] may change a document written as `to`. A layer that
    /// keeps the default no-op says `false`: its `before_write` isn't called, and when no layer
    /// edits, the core writes the document itself rather than a copy of it.
    fn edits_before_write(&self, _to: Kind) -> bool {
        true
    }

    /// Runs after the core has written the package for a document of kind `to`: add or update
    /// the layer's own parts (styles, numbering, notes slides).
    fn write_parts(&self, _doc: &Document, _package: &mut Package, _to: Kind) -> Result<()> {
        Ok(())
    }
}
