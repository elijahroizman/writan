//! The shared document engine.
//!
//! The office compiler reads files into one model; this crate lays that model out, draws it and
//! edits it. What Word, PowerPoint and Excel share lives here: measuring and breaking text into
//! lines, tables, shapes and their geometry, positions in text, selections, editing steps with
//! undo, and the display list the app paints. Each layer crate adds what only its format has on
//! top: pages and sections for Word, slides and placeholders for PowerPoint, the grid for Excel.
//!
//! Text is measured by the host (see [`measure::Measurer`]), so what's measured is exactly what
//! the app's text engine draws, system fonts included. Lengths are CSS pixels at 96 per inch.
//!
//! See `docs/internals/office-engine.md` for the design.

pub mod collab;
pub mod display;
pub mod edit;
pub mod flow;
pub mod geometry;
pub mod history;
pub mod measure;
pub mod para;
pub mod pos;
pub mod style;
pub mod text;
