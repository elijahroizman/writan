# The office engine

The Word, PowerPoint and Excel editors run on one Rust engine compiled to WebAssembly. The [office compiler](office-compiler.md) reads a file into the shared model and writes it back. The engine lays that model out, draws it and edits it, and the editors in `src/editors` are React UI over it.

```
            office-core (read, model, write)
                       │
             office-engine (shared)
   text measuring · line breaking · flow layout · tables
   positions · selections · edit steps · undo · display list
                       │
   ┌───────────────────┼───────────────────┐
office-word        office-slides       office-sheets
 view: pages,       view: slides,       view: grid,
 sections,          placeholders,       number formats,
 headers, notes     shapes, tables      formulas
   └───────────────────┼───────────────────┘
                       │
         office-wasm (WordDoc, SlidesDoc, SheetsDoc)
                       │
     src/engine (host, paint) → src/editors (React UI)
```

## Crates

- **`office-engine`** holds everything the three formats share:
  - `measure`: text widths and font metrics, cached per style and string.
  - `para`: breaking a paragraph into lines (Unicode line breaking, tabs, justification, indents, line spacing).
  - `flow`: stacking paragraphs and tables into a column, with an incremental re-layout from the first changed block.
  - `pos`: positions and selections as a path of block indexes plus an offset.
  - `edit`: `Splice` steps that insert, delete, split and format.
  - `history`: undo, merging runs of typing into one step.
  - `display`: the display list (text, rects, lines, images, paths, groups) the host paints.
  - `geometry`: shape outlines.
  - `collab`: keeping the copies of a shared document the same (see [editing together](collaboration.md)).
- **`office-word`, `office-slides`, `office-sheets`** each add a `view` module on top of their layer:
  - `WordView` paginates (widows and orphans, keep-with-next, page breaks), places headers, footers, footnotes and floating pictures, and handles the editing commands.
  - `SlidesView` resolves placeholders through layouts and masters, applies theme style references and table styles, and edits frames, text and slides.
  - `SheetsView` turns a sheet into grid geometry, draws cells with Excel's number formats, overflow and merges, parses what's typed (numbers, percentages, currency, dates, formulas) and recalculates. A sheet's size (the rows and columns someone set in the inspector) isn't something Excel has, so the sheets layer keeps it in a hidden name scoped to the sheet, `Writan.SheetSize`, holding a constant such as `{500,52}`. Excel keeps names it doesn't use, and a constant needs no change when the sheet is renamed. A sheet without one is 200 rows and 26 columns past what's in use.
- **`office-wasm`** wraps the three views for JavaScript. Each has a JSON command interface (`{ cmd: "insertText", … }`), plus direct calls for the hot paths: paint, hit testing and state.

## Measuring

The engine never measures text itself. The host passes in a `Measurer`, which in the app is `src/engine/host.ts` measuring with a canvas. Line breaks therefore match exactly what the browser draws, system fonts included. The line gap Word adds comes from a DOM probe, since canvas doesn't report it. Carlito, Caladea and the Liberation fonts are bundled under `src/engine/fonts` as metric-compatible stand-ins for Calibri, Cambria, Arial, Times New Roman and Courier New. When a font finishes loading, `fontsChanged` clears the measure cache and the view lays out again.

## Painting

A view paints one page, slide or block of cells into a display list. `src/engine/paint.ts` turns the list into an HTML string. Text styles become CSS classes in a per-document style sheet, and each run of text is placed with its baseline at `top = baseline - ascent`. Pages whose list hasn't changed since the last paint are skipped (they're hashed). Selections, carets and find highlights are a separate overlay list, so moving the caret doesn't repaint the page.

## Speed

Typing re-lays out only from the edited paragraph. A page's blocks stay cached until something above them changes height. Paragraph specs, layouts and table looks are cached. While the app is idle after launch it loads the wasm, the fonts and a warm-up document, so the first file opens without a cold start. Saving re-serializes only the parts that changed. Every other part is copied from the original zip still compressed, and the original body is parsed once on open and shared with each save that compares against it.

## Adding a command

1. Add the method to the format's view (for example `WordView::toggle` in `office-word/src/view/commands.rs`), as one `History` step so undo works.
2. Add a variant to the binding's `Cmd` enum in `office-wasm/src/<format>.rs`.
3. Call it from the editor through the engine wrapper in `src/engine/<format>/engine.ts`.
4. Add a test in the view's `tests.rs` that runs the command on a document built in code and checks the saved file.

## Building

`npm run build` and `npm run dev` call `scripts/engine.mjs`, which builds `office-wasm` for `wasm32-unknown-unknown` and runs `wasm-bindgen` into `src/engine/wasm` (gitignored). It needs the target (`rustup target add wasm32-unknown-unknown`) and `wasm-bindgen-cli` at the crate's exact version (`cargo install wasm-bindgen-cli --version 0.2.129`). It skips the build when nothing in `crates/` changed; `npm run engine` forces one.
