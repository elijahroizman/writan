# Vector PDF Export and Offline OCR

Writan exports Office documents, charts, equations, and images as native vector PDFs via pure-Rust libraries compiled to a lazy WebAssembly module, and performs offline optical character recognition (OCR) locally in the browser and desktop app with zero remote inference requests.

## Vector PDF export architecture

- **Shared Rust engine (`crates/office-pdf`) and WASM bindings (`crates/office-pdf-wasm`)**:
  Built via `node scripts/engine.mjs` into `src/lib/pdf/wasm/` and dynamically loaded inside a dedicated Web Worker (`src/lib/pdf/worker.ts`, client in `src/lib/pdf/client.ts`) only when exporting PDFs, converting to PDF, bundling tabs, or inspecting/injecting PDF OCR layers. Startup bundles (`index-*.js`) contain zero references to `office_pdf_wasm`.
- **Display-list and vector fidelity (`crates/office-pdf/src/writer.rs`, `svg_path.rs`)**:
  - Translates Writan's `PaintItem` display lists (runs, styled paragraphs, table borders/shading, vector shapes, headers/footers, footnotes, MathJax/OMML SVGs, and Excel `ChartSvg` graphics) into PDF 1.7 vector paths and text operators instead of rasterizing full pages to JPEG.
  - Passes through JPEG images losslessly via `/DCTDecode` and encodes alpha-channel PNGs with `/SMask` streams (`image_pdf.rs`).
  - Emits interactive clickable `/URI` link annotations only for validated `http://`, `https://`, and `mailto:` schemes.
- **Typography, font subsetting, and BiDi/RTL (`crates/office-pdf/src/fonts.rs`)**:
  - Uses `rustybuzz` for OpenType/Complex script shaping and `subsetter` to embed subsetted TrueType CID (`CIDFontType2` / `/Identity-H`) fonts with `/ToUnicode` CMaps and `/ActualText` spans where decomposition or ligatures apply.
  - Lazy fallback fonts (`NotoSans-Regular`, `NotoSansArabic-Regular`, `NotoSansArabic-Bold`, `NotoSansMath-Regular` under `src/engine/fonts/pdf/`, OFL-1.1) handle Arabic, extended math, and non-Latin glyphs outside the startup font preload.
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
  - For unsigned PDFs, `inject_ocr_text_layer` uses `lopdf::IncrementalDocument` to append compressed invisible (`3 Tr`) text streams and subsetted font resources, preserving the original PDF bytes, scanned images, annotations, forms, and bookmarks intact while skipping pages with existing digital text by default.
  - Encrypted PDFs are rejected. Digitally signed PDFs are rejected unless explicitly confirmed (`allow_signed: true`), in which case `inject_ocr_text_layer` writes a separate unsigned searchable copy with `/Sig` fields, `/SigFlags`, and `/Perms` cleanly stripped while leaving the signed source file untouched.
- **Editable reconstruction (`crates/office-wasm/src/ocr_docx.rs`, `src/lib/ocr/toDocx.ts`)**:
  - `ocrToDocx` reconstructs column-aware reading order, unwrapped paragraphs, built-in headings, native lists (note: Hebrew ordered lists emit Word's `hebrew2` numbering format), bordered tables, RTL paragraphs, and page breaks into `.docx`, alongside `ocrResultsToPlainText` and `ocrResultsToMarkdown`.
- **UI & format integrations**:
  - PDF Editor scanned-page banner and cancellable OCR modal (`src/editors/pdf/PdfEditor.tsx`), Images plugin WebP viewer + host RPC OCR/Searchable-PDF actions (`plugins/images/`, `src/plugins/`, `src-tauri/tauri.conf.json`), `Settings › Offline OCR`, `ConvertDialog` (`pdf → docx`, `image → pdf/txt/md/docx`), native Tauri menu, command palette, and cross-platform shortcuts via `shortcut()`.

## Verified Benchmarks & Accuracy

Measured on Windows (`x86_64-pc-windows-msvc`, `VITE_TEST_HOOKS=1` production preview) with `node scripts/benchmark-pdf-ocr.mjs` and `node tests/stress/ui/offline-ocr-accuracy.mjs`:

### Bundle & Lazy Asset Isolation

| Asset | File | Bytes | Gzip bytes | Loaded at startup |
| --- | --- | ---: | ---: | --- |
| App entry | `index-*.js` | 833,049 | 261,626 | yes (`0` eager PDF-export / Tesseract refs) |
| PDF editor | `PdfEditor-*.js` | 51,363 | 17,786 | no |
| PDF export worker | `worker-*.js` | 4,885 | 1,960 | no |
| PDF export WASM | `office_pdf_wasm_bg-*.wasm` | 2,260,860 | 893,932 | no |
| OCR worker | `worker-*.js` | 10,306 | 4,814 | no |
| Tesseract SIMD WASM | `tesseract-core-simd-lstm-*.wasm` | 2,871,377 | 1,068,005 | no |
| Tesseract scalar WASM | `tesseract-core-lstm-*.wasm` | 2,871,085 | 1,067,839 | no |

### Vector Export vs Canvas-to-JPEG Baseline

| Fixture | Pipeline | PDF Bytes | Export ms | Pages | PDFium Chars | Links | JPEG XObjects |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `baseline-word.docx` | Native Vector | 26,587 (-59.2%) | 56.0 | 1 | 136 | 1 | 0 |
| `baseline-word.docx` | Saved Baseline | 65,105 | — | 1 | 0 | 0 | 1 |
| `baseline-slides.pptx` | Native Vector | 6,589 (-86.7%) | 14.1 | 1 | 20 | 0 | 0 |
| `baseline-slides.pptx` | Saved Baseline | 49,383 | — | 1 | 0 | 0 | 1 |
| `baseline-sheets.xlsx` | Native Vector | 7,243 (-49.9%) | 9.0 | 1 | 37 | 0 | 0 |
| `baseline-sheets.xlsx` | Saved Baseline | 14,471 | — | 1 | 0 | 0 | 1 |

### Offline OCR Latency & Reconstruction

| Language | IndexedDB Read ms | Worker Init ms | Recognition ms | Lossless PDF Injection ms | DOCX Build ms | Chars |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `eng` | 7.4 | 132.1 | 294.0 | 3.7 | 3.6 | 171 |
| `heb` | 1.9 | 125.8 | 227.4 | 2.1 | 1.5 | 112 |
| `ara` | 3.2 | 126.0 | 256.4 | 2.4 | 0.9 | 110 |

### Empirical CER / WER Accuracy (`tessdata_fast` Synthetic Scans)

| Fixture | Packs | CER | WER | Reference Chars | Recognition ms |
| --- | --- | ---: | ---: | ---: | ---: |
| Clean English | `eng` | 0.00% | 0.00% | 169 | 252.4 |
| Clean Hebrew | `heb` | 0.00% | 0.00% | 112 | 198.3 |
| Clean Arabic | `ara` | 0.00% | 0.00% | 110 | 231.5 |
| Mixed Hebrew & English | `heb+eng` | 0.00% | 0.00% | 138 | 277.2 |
| English rotated 90° | `eng` | 0.00% | 0.00% | 169 | 179.7 |
| English skewed -6° | `eng` | 0.00% | 0.00% | 169 | 198.7 |
