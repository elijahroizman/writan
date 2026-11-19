# The office compiler

Status: in use. The shared core (`crates/office-core`) reads and writes all four formats, the Word, PowerPoint and Excel layers sit on top of it, and the app's Word, PowerPoint and Excel editors run on it through the [office engine](office-engine.md), compiled to WebAssembly.

Before it, each Office format had its own JavaScript engine: docx-editor for Word, pptx-react-viewer and ooxml-ui for PowerPoint, and nothing for Excel. Each one parsed the file its own way, kept its own model of text and colors, and saved by patching the original zip. The office compiler replaced the parsing and saving under all of them with one Rust library that reads and writes Word (.docx), PowerPoint (.pptx), Excel (.xlsx) and CSV.

It works like a compiler: a **front end** for each format reads the file into one shared **model**, **passes** work on the model, and a **back end** writes it out again, in the same format or another one.

```
 .docx ─┐                                         ┌─ .docx
 .pptx ─┤  front ends   ┌──────────────┐  back    ├─ .pptx
 .xlsx ─┼─────────────▶ │ shared model │ ───────▶ ├─ .xlsx
 .csv  ─┘               └──────┬───────┘  ends    └─ .csv
                               │
                  layers: Word, PowerPoint, Excel
                  (passes over the model + their own parts)
```

## Goals

1. **One model for what the formats share.** A bold red run of Calibri is the same thing in a Word paragraph, a slide's text box and a spreadsheet's rich-text cell. A table is a table in Word and PowerPoint. A sheet is a sheet in Excel and CSV. Themes, colors, pictures, hyperlinks and document properties are shared too.
2. **Lossless round trips.** Opening and saving a file must not drop what the compiler doesn't understand. Untouched parts of the package are copied byte for byte; inside parts it rewrites, anything it doesn't model is kept as raw XML in its original place.
3. **Format-only features live in layers.** Word's styles, numbering, sections, comments and tracked changes; PowerPoint's masters, layouts, notes, transitions and animations; Excel's formulas, number formats, merged-cell rules and conditional formatting. Each layer is its own crate on top of the core, and the core never depends on a layer.
4. **No file system, no platform code.** The core takes bytes and returns bytes, so the same code runs in the Tauri app and, compiled to WebAssembly, in the browser version used for development.
5. **Cross-format conversion falls out.** Because every format meets in one model, a Word table can become a CSV, a CSV can become a workbook, and a sheet can become a table on a slide.

Not goals for the core: rendering, layout (line breaking, pagination) or any UI. Those are the [office engine](office-engine.md)'s, above it.

## What the old engines taught us

We read how the engines we used to ship handled these formats, and kept the ideas, not the code:

- **docx-editor** keeps a document model separate from its editing view and saves by updating the original package, which is why Word files survive a save. The compiler keeps the same split, and goes further by keeping unknown XML in place inside rewritten parts.
- **pptx-react-viewer** turns each slide into a flat list of typed elements (text, shape, picture, table, group) with a position and size. The shared model's `Frame` is that idea, and Word's floating pictures and Excel's drawings use it too.
- Both engines resolve theme colors and fonts on their own, slightly differently. The compiler does it once, in `drawing::theme` and `drawing::color`.
- Both treat the zip, `[Content_Types].xml` and relationships as plumbing they each rewrite. The compiler has one OPC package layer for all three formats.

## Crates

```
crates/
  Cargo.toml             workspace (separate from src-tauri, which keeps its own lock file and target/)
  office-core/           the shared compiler
  office-engine/         layout, drawing and editing shared by the three editors (see office-engine.md)
  office-word/           Word-only layer, and its view
  office-slides/         PowerPoint-only layer, and its view
  office-sheets/         Excel-only layer, and its view
  office-wasm/           the WebAssembly bindings the app loads
```

The app loads `office-wasm`, built into `src/engine/wasm` by `scripts/engine.mjs`. `src-tauri` doesn't depend on the crates: the editors run them in the web view.

## The core, module by module

| Module | What it does |
| --- | --- |
| `xml` | A small XML tree that keeps everything: element and attribute order, namespace declarations, comments, processing instructions, whitespace. Parsed with quick-xml, written back exactly. Namespace-aware lookups (`child(ns::W, "p")`). |
| `opc` | The Open Packaging Conventions package every Office file is: the zip, `[Content_Types].xml`, relationships (`_rels/*.rels`), part names and how relationship targets resolve, and the core and app properties (title, author, dates). Parts that are never touched are written back as the bytes they were read as. |
| `units` | EMUs, twips, points, half-points and percentages, with lossless conversions. Each format measures in its own unit; the model uses EMUs for lengths and hundredths of a point for font sizes. |
| `drawing` | DrawingML, the part of OOXML the three formats share: themes (color and font schemes), colors with their transforms (`lumMod`, `tint`, `shade`, `alpha`…) resolved to RGB, 2D transforms (offset, size, rotation, flip), and picture references. |
| `model` | The shared model: `Document`, rich text (`Paragraph`, `Run`, `RunProps`, `ParaProps`), `Table`, `Sheet` and `Cell`, `Frame` for positioned objects, `Slide`, media and hyperlinks. Every container has a `Raw` variant holding XML it doesn't model. |
| `formats` | A front end and back end per format: `docx`, `pptx`, `xlsx`, `csv`. Each reads what the shared model covers and keeps the rest raw. |
| `layer` | The contract layers implement: hooks that run after reading and before writing, and a typed slot on `Document` for a layer's own data. |
| `convert` | Shared passes between model shapes: table ↔ sheet (spans ↔ merged ranges), a document's body into another kind's (pages to slides, slides to pages, tables to sheets, sheets to tables), and any document to plain text. `Compiler::write` runs it when asked for a kind the body isn't. |
| `officec` | A command line (`--features cli`): `inspect` prints the model as JSON, `text` the text, `convert` writes another kind, `save` reads and writes a file back and says which parts changed, `diff` says where two packages differ. |

### The model

```rust
pub struct Document {
    pub kind: Kind,                 // Word, Slides, Sheets or Csv
    pub properties: Properties,     // title, author, created, modified…
    pub theme: Option<Theme>,
    pub media: Vec<Media>,          // pictures and other binary parts, by id
    pub body: Body,
    pub package: Option<Package>,   // the source package, for lossless writes
    pub extensions: Extensions,     // each format's and layer's own typed data
    pub origin: Origin,             // the main part's markup around the body
}

pub enum Body {
    Flow { blocks: Vec<Block> },    // Word: paragraphs and tables, top to bottom
    Slides { size: SlideSize, slides: Vec<Slide> },
    Workbook { sheets: Vec<Sheet> },// Excel, and CSV as a one-sheet workbook
}
```

Text is the same everywhere: a `Block` is a `Paragraph`, a `Table` or `Raw`; a paragraph holds `Inline`s (runs of text, page and column breaks, pictures, or `Raw`). A run's text writes tabs, line breaks and special hyphens as characters, whichever markup the format uses for them, and a hyperlink is a property of the runs it covers (Word wraps runs in `w:hyperlink`, PowerPoint puts `a:hlinkClick` in each run). Run formatting covers what all three formats can say directly: font, size, bold, italic, underline, strike, color, highlight, superscript and subscript, caps. A table has a cell for every grid slot, covered ones included, so spans mean the same whichever format a table came from. A slide is a list of `Frame`s. A frame is a shape with an optional text body, a picture, a table, a group of frames, or `Raw`. A sheet is sparse: cells keyed by row and column, each with a value (number, text, rich text, boolean, error) and, kept as-is for the Excel layer, its formula and style index.

What the model doesn't cover stays rather than being dropped. Whole objects it can't read (a field, a chart, a content control) are `Raw` nodes. Every node also has an `Origin`: the element it was read from, minus the parts the model reads. A paragraph's origin keeps its unknown properties, a shape's keeps its effects and style. Origins never take part in equality or JSON.

### Lossless writing

A document opened from a file keeps its `Package`. Saving it in the same format:

1. Copies every part the compiler didn't read, still compressed.
2. Rebuilds each part it did read from the model and the nodes' origins, then compares the result with the part as it is (ignoring layout whitespace, namespace prefixes and attribute order). A part that says the same thing keeps its original bytes, so saving an unchanged file changes nothing.
3. Inside a rebuilt part, a run nobody touched is put back exactly as it was read, and properties are patched one field at a time in schema order, so formatting the model doesn't read stays where it was.
4. Keeps content types and relationships in step with added and removed parts: a new picture gets a media part, a content type and a relationship; a removed slide takes its notes with it; Excel's calculation chain is dropped when cells change, and Excel rebuilds it.

Saving a document that was made from scratch, or converted from another format, writes a fresh package with what each application insists on: for Word an A4 page, a theme and default styles (Calibri 11, 8 pt after paragraphs); for PowerPoint a master with a blank layout, a theme and the presentation's property parts; for Excel the minimal style sheet. Origins from another format are ignored, and raw markup from another format is left out.

Every model node is reachable as JSON (serde), with raw XML as its source text, which is what a WebAssembly or IPC binding will hand the editors.

### Layers

A layer is a crate that implements `office_core::Layer`:

```rust
pub trait Layer: Send + Sync {
    fn name(&self) -> &'static str;
    fn kinds(&self) -> &[Kind];                                                  // which documents it applies to
    fn after_read(&self, doc: &mut Document) -> Result<()>;                      // parse its parts, resolve inheritance
    fn before_write(&self, doc: &mut Document, to: Kind) -> Result<()>;          // fold its data back into the model
    fn write_parts(&self, doc: &Document, pkg: &mut Package, to: Kind) -> Result<()>; // add or update its own parts
}
```

`Compiler::new().with_layer(…)` runs a layer's hooks for the kinds it names. `before_write` works on a copy, so writing never changes the document. A layer reads the parts it owns from `doc.package` and keeps its typed data in `doc.extensions`. It can also turn the core's `Raw` nodes into its own typed data and back again. The layers:

- **Word** (`office-word`, built): styles resolved the way Word does (document defaults, the paragraph style's chain, the character style, toggled bold and italic, direct formatting, theme fonts), and Word's built-in styles added when a document uses one it doesn't define; lists and their labels counted through the document, and new bulleted and numbered lists; sections' page size, orientation, margins and columns; headers, footers, footnotes, endnotes and comments read and written as the core's blocks, with comments added and removed; tracked changes listed, accepted and rejected (insertions, deletions, moves, formatting changes, paragraph marks, table rows). Not yet: fields, bookmarks and floating drawings, which stay as raw markup.
- **PowerPoint** (`office-slides`, built): slide masters and layouts read, and placeholder frames resolved the way PowerPoint shows them (position from the layout or master, text formatting per outline level from the master's title, body or other style, then the master's and layout's placeholders, then the frame's own list style; theme fonts named); theme colors mapped through the master's color map and the layout's and slide's overrides; slides added on a layout with its placeholders; speaker notes read, changed and added (making the notes master if the deck has none); transitions read (including PowerPoint 2010's, which come as alternatives), set and removed; which frames a slide animates, and removing animations. Not yet: editing animations, and slide comments, which stay as they are in the file.
- **Excel** (`office-sheets`, built): the style sheet (each cell's number format, font, fill, border, alignment and protection, read and added); number formats and dates shown as Excel shows them (sections, conditions, colors, scaling, scientific, fractions, dates, elapsed times; both date systems); formulas parsed, shared formulas expanded, and recalculated with nearly eighty of the common functions (maths, logic, text, lookups, dates, conditional sums); defined names; column widths and row heights. Conditional formatting (cell, formula, text, blank, error, duplicate, top/bottom and average rules, color scales, data bars and icon sets, from the main list and the 2010 extension list) and data validation (lists, input messages, and refusing values on entry) are worked out from the worksheet's markup; pictures are read from the drawing and shown. All three stay in the file as they were read.

## Milestones

All done for the shared core:

1. **Plan and crate skeleton.** This document, `crates/` workspace, `office-core` with its module layout.
2. **XML.** Lossless tree, parser, writer, namespace helpers.
3. **Packages.** OPC read and write, content types, relationships, document properties.
4. **Units and DrawingML.** Unit conversions, themes, colors and transforms, shape properties.
5. **The shared model, and CSV.** Model types, the layer contract, CSV in and out (delimiter and encoding detection).
6. **Word, PowerPoint and Excel front ends.** Reading each format's shared subset into the model.
7. **Back ends.** Writing each format, lossless for untouched content, and fresh packages for new or converted documents.
8. **Conversion and a command line.** Passes between kinds and `officec`.

Then the three layers, the shared engine and the WebAssembly bindings, and the Word, PowerPoint and Excel editors moved onto them (all done).

## Known gaps in the core

These are the core's own, not the layers':

- **Big workbooks** are read and written a few hundred kilobytes of rows at a time (`xml::stream`), cells keep an origin only when they have markup the writer couldn't make again, and saving skips sheets whose fingerprint shows nobody touched them. A million cells read in about 2 s and write in about 2 s in a release build; most of that is allocating and freeing each batch's XML tree, which interning element names would cut further.
- **Excel's phonetic guides** (`rPh`, for Japanese) survive on strings that don't change; a string whose text changes is written without them.
- **Word runs split by a page break** are written back as separate runs. Hyperlink edits preserve the original wrapper and its attributes; only new hyperlinks default to `w:history="1"`.
- **Characters XML can't hold** (control characters other than tab and line breaks) are dropped from Word and PowerPoint text. Excel keeps them with its `_xHHHH_` escape.
- **Converting between kinds** lays slides out simply: each page's paragraphs in text boxes, then each table and picture below, carried onto more slides when they don't fit (tables split by rows, repeating a header row; pictures shrink to fit). Heights are estimated from type sizes, not measured, since the core does no text layout. Numbers in converted sheets show without their number format unless the Excel layer formats them.

## Testing

```sh
cd crates
cargo test                                   # unit tests, and documents built in code (tests/write.rs)
python3 samples.py /tmp/office-samples       # sample files from python-docx, python-pptx, openpyxl and LibreOffice
OFFICE_SAMPLES=/tmp/office-samples cargo test                                   # also round-trips the samples
OFFICE_SAMPLES=/tmp/office-samples cargo test --test other_apps -- --ignored    # new, edited and converted files open in LibreOffice and the Python libraries
cargo run --features cli --bin officec -- save some.pptx /tmp/out.pptx          # which parts a save rewrites
```

Nothing binary is checked in: tests build their documents in code, and `samples.py` makes the rest. The sample tests check that each file's parts round-trip, that the front ends read what the file holds, and that saving an unchanged file leaves every part byte-identical. `other_apps` needs `soffice` on the PATH; it uses the Python libraries if they're installed.
