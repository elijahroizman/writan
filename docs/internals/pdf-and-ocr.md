# Vector PDF Export and Offline OCR

Writan exports Office documents, charts, equations, and images as native vector PDFs via pure-Rust libraries compiled to a lazy WebAssembly module, and performs offline optical character recognition (OCR) locally in the browser and desktop app with zero remote inference requests.

## Vector PDF export architecture

- **Shared Rust engine (`crates/office-pdf`) and WASM bindings (`crates/office-pdf-wasm`)**:
  Built via `node scripts/engine.mjs` into `src/lib/pdf/wasm/` and dynamically loaded inside a dedicated Web Worker (`src/lib/pdf/worker.ts`, client in `src/lib/pdf/client.ts`) only when exporting PDFs, converting to PDF, bundling tabs, or inspecting/injecting PDF OCR layers. Startup bundles (`index-*.js`) contain zero references to `office_pdf_wasm`.
- **Display-list and vector fidelity (`crates/office-pdf/src/writer.rs`, `svg_path.rs`)**:
  - Translates Writan's `PaintItem` display lists (runs, styled paragraphs, table borders/shading, vector shapes, headers/footers, footnotes, MathJax/OMML SVGs, and Excel `ChartSvg` graphics) into PDF 1.7 vector paths and text operators instead of rasterizing full pages to JPEG.
  - Passes through JPEG images losslessly via `/DCTDecode` with EXIF orientation matrices (`1`–`8`), encodes alpha-channel PNGs with `/SMask` streams (`image_pdf.rs`), re-encodes opaque HEIC and WebP photos as JPEG (other formats as PNG), and keeps MathJax and `ChartSvg` vector (a chart whose labels the bundled fonts lack is drawn as a picture). An embedded SVG goes through the browser as a 4× PNG when it uses anything `svg_path.rs` doesn't draw: an element outside its list, a `DOCTYPE`, `class` styles, `url(#...)` references, or text the bundled fonts lack. An SVG the parser still can't read is left out rather than failing the export.
  - Filters paginated Excel spreadsheet paint items by slice Y-bounds before worker JSON serialization so multi-page workbooks do not duplicate full-sheet payloads on every slice. Text and cell groups go only on the pages they overlap: text hidden by a page's clip would still be found by search and copy.
  - Emits interactive clickable `/URI` link annotations only for validated `http://`, `https://`, and `mailto:` schemes.
- **Typography, font subsetting, and BiDi/RTL (`crates/office-pdf/src/fonts.rs`)**:
  - Uses `rustybuzz` for OpenType/Complex script shaping and `subsetter` to embed subsetted TrueType CID (`CIDFontType2` / `/Identity-H`) fonts with `/ToUnicode` CMaps, nominal `/W` advance tracking for kerning (`TJ`) adjustments, and `/ActualText` spans where decomposition or ligatures apply.
  - Lazy fallback fonts (`NotoSans-Regular`, `NotoSansArabic-Regular`, `NotoSansArabic-Bold`, `NotoSansMath-Regular` under `src/engine/fonts/pdf/`, OFL-1.1) handle Arabic, extended math, and non-Latin glyphs outside the startup font preload. For runs with characters outside what the bundled fonts cover (`RASTER_TEXT_RE`: CJK including supplementary Extension B, Indic, Thai, emoji and other scripts), `src/lib/pdf/exporter.ts` renders crisp 4× raster overlays with the system's fonts padded to measured glyph ink bounds paired with invisible (`3 Tr`) Unicode text layers and preserved vector underlines/strikethroughs (note: PDFium's `FPDFText` drops non-BMP surrogate pairs in extraction while PyMuPDF and standard readers read `/ActualText` cleanly).
  - Shapes RTL runs into HarfBuzz's native visual left-to-right batched `TJ`/`Tj` streams with same-font neutral spans, enabling accurate Unicode extraction and search across standalone Hebrew (`שלום עולם`), Arabic (`مرحبا بالعالم`), and LTR-base mixed paragraphs in both PDFium (`@embedpdf/pdfium`) and PyMuPDF.
  - *Known upstream viewer heuristic*: On RTL-base paragraphs that also contain space-separated LTR words (for example `שלום עולם Hello 2026 مرحبا بالعالم`), PDFium's internal text extractor geometrically reorders the space-separated LTR tokens descending by X (`2026 Hello`), while PyMuPDF extracts logical order (`Hello 2026`). Writan emits spec-faithful visual-LTR PDF streams rather than synthetic NBSP workarounds.

## Offline OCR architecture

- **Worker runtime (`src/lib/ocr/worker.ts`, `client.ts`)**:
  - Runs `tesseract.js-core@6.1.2` (`tesseract-core-simd-lstm.wasm` with scalar `tesseract-core-lstm.wasm` fallback bundled via Vite assets) inside a dedicated worker with zero external CDN requests during recognition (`page.route` verified).
  - Reuses initialized language contexts across multi-page jobs, terminates idle workers after 30 seconds, and cancels cleanly via `AbortController`.
- **Preprocessing (`src/lib/ocr/preprocess.ts`)**:
  - Detects coarse 90°/180°/270° page rotation and fine skew, normalizes grayscale/contrast, scales to optimal recognition DPI, and maps recognized word/line/paragraph bounding boxes back into original image coordinates.
- **Scanned pages in the PDF editor (`src/editors/pdf/ocr.ts`)**:
  - Nothing to start: when a PDF opens, `documentTextStats` reports each page's text and `imageCoverage` (the share of the page under pictures, added up so scans drawn in strips count, with pictures in form XObjects carried to the page through each form's matrix) and, on pages that are mostly a picture, `textCoverage` (the share its characters' boxes cover). `isScannedPage` takes pages at least 30% picture with under 10 characters or under 3% text: library copies with a "downloaded from" footer or a page number are scans, blank pages and drawings aren't.
  - `recognizePages` renders those pages at about 220 dpi with the PDF editor's own PDFium worker (so owner-password PDFs work) at the lowest priority, nearest the page on screen first, while the tab is awake. Words whose middle falls in the page's own text are dropped, so a footer isn't found twice.
  - Results stay in memory in `DocStore` (by page key and rotation): `getText` adds one run per recognized word (`ocr: true`, no PDFium indices) to the text layer, so selection and copy work, highlights use the word boxes, and Find searches the words (`searchPageText`: case-insensitive, curly quotes and dashes folded, one box per line of a match). Pages finished while Find is open add their matches. The file isn't changed, and the left pane's footer shows "Reading scans 3/12" while it works.
- **Verified language pack manager (`src-tauri/src/ocr.rs`, `src/lib/ocr/packs.ts`, `catalog.ts`)**:
  - English (`src/lib/ocr/eng.traineddata`, the same pinned `tessdata_fast` file, Apache-2.0) ships as an asset and is always installed. `availableLangs` downloads the other chosen languages on first use and leaves out any that fail; the first-run choice is English plus the system's languages that have a pack.
  - Curates 12 `tesseract-ocr/tessdata_fast` packs (`eng`, `heb`, `ara`, `fra`, `deu`, `spa`, `ita`, `por`, `rus`, `chi_sim`, `jpn`, `kor`) pinned at commit `87416418657359cb625c412a48b6e1d6d41c29bd` with exact byte lengths and SHA-256 digests enforced on download and every offline read (`src-tauri` disk cache on desktop, IndexedDB in browser).
  - Strictly validates pack codes against the fixed allowlist to reject path traversal.
- **Lossless PDF layer injection and signature safety (`crates/office-pdf/src/ocr_layer.rs`, `src/lib/ocr/pdfOcr.ts`)**:
  - Used for searchable copies of pictures. `ocrPdfDocument` (PDF to Word) only reads pages, so it skips this structural check and handles PDFs lopdf can't load.
  - For unsigned PDFs, `inject_ocr_text_layer` uses `lopdf::IncrementalDocument` to append compressed invisible (`3 Tr`) text streams (using oriented `cm` baseline transforms on sideways/skewed scans) and subsetted font resources, preserving the original PDF bytes, scanned images, annotations, forms, and bookmarks intact while skipping pages with existing digital text by default.
  - Encrypted PDFs are rejected. Digitally signed PDFs are rejected unless explicitly confirmed (`allow_signed: true`), in which case `inject_ocr_text_layer` writes a separate unsigned searchable copy with `/Sig` fields, `/V` values, widget `/Kids`, `/SigFlags`, and `/Perms` cleanly stripped while preserving non-signature form fields and annotations, and leaving the signed source file untouched. Native OCR file saves enforce race-free `persist_noclobber` (`x-no-clobber`) in `src-tauri/src/files.rs`.
- **Editable reconstruction (`crates/office-wasm/src/ocr_docx.rs`, `src/lib/ocr/toDocx.ts`)**:
  - `ocrToDocx` reconstructs column-aware reading order, unwrapped paragraphs, built-in headings, native lists (note: Hebrew ordered lists emit Word's `hebrew2` numbering format; blank source pages with no recognized text are omitted), bordered tables, RTL paragraphs, and page breaks into `.docx`, alongside `ocrResultsToPlainText` and `ocrResultsToMarkdown`.
- **Digital PDFs to Word (`src/lib/pdfToDocx.ts`, `layout` in `src/editors/pdf/engine/worker.ts`, `crates/office-wasm/src/pdf_docx.rs`)**:
  - The PDF inspector's **Edit as Word document**, **File › Edit PDF as Word Document** (`edit-as-word`, `src/state/pdfToWord.ts`) and the Convert dialog's PDF → Word all run `pdfToDocx` on a PDFium engine of their own, opened on the tab's current bytes.
  - `layout` reads each page's characters with their font (`FPDFFont_GetFamilyName` for embedded fonts, the cleaned base name otherwise; Carlito, Caladea and the Liberation fonts become Calibri, Cambria, Arial, Times New Roman and Courier New), size (PDFium's size times the character matrix's scale: Word writes size 1, scaled), weight, slant, fill colour and link, and groups them into fragments: characters on one baseline closer than 0.8 em, split into spans where the format changes. Text turned away from upright is skipped. It also returns thin rules (underlines, table borders) and pictures: each image object drawn alone, on a transparent background, at up to 3× its size on the page (every other object is switched off with `FPDFPageObj_SetIsActive`, hence the engine of its own), and drawings (clusters of curved or slanted paths with little text inside) drawn whole, labels included.
  - `pdf_docx.rs` joins fragments into lines, takes out rows of aligned cells as tables (not columns of running text), rejoins wide gaps that the lines around run across (justified text) or sets a lone right-hand part after a tab stop, splits the page into columns where it splits into side-by-side columns, and groups lines into paragraphs: a line continues the one above when it has the same size, spacing and alignment (or hangs under a list marker's text) and the line above was full, that is, the next word wouldn't have fitted. Reading order is `reading_order` from `ocr_docx.rs`. Paragraphs keep their alignment, indents, first-line indent, spacing before and line spacing; larger short lines become Heading 1 to 3 by size, keeping their own font and colour; list markers become Word lists. Lines repeated at the top or bottom of most pages, and page numbers, are dropped. A paragraph running from one column, or page, into the next is one paragraph; otherwise each PDF page starts a page (`pageBreakBefore`). The page size is the most common one and the margins come from where the text sits.
  - Scanned pages (`isScannedPage`) are recognized and their words go through the same reconstruction; a document of scans only uses `ocrResultsToDocx`. A PDF that already has an invisible text layer uses it, and drops the page-sized scan behind it.
- **UI & format integrations**:
  - Automatic recognition in the PDF editor (above), Images plugin WebP viewer + host RPC OCR/Searchable-PDF actions (`plugins/images/`, `src/plugins/`, `src-tauri/tauri.conf.json`), `Settings › Text in scanned PDFs`, `ConvertDialog` (`pdf → docx` and `image → txt/md/docx` with OCR; `image → pdf` is a plain picture PDF), native Tauri menu, command palette, and cross-platform shortcuts via `shortcut()`.

## Verified Benchmarks & Accuracy

Measured on Windows (`x86_64-pc-windows-msvc`, `VITE_TEST_HOOKS=1` production preview) with `node scripts/benchmark-pdf-ocr.mjs` and `node tests/stress/ui/offline-ocr-accuracy.mjs`:

### Bundle & Lazy Asset Isolation

| Asset | File | Bytes | Gzip bytes | Loaded at startup |
| --- | --- | ---: | ---: | --- |
| App entry | `index-*.js` | 833,064 | 261,634 | yes (`0` eager PDF-export / Tesseract refs) |
| PDF editor | `PdfEditor-*.js` | 51,363 | 17,790 | no |
| PDF export worker | `worker-*.js` | 4,885 | 1,959 | no |
| PDF export WASM | `office_pdf_wasm_bg-*.wasm` | 2,268,517 | 896,069 | no |
| OCR worker | `worker-*.js` | 10,619 | 4,960 | no |
| Tesseract SIMD WASM | `tesseract-core-simd-lstm-*.wasm` | 2,871,377 | 1,068,005 | no |
| Tesseract scalar WASM | `tesseract-core-lstm-*.wasm` | 2,871,085 | 1,067,839 | no |

### Vector Export vs Canvas-to-JPEG Baseline

| Fixture | Pipeline | PDF Bytes | Export ms | Pages | PDFium Chars | Links | JPEG XObjects |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `baseline-word.docx` | Native Vector | 26,657 (-59.1%) | 63.6 | 1 | 136 | 1 | 0 |
| `baseline-word.docx` | Saved Baseline | 65,105 | — | 1 | 0 | 0 | 1 |
| `baseline-slides.pptx` | Native Vector | 6,589 (-86.7%) | 14.5 | 1 | 20 | 0 | 0 |
| `baseline-slides.pptx` | Saved Baseline | 49,383 | — | 1 | 0 | 0 | 1 |
| `baseline-sheets.xlsx` | Native Vector | 7,282 (-49.7%) | 8.4 | 1 | 37 | 0 | 0 |
| `baseline-sheets.xlsx` | Saved Baseline | 14,471 | — | 1 | 0 | 0 | 1 |

### Offline OCR Latency & Reconstruction

| Language | IndexedDB Read ms | Worker Init ms | Recognition ms | Lossless PDF Injection ms | DOCX Build ms | Chars |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `eng` | 7.9 | 129.0 | 296.1 | 4.0 | 3.4 | 171 |
| `heb` | 2.1 | 127.1 | 235.5 | 3.0 | 1.4 | 112 |
| `ara` | 3.9 | 127.4 | 251.6 | 3.1 | 1.1 | 110 |

### Empirical CER / WER Accuracy (`tessdata_fast` Synthetic Scans)

| Fixture | Packs | CER | WER | Reference Chars | Recognition ms |
| --- | --- | ---: | ---: | ---: | ---: |
| Clean English | `eng` | 0.00% | 0.00% | 169 | 265.0 |
| Clean Hebrew | `heb` | 0.00% | 0.00% | 112 | 215.8 |
| Clean Arabic | `ara` | 0.00% | 0.00% | 110 | 236.5 |
| Mixed Hebrew & English | `heb+eng` | 0.00% | 0.00% | 138 | 287.5 |
| English rotated 90° | `eng` | 0.00% | 0.00% | 169 | 186.0 |
| English skewed -6° | `eng` | 0.00% | 0.00% | 169 | 196.9 |
