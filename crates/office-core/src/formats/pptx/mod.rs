//! PowerPoint (.pptx): the presentation's slides and what's on them. Masters, layouts, notes,
//! transitions and animations stay in the package (and the slides' origins) for the PowerPoint
//! layer.

mod read;
mod write;

pub use read::{diagram_frames, read, read_part_frames};
pub use write::write;

use crate::opc::{rels::kind, Package};
use crate::xml::{ns, Element};

/// The chart reference in an ordinary DrawingML graphic frame.
pub fn chart_element(frame: &Element) -> Option<&Element> {
    frame.is(ns::P, "graphicFrame").then_some(())?;
    frame.path(ns::A, &["graphic", "graphicData"])?.child(ns::C, "chart")
}

pub fn chart_element_mut(frame: &mut Element) -> Option<&mut Element> {
    frame.is(ns::P, "graphicFrame").then_some(())?;
    frame.child_mut(ns::A, "graphic")?.child_mut(ns::A, "graphicData")?.child_mut(ns::C, "chart")
}

/// Resolves a chart's slide-local id without following an external relationship.
pub fn chart_target(pkg: &Package, slide_part: &str, frame: &Element) -> Option<String> {
    let rid = chart_element(frame)?.attr_ns(ns::R, "id")?;
    let rels = pkg.rels(slide_part).ok()?;
    pkg.target(slide_part, rels.get(rid).filter(|rel| rel.is(kind::CHART))?).filter(|part| pkg.has(part))
}

/// What `p:spTree` holds besides shapes.
fn is_tree_props(el: &crate::xml::Element) -> bool {
    el.is(ns::P, "nvGrpSpPr") || el.is(ns::P, "grpSpPr") || el.is(ns::P, "extLst")
}
