# Verified corpus fixes

Verified on `eli@bbb` in `~/stress-corpus-fix`, based on application baseline `b088f996153b1a639dedcd9ca5119ee050e892e5`. The original [BUGS-corpus.md](BUGS-corpus.md) remains unchanged. [Compact verification evidence](corpus/evidence/fixes-2026-10-24.json) records source/sample hashes, test results, browser markers, table-order checks, and reference pagination. Full logs, screenshots, saved documents, and PDFs remain on bbb.

## CORPUS-003 fixed

DOCX reading now retains empty positions for modeled `w:tc` and `w:tr` children. Writing fills those positions in model order while retaining raw content controls and other siblings. Added children follow the final modeled position; removed children leave no duplicate XML. The table origin also retains `w:tblGrid` in its original position, including when a raw wrapped row precedes the first modeled row.

The unchanged NZ RFP save preserves the original checklist order. LibreOffice exports the original, CLI save, Chromium app save, and WebKit app save as 22 pages each. The baseline saves produced 32 pages. These use the same reference installation. All 50 original corpus DOCX files plus both generated samples preserve their table/row child order and raw table content controls through CLI saves. Both browser saves also pass that check for the RFP and two CJK originals. This check does not claim that every document part is unchanged.

[table-content-controls.docx](samples/table-content-controls.docx), 1.1 KB, is an original generated sample with controls before/between/after modeled cells and rows, a leading wrapped row, and a merged cell. `office-core/tests/docx_table_order.rs` covers unchanged and repeated saves, text/header edits, merges, added cells/rows, and removed model children. All three tests fail against the baseline implementation and pass with the fix.

## CORPUS-012 fixed

`office-word` now formats `ideographTraditional` as `甲` through `癸` and `ideographZodiac` as `子` through `亥`. Values outside those finite sequences use decimal, matching the corpus reference exports. Numbering templates retain their punctuation, nested-level references, and start overrides. Microsoft's [numFmt format examples](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-docx/a1bb5809-e361-4e49-8e16-7f1a67da4121) identify the two ideograph sequences.

[cjk-list-markers.docx](samples/cjk-list-markers.docx), 1.6 KB, is an original generated fixture that includes the transition to decimal. Four regressions check the finite sequences, zero/large values, nested templates and overrides, actual display-list markers, and save/reopen behavior. With `CORPUS_FILES` set, they also check both original `lo-cjklist30.docx` and `lo-cjklist31.docx` documents. All four fail against the baseline implementation and pass with the fix. Both browsers paint all 20 expected markers from each original, including the decimal suffixes after the ideograph ranges.

## CORPUS-011 fixed

`w:bidiVisual` already reached the table layout through the right-to-left work of #41 (`5543e8e`): `office-word`'s `table_look` reads it from `w:tblPr`, and `office-engine`'s `flow::table` mirrors each cell's x and swaps its start/end margins and borders, so the first grid column draws at the right and the indent measures from the right edge. The gap was untested rather than unfixed.

[rtl-table.docx](samples/rtl-table.docx), 1.0 KB, is an original generated sample with a `w:bidiVisual` two-column table of unequal widths and a `w:tblInd`, plus an ordinary table. `office-word/tests/rtl_table.rs` checks the painted display order of every row, that the LTR table is unchanged, and that `w:bidiVisual` survives a save/reopen; with `CORPUS_FILES` set it also checks the original `lo-table-rtl.docx`. With the `bidiVisual` read reverted, the sample assertions fail (`second` paints right of `first`).

## Commands and results

All builds, formatting checks, Python analysis, and browser tests ran on bbb. No local builds or tests ran. Sources were touched on bbb before Cargo runs to avoid stale fingerprints from the clock difference. The final WASM command used `--force`; its log confirms compilation of the changed crates. Recorded source hashes match the local commit inputs.

From the dedicated remote checkout:

```sh
export PATH="$HOME/.local/node24/bin:$HOME/.cargo/bin:$HOME/.local/bin:$PATH"
python3 tests/stress/samples/make-corpus-samples.py
cd crates
rg --files -g '*.rs' | xargs touch
cargo fmt --all -- --check
CORPUS_FILES=$HOME/stress-corpus-files cargo test -p office-core -p office-word
cargo build -p office-core --features cli --bin officec
cd ..
python3 tests/stress/verify-corpus-fixes.py --officec crates/target/debug/officec
node scripts/engine.mjs --force
VITE_TEST_HOOKS=1 node node_modules/vite/bin/vite.js build
```

Formatting passed. Core/Word reported 196 passing tests, including the seven new regressions and one doc test. Existing optional sample tests still require their separate `OFFICE_SAMPLES` fixtures; the new CJK corpus test ran with `CORPUS_FILES` supplied. The CLI, forced WASM, and Vite builds passed. The table-order checker passed all 52 saves.

The preview ran at `http://127.0.0.1:1453`, with `dist/dev-samples` linked to `~/stress-corpus-files`. The established remote Playwright workflow satisfies the bbb-only constraint. The Browser plugin was unavailable, and the T3 preview had no attached tab. Chromium and WebKit used 1440 by 1100 viewports.

```sh
export CORPUS_URL=http://127.0.0.1:1453
export CORPUS_RESULTS=tests/stress/corpus/results-fixes
CORPUS_FILTER='^nz-template-rfp-government-model.docx$|^lo-cjklist3[01].docx$' node tests/stress/corpus/browser.mjs
node tests/stress/verify-cjk-markers.mjs
python3 tests/stress/verify-corpus-fixes.py --mode chromium
python3 tests/stress/verify-corpus-fixes.py --mode webkit
# Use the SOFFICE and LD_LIBRARY_PATH setup from corpus/README.md.
python3 tests/stress/corpus/reference.py --results tests/stress/corpus/results-fixes --filter '^nz-template-rfp-government-model.docx$'
```

All six browser open/save flows passed with zero console/page errors. Page identity was Writan at the requested document route. Documents produced content and screenshots, with no framework error overlay reported. Saving produced a downloaded DOCX that passed the structural checks; reference reopening confirmed 22-page RFP output. The marker probe independently checked the actual browser display lists. Raw screenshots remain on bbb because redistribution rights for the originals were not established. The dedicated preview was stopped after verification.

## Integration and limits

For integration with core `fdb8aa9` and grid preservation `ade662c`, retain this branch's `table_shell(tbl, "tr")` reader call and both slot helpers. This final revision has no omit list and retains the original grid. Keep the core branch's `common::grid` writer implementation, including its unknown column metadata and tracked-grid preservation. This branch does not modify `common.rs`. Re-run the combined core tests and forced WASM build after resolving the shared read/write files.

Raw content controls remain unmodeled for rendering and editing. Full footnote, SmartArt, chart, math, RTL-table, and other open baseline findings are outside these two fixes. No release files or versions changed; nothing was pushed or released.
