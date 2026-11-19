# Writan stress findings, 2026-10-24

This is the consolidated status for `stress-test`. The original corpus report records the earlier baseline, not the final fixed behavior. S1 means data loss, an abort, or unsafe document execution; S2 means incorrect content or a broken workflow; S3 means a smaller defect. Implemented platform fixes without native verification are marked separately.

Every row names a small committed repro, a test that creates its sample, or a pinned source in [corpus/manifest.tsv](corpus/manifest.tsv). Reproduction commands and evidence are in the linked detailed reports. This is a bounded stress pass, not a claim that every document or platform is safe.

Additional findings recovered after the 0.10.5 release are tracked in [FOLLOWUP-core.md](FOLLOWUP-core.md). Their fixes are separate from that release.

## S1

| Finding | Final status | Reproducible sample or test |
| --- | --- | --- |
| UI-001: a crash or renderer reload loses unsaved edits and untitled tabs. | Open. Session restore keeps paths, not unsaved buffers. | [session.mjs](ui/session.mjs), [UTF-16 text](samples/S1-text-utf16le-bom.txt). The script confirms the loss; exit 0 means the observation completed. |
| CORE-001 / CORPUS-001: unchanged workbook saves replace 173,805 valid cached results with errors. | Fixed for open/unchanged save. Editing still recalculates unsupported formulas and can replace useful caches. | [Small cached-formula workbook](samples/S1-unchanged-formula-caches.xlsx); pinned `au-pbo-PBO---Build-your-own-budget---2026-27-Budget.xlsx`. |
| CORPUS-002: unchanged signed-PDF saves invalidate the digital signature. | Fixed. Clean revisions preserve the original or last saved bytes. Editing a signed PDF can invalidate its signature. | Pinned `us-budget-2025-appendix.pdf`; [PDF save regression](native/pdf-unchanged.mjs). |
| NATIVE-001 / UI-003: stale tabs and outside edits are silently overwritten. | Fixed for detected conflicts and simultaneous writes to the same path in this renderer. Content comparison catches equal-size/equal-time changes. Other processes and path aliases remain outside the lock. | [Data-safety suite](ui/datasafety.mjs), [autosave conflict](native/save-conflict.mjs). |
| NATIVE-002: predictable temporary save names can overwrite another file through a symlink or mix concurrent saves. | Fixed with exclusively created temporary files. | Rust `predictable_temp_symlink_cannot_overwrite_another_file` and `concurrent_saves_never_mix_bytes` create their own files. |
| UI-002: saving UTF-16 or legacy text replaces characters, removes BOMs, or changes untouched line endings. | Fixed for the tested encodings and line endings. Unchanged malformed BOM-marked input also retains its original bytes. Saving edits to malformed BOM-marked input is refused with an explanation; unmarked legacy encoding detection remains ambiguous. | [UTF-16 LE](samples/S1-text-utf16le-bom.txt), [UTF-16 BE](samples/S1-text-utf16be-bom.txt), [Windows-1252](samples/S1-text-windows1252.txt), [mixed line endings](samples/S3-text-mixed-eol.txt); [suite](ui/datasafety.mjs). |
| CORE-002: long flat formulas abort during recursive expression cloning. | Fixed in workbook recalculation. | [Long-formula workbook](samples/S1-long-formula-stack-overflow.xlsx). |
| CORE-003: deeply nested slide groups overflow the stack. | Fixed for the demonstrated inputs. XML over 256 nested elements is refused. | [Deep-group deck](samples/S1-deep-groups-stack-overflow.pptx). |
| CORE-004: a single large XML part exhausts memory before parsing. | Demonstrated cases fixed by a 128 MiB XML-part limit. Aggregate/model/media memory exhaustion remains a risk. | [Hostile generator](gen/hostile.py), oversized styles/slide/sheet/text cases documented in [core results](BUGS-core.md). |
| Recovered core fix: a huge row index or amplified shared strings exhaust memory. | Invalid row indices are refused; shared cell text has a budget. The amplification case still reaches about 1 GiB RSS before refusal. | [Huge row](samples/S1-row-2147483648-oom.xlsx), [shared-string amplification](samples/S1-shared-string-amplification-oom.xlsx). |
| UI-004 / recovered core fix: renamed Office files become blank editable documents or incompatible views. | Fixed with explicit type refusal. | [Word renamed as slides](samples/S2-docx-renamed.pptx), [workbook renamed as Word](samples/S1-xlsx-named-docx-panics.docx). |
| Recovered core fix: an escaped sheet name prevents saving. | Fixed. | [Escaped sheet name](samples/S1-sheet-name-escape-cant-save.xlsx). |
| UI-005: document links reach unsupported external schemes. | Fixed for the tested click paths. Only web and email URLs reach the opener. | [Word](samples/S1-docx-evil-links.docx), [slides](samples/S1-pptx-evil-links.pptx), [PDF](samples/S1-pdf-evil-links.pdf), [Markdown](samples/S1-markdown-xss.md); [suite](ui/security.mjs). |
| NATIVE-003: macOS Dock Quit or logout bypasses unsaved changes. | Termination handler implemented; actual macOS Dock/logout behavior remains unverified. | Edit [plain text](samples/S3-text-mixed-eol.txt) with autosave disabled, then quit from the Dock or log out. |

## S2

| Finding | Final status | Reproducible sample or test |
| --- | --- | --- |
| CORPUS-004: block content controls, including a university thesis TOC, are invisible. | Fixed for the saved content of block controls. The preview is read-only and the original XML survives saving. | Pinned `ucsd-ThesisTemplate_Master_v2.3.docx`; [corpus report](BUGS-corpus.md). |
| CORPUS-005: footnote bodies are absent although reference marks appear. | Open. | Pinned `lo-footnote.docx` and `au-budget-bp1_bs-4.docx`. |
| CORPUS-006: existing PowerPoint charts show empty rectangles. | Open. | Pinned `lo-chart_pt_color_bg1.pptx`. |
| CORPUS-007: SmartArt is absent in Word and PowerPoint. | Open. | Pinned `lo-smartart.docx` and `lo-smartart-accent-process.pptx`. |
| CORPUS-008: Office Math equations render blank. | Fixed for display: equations show as one line of linear text (accents as combining marks, Unicode scripts, written-out fractions and roots), not built-up math. The original XML survives saving. | Pinned `lo-math-accents.docx`; Rust `office_math_equations_are_drawn_and_survive_saving`, and `original_corpus_math_accents_are_drawn` with `CORPUS_FILES`. |
| CORPUS-010: conditional-formatting colors are missing. | Open; original rules survive saves. | Pinned `lo-conditional_fmt_checkpriority.xlsx`. |
| CORPUS-011: RTL table columns display in the wrong order. | Open. | Pinned `lo-table-rtl.docx`. |
| CORPUS-003: an unchanged RFP save reverses checklist cells and expands 22 pages to 32. | Fixed. Mixed raw/modeled table siblings retain order; reference saves return to 22 pages. | Pinned `nz-template-rfp-government-model.docx`; small samples and checks in [corpus fixes](FIXES-corpus.md). |
| CORPUS-009 / UI-007: hidden sheets appear in navigation and can be the initial canvas. | Hidden entries are omitted and startup selects a visible sheet. Restoring the producer's exact visible active-tab preference remains unverified. | [Hidden-first workbook](samples/S2-xlsx-hidden-first.xlsx); [suite](ui/hidden-sheets.mjs). |
| CORPUS-012: traditional and zodiac CJK list markers become decimal numbers. | Fixed for these two formats. | Pinned `lo-cjklist30.docx` and `lo-cjklist31.docx`; [corpus fixes](FIXES-corpus.md). |
| UI-006: malformed font names poison the WASM editor through a throwing host callback. | Fixed with valid font fallback and nonthrowing font loading. | [Word](samples/S2-docx-css-font-names.docx), [slides](samples/S2-pptx-css-font-names.pptx), [workbook](samples/S2-xlsx-css-font-names.xlsx); [save regression](ui/css-fonts.mjs). |
| NATIVE-004: read-only files are replaced and symlink saves destroy the link. | Fixed. Broken symlinks are refused. | Rust `refuses_read_only_files`, `saves_through_symlinks`, and `refuses_to_replace_a_broken_symlink` create the samples. |
| NATIVE-005: failed writes leave partial temporary files and unclear errors. | Fixed for tested partial-write/replacement failures. A real mounted-full-filesystem test was not run. | Rust `partial_write_failure_keeps_original_and_removes_temp`; [native report](BUGS-native.md). |
| NATIVE-006: Windows canonical paths defeat duplicate-tab matching. | Verbatim-prefix normalization implemented and unit tested. Full Windows path/association behavior remains unverified. | Rust `cli::tests::drops_the_verbatim_prefix`. |
| UI-008: a malformed percent escape breaks Markdown image loading. | Fixed. | [Markdown sample](samples/S2-markdown-percent-path.md). |
| UI-009: a zero-byte Office file opens as a new document. | Fixed. Existing empty Office files are refused; empty CSV remains valid. | [Empty Word](samples/S2-empty.docx), [empty slides](samples/S2-empty.pptx), [empty workbook](samples/S2-empty.xlsx). |
| Recovered core fix: packages with backslash entry names fail to open. | Fixed. | [Backslash package](samples/S2-backslash-part-names.xlsx). |
| Recovered core fix: encrypted Office files have an unhelpful legacy-format error. | Explanation fixed; encrypted Office editing remains unsupported. | [Encrypted workbook](samples/S2-password.xlsx), [encrypted Word](samples/S2-password-protected.docx). |
| Recovered core fix: Word tracked-grid metadata disappears during saves. | Fixed. | [Tracked table grid](samples/S2-docx-table-grid-change-lost.docx). |

## S3

| Finding | Final status | Reproducible sample or test |
| --- | --- | --- |
| CORPUS-013: saves drop unknown PowerPoint table-column IDs. | Fixed with original grid/column metadata preservation. | Pinned NASA SEWP decks in the manifest; core regression `table_grids_keep_what_their_columns_carry`. |
| UI-010: a zero-page PDF gives no useful explanation. | Fixed. | [Zero-page PDF](samples/S2-pdf-no-pages.pdf); [suite](ui/nasty.mjs). |
| UI-011: damaged images show a browser decode exception. | Fixed with a clear error. | [Damaged PNG](samples/S2-random.png), [empty PNG](samples/S2-empty.png). |
| NATIVE-007: Linux launcher lacks an explicit multiple-file placeholder. | Fixed; controlled GIO forwarding test passes. Packaged app launch remains unverified. | [GIO test](native/desktop-open.py), launcher template `src-tauri/linux/main.desktop`. |

## Evidence and practical limits

See [core results](BUGS-core.md), [UI results](BUGS-ui.md), [native/PDF results](BUGS-native.md), [baseline corpus](BUGS-corpus.md), and [corpus fixes](FIXES-corpus.md). Baseline observations and follow-up results must not be added together as independent full-corpus passes.

No release, tag, package repository, updater feed, or version bump was made. Heavy test workloads ran on bbb; local tests ran one workload at a time. Actual macOS/Windows application behavior, cloud synchronization, full-disk mounts, installer upgrades, native crash recovery, IME/paste interoperability, sustained memory soaks, and physical multi-monitor testing remain outside verified coverage. The 128 MiB XML limit can refuse legitimate large files. These limits are part of the result.
