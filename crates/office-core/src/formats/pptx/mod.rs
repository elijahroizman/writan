//! PowerPoint (.pptx): the presentation's slides and what's on them. Masters, layouts, notes,
//! transitions and animations stay in the package (and the slides' origins) for the PowerPoint
//! layer.

mod read;
mod write;

pub use read::{diagram_frames, read, read_part_frames};
pub use write::write;

use crate::xml::ns;

/// What `p:spTree` holds besides shapes.
fn is_tree_props(el: &crate::xml::Element) -> bool {
    el.is(ns::P, "nvGrpSpPr") || el.is(ns::P, "grpSpPr") || el.is(ns::P, "extLst")
}
