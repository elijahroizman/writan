# Vector PDF Export and Offline OCR

Writan exports Office documents, charts, equations, and images as native vector PDFs via pure-Rust libraries compiled to a lazy WebAssembly module, and performs offline optical character recognition (OCR) locally in the browser and desktop app with zero remote inference requests.

## Vector PDF export architecture

- **Shared Rust engine (`crates/office-pdf`) and WASM bindings (`crates/office-pdf-wasm`)**:
  Built via `node scripts/engine.mjs` into `src/lib/pdf/wasm/` and dynamically loaded inside a dedicated Web Worker (`src/lib/pdf/worker.ts`, client in `src/lib/pdf/client.ts`) only when exporting PDFs, converting to PDF, bundling tabs, or inspecting/injecting PDF OCR layers. Startup bundles (`index-*.js`) contain zero references to `office_pdf_wasm`.
- **Display-list and vector fidelity (`crates/office-pdf/src/writer.rs`, `svg_path.rs`)**:
  - Translates Writan's `PaintItem` display lists (runs, styled paragraphs, table borders/shading, vector shapes, headers/footers, footnotes, MathJax/OMML SVGs, and Excel `ChartSvg` graphics) into PDF 1.7 vector paths and text operators instead of rasterizing full pages to JPEG.
  - Passes through JPEG images losslessly via `/DCTDecode` with EXIF orientation matrices (`1`–`8`), encodes alpha-channel PNGs with `/SMask` streams (`image_pdf.rs`), and rasterizes embedded SVGs using unsupported gradient/filter/mask constructs (`url(#...)`) cleanly while keeping MathJax and `ChartSvg` vector.
  - Filters paginated Excel spreadsheet paint items by slice Y-bounds before worker JSON serialization so multi-page workbooks do not duplicate full-sheet payloads on every slice.
  - Emits interactive clickable `/URI` link annotations only for validated `http://`, `https://`, and `mailto:` schemes.
- **Typography, font subsetting, and BiDi/RTL (`crates/office-pdf/src/fonts.rs`)**:
  - Uses `rustybuzz` for OpenType/Complex script shaping and `subsetter` to embed subsetted TrueType CID (`CIDFontType2` / `/Identity-H`) fonts with `/ToUnicode` CMaps, nominal `/W` advance tracking for kerning (`TJ`) adjustments, and `/ActualText` spans where decomposition or ligatures apply.
  - Lazy fallback fonts (`NotoSans-Regular`, `NotoSansArabic-Regular`, `NotoSansArabic-Bold`, `NotoSansMath-Regular` under `src/engine/fonts/pdf/`, OFL-1.1) handle Arabic, extended math, and non-Latin glyphs outside the startup font preload. For CJK runs lacking coverage in bundled Latin/Arabic/Math fonts (including supplementary Extension B characters matched by `CJK_RE`), `src/lib/pdf/exporter.ts` renders crisp 4× raster overlays padded to measured glyph ink bounds paired with invisible (`3 Tr`) Unicode text layers and preserved vector underlines/strikethroughs (note: PDFium's `FPDFText` drops non-BMP surrogate pairs in extraction while PyMuPDF and standard readers read `/ActualText` cleanly).
  - Shapes RTL runs into HarfBuzz's native visual left-to-right batched `TJ`/`Tj` streams with same-font neutral spans, enabling accurate Unicode extraction and search across standalone Hebrew (`שלום עולם`), Arabic (`مرحبا بالعالم`), and LTR-base mixed paragraphs in both PDFium (`@embedpdf/pdfium`) and PyMuPDF.
  - *Known upstream viewer heuristic*: On RTL-base paragraphs that also contain space-separated LTR words (for example `שלום עולם Hello 2026 مرحبا بالعالم`), PDFium's internal text extractor geometrically reorders the space-separated LTR tokens descending by X (`2026 Hello`), while PyMuPDF extracts logical order (`Hello 2026`). Writan emits spec-faithful visual-LTR PDF streams rather than synthetic NBSP workarounds.

## Offline OCR architecture

- **Worker runtime (`src/lib/ocr/worker.ts`, `client.ts`)**:
  - Runs `tesseract.js-core@6.1.2` (`tesseract-core-simd-lstm.wasm` with scalar `tesseract-core-lstm.wasm` fallback bundled via Vite assets) inside a dedicated worker with zero external CDN requests during recognition (`page.route` verified).
  - Reuses initialized language contexts across multi-page jobs, terminates idle workers after 30 seconds, and cancels cleanly via `AbortController`.
- **Preprocessing (`src/lib/ocr/preprocess.ts`)**:
  - Detects coarse 90°/180°/270° page rotation and fine skew, normalizes grayscale/contrast, scales to optimal recognition DPI, and maps recognized word/line/paragraph bounding boxes back into original image coordinates.
- **Verified language pack manager (`src-tauri/src/ocr.rs`, `src/lib/ocr/packs.ts`, `catalog.ts`)**:
  - Curates 12 `tesseract-ocr/tessdata_fast` packs (`eng`, `heb`, `ara`, `fra`, `deu`, `spa`, `ita`, `por`, `rus`, `chi_sim`, `jpn`, `kor`) pinned at commit `87416418657359cb625c412a48b6e1d6d41c29bd` with exact byte lengths and SHA-256 digests enforced on download and every offline read (`src-tauri` disk cache on desktop, IndexedDB in browser).
  - Strictly validates pack codes against the fixed allowlist to reject path traversal.
- **Lossless PDF layer injection and signature safety (`crates/office-pdf/src/ocr_layer.rs`, `src/lib/ocr/pdfOcr.ts`)**:
  - Scanned-page detection uses `documentTextStats` (`SCANNED_CHAR_THRESHOLD = 10`) in the existing PDF worker without eagerly loading `office-pdf-wasm` or `tesseract-core`.
  - For unsigned PDFs, `inject_ocr_text_layer` uses `lopdf::IncrementalDocument` to append compressed invisible (`3 Tr`) text streams (using oriented `cm` baseline transforms on sideways/skewed scans) and subsetted font resources, preserving the original PDF bytes, scanned images, annotations, forms, and bookmarks intact while skipping pages with existing digital text by default.
  - Encrypted PDFs are rejected. Digitally signed PDFs are rejected unless explicitly confirmed (`allow_signed: true`), in which case `inject_ocr_text_layer` writes a separate unsigned searchable copy with `/Sig` fields, `/V` values, widget `/Kids`, `/SigFlags`, and `/Perms` cleanly stripped while preserving non-signature form fields and annotations, and leaving the signed source file untouched. Native OCR file saves enforce race-free `persist_noclobber` (`x-no-clobber`) in `src-tauri/src/files.rs`.
- **Editable reconstruction (`crates/office-wasm/src/ocr_docx.rs`, `src/lib/ocr/toDocx.ts`)**:
  - `ocrToDocx` reconstructs column-aware reading order, unwrapped paragraphs, built-in headings, native lists (note: Hebrew ordered lists emit Word's `hebrew2` numbering format; blank source pages with no recognized text are omitted), bordered tables, RTL paragraphs, and page breaks into `.docx`, alongside `ocrResultsToPlainText` and `ocrResultsToMarkdown`.
- **UI & format integrations**:
  - PDF Editor scanned-page banner and cancellable OCR modal (`src/editors/pdf/PdfEditor.tsx`), Images plugin WebP viewer + host RPC OCR/Searchable-PDF actions (`plugins/images/`, `src/plugins/`, `src-tauri/tauri.conf.json`), `Settings › Offline OCR`, `ConvertDialog` (`pdf → docx`, `image → pdf/txt/md/docx`), native Tauri menu, command palette, and cross-platform shortcuts via `shortcut()`.

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
