# UI stress findings

Integration update: the combined `stress-test` branch also fixes UI-006 with a nonthrowing font host and UI-010 with an explicit zero-page PDF error. Their failing observations below describe the delegate branch before those follow-ups. The final status is in [BUGS.md](BUGS.md). The combined save flow retains both per-tab and per-path serialization, including queued Save As destinations.

This report covers branch `t3/ui-stress-test-bugfixes`, including the four inherited UI commits `0adedb2`, `a0d664b`, `f8e88bb`, and `35aa2cb`. Findings are sorted by severity. Fixed means verified on this UI branch. Core fixes being developed on the parent branch are outside this result.

All builds and tests ran on `eli@bbb` in `~/stress-ui`. No local test workload, release, version bump, push, or PR was used. Browser automation used the existing remote Playwright installation because the requested test environment is bbb. The Browser plugin is not available. Chromium and WebKit ran at 1400 by 900 CSS pixels. WebKit approximates WKWebView; it does not test the actual macOS application.

The fake Tauri bridge exercises the real frontend, WASM engines, dialogs, save orchestration, and window-close handler. Reads use real sample bytes. Writes stay in a per-page memory overlay. It does not prove native filesystem atomicity, native dialogs, process termination, or operating-system crash recovery. Native save/PDF regressions belong to the parent agent.

## High severity

### UI-001. Fixed: unsaved edits and untitled tabs do not survive renderer reload

Sample: `tests/stress/samples/S1-text-utf16le-bom.txt` and an untitled text tab created during the test.

Repro: disable autosave, open the sample, append `UNSAVED FILE MARKER`, create an untitled text tab containing `UNTITLED RECOVERY MARKER`, and reload without calling the normal quit handler. Run `ENGINE=chromium node tests/stress/ui/session.mjs`, then repeat with `ENGINE=webkit`.

Previously the file path reopened with its original disk contents, its unsaved marker gone, and the untitled tab was gone: `persistSession()` stored only `paths` and `active`.

`src/lib/recovery.ts` now keeps a copy of every unsaved tab's document in IndexedDB (`writan-recovery`), a second after typing stops and at least every 10 seconds while it continues. It stores the bytes, kind, title, path, tab position, and the digest of the file as the tab last read or wrote it. On launch, `restoreSession()` reopens the saved paths and then puts each kept document back: into the reopened tab for its path, or into a new tab at its old position (untitled, or a file that has since been deleted, which then saves as new). A recovered tab stays unsaved until it is saved, even after undoing back to what was recovered. Saving compares the file against the kept digest, so a file changed on disk since the edits started asks before it is overwritten. Saving, reverting, or closing a tab with **Don't Save** drops its copy, and so does a quit after every unsaved tab is saved or discarded; a cancelled quit or failed update restart keeps recovery going. Documents of a disabled plugin stay stored until it is enabled again.

The test asserts both documents come back unsaved with their text, UTF-16 decoding, tab order and active tab, that nothing is written to disk before a save, that undo keeps a recovered tab unsaved, the changed-on-disk prompt, and that save, **Don't Save** and quit each drop the kept copy. A renderer reload is a bounded crash-equivalent observation, not a simulated kernel crash or real native kill.

The normal quit path is unchanged. `datasafety.mjs` verifies that closing a changed tab or requesting window close prompts, Cancel keeps the work, and a failed save from the prompt keeps the tab.

### UI-002. Fixed: opening and saving non-UTF-8 text corrupts existing content

Samples: `S1-text-utf16le-bom.txt`, `S1-text-utf16be-bom.txt`, `S1-text-windows1252.txt`, `S1-markdown-latin1.md`, `S2-text-utf8-bom.txt`, `S2-text-double-bom.txt`, and `S2-markdown-utf16le.md` under `tests/stress/samples/`.

Repro: open a sample, save without editing, append `Z`, save, undo, and save again. Previously UTF-8-only decoding replaced non-UTF-8 characters; saving wrote those replacements. A UTF-8 BOM was also discarded. Run `node tests/stress/ui/datasafety.mjs` in each engine.

`src/lib/textcodec.ts` now detects BOM-marked UTF-16 LE/BE and UTF-8, validates unmarked UTF-8, and uses Windows-1252 as the fallback. Both text and Markdown editors retain that encoding. A character outside Windows-1252, tested with Japanese text and an emoji, upgrades the output to UTF-8 instead of dropping the character. A second U+FEFF after the BOM remains document content. All tested untouched, append, and undo results are compared byte for byte.

Mixed, CRLF, and CR-only line endings also survive unchanged regions and undo. Samples: `S3-text-mixed-eol.txt`, `S2-text-cr.txt`, and `S2-markdown-crlf.md`. The shared text model composes CodeMirror changes against the original text, retains existing line endings outside replacements, and gives inserted lines the first existing line-ending style. No-edit, append, line insertion, line deletion, and undo are verified.

Limits: unmarked legacy encodings cannot be identified reliably. This fallback does not promise correct interpretation of Shift-JIS, UTF-16 without a BOM, or arbitrary legacy character sets. A follow-up keeps the original byte buffer for unchanged revisions, including malformed BOM-marked UTF-8 and truncated UTF-16. Two committed samples verify no-edit and undo byte preservation in both browsers. Saving actual edits to malformed input still serializes the decoded text and can replace invalid byte sequences. CSV has its own importer and is not covered by the textcodec fix.

### UI-003. Fixed: stale tabs and external edits can be overwritten silently

Sample: `tests/stress/samples/S3-text-mixed-eol.txt`. Run `node tests/stress/ui/datasafety.mjs` in both engines.

Repro: duplicate the tab, change and save A, then change and save the older B. Previously B replaced A without a conflict prompt. Also replace a file externally with different bytes of identical size and timestamp, or start both duplicate saves concurrently.

The workspace remembers opened bytes and a digest after a successful save. Before overwriting the same path it reads and compares current content. Cancel preserves the on-disk version and the unsaved tab. An explicit overwrite saves the selected copy. Quiet saves reject conflicts without a dialog or write. A failed conflict read aborts saving; external deletion requires confirmation before recreation. Per-path serialization keeps two duplicate tabs from both validating the same old contents before either writes.

Both engines verify the cases above, failed writes, close-after-failed-save, and undo while a delayed save is in flight. Text and Markdown expose content revisions so undoing to the old clean text during a write cannot mark a different disk snapshot as saved.

Limits: this is frontend conflict detection. A different process may still write between the final read and the native write. Locks use the path string, so symlink/case aliases and separate application processes are not serialized here. The parent's native changes and same-tab save queue must be retained during integration. Version-history restoration has not been added to this lock protocol.

Integration note: `datasafety.mjs` asserts exact buttons `Cancel`, `Save`, `Save Anyway`, and `OK`. Conflict messages contain `changed on disk`. If the parent retains its `Replace` label, update the three `Save Anyway` assertions. This branch does not merge or replace the parent's same-tab queue.

### UI-004. Fixed in inherited commits: renamed Office files become blank editable documents

Samples: `tests/stress/samples/S2-docx-renamed.pptx` and `S2-docx-renamed.xlsx`.

Repro: open either sample. Previously a Word document renamed to a slide/workbook extension could become a blank deck or workbook, making later saves destructive. `expectKind()` in `src/engine/load.ts` now refuses the mismatched Office type with the correct extension in its message. `nasty.mjs` verifies the specific explanation, no writable editor, no write, and continued editing in another tab.

### UI-005. Fixed in inherited commit: document links accept unsupported external schemes

Samples: `S1-docx-evil-links.docx`, `S1-pptx-evil-links.pptx`, `S1-pdf-evil-links.pdf`, and `S1-markdown-xss.md` under `tests/stress/samples/`.

Run `node tests/stress/ui/security.mjs` in each engine. The script clicks document links, checks that only HTTP, HTTPS, and mailto reach the opener, verifies the allowed Word link still opens, checks the refusal dialogs, and verifies no script execution, unhandled page errors, or navigation away from the application. The PDF step rendered no clickable annotation anchors for this hand-written fixture in these runs, so its empty opener result does not prove PDF link interaction; that remains a coverage gap. Word and PowerPoint have positive allowed-link evidence. HTML-like filenames and cell contents remain text. This is an interaction regression suite, not a repository security scan.

## Medium severity

### UI-006. Open on this branch: hostile font names cause WASM borrow errors

Samples: `tests/stress/samples/S2-docx-css-font-names.docx`, `S2-pptx-css-font-names.pptx`, and `S2-xlsx-css-font-names.xlsx`.

Run `node tests/stress/ui/css-fonts.mjs` in each engine. The app chrome stays visible and the document does not inject global CSS rules. However, both browser runs log `recursive use of an object detected which would lead to unsafe aliasing in rust` and `attempted to take ownership of Rust value while it was borrowed`. The suite now explicitly fails on those errors instead of reporting a clean pass merely because no `pageerror` fired. Preserve the samples for the core delegate and recheck against the combined branch. No Rust change was made here.

### UI-007. Fixed: a hidden worksheet can be the initial visible canvas

Sample: `tests/stress/samples/S2-xlsx-hidden-first.xlsx`, containing a hidden first sheet, a very-hidden second sheet, and a visible third sheet.

Run `node tests/stress/ui/hidden-sheets.mjs` in each engine. `SheetsEngine.open()` now selects a visible sheet if the imported active sheet is hidden. Sheet navigation omits hidden entries while keeping their original indices. The test verifies imported hidden flags, the visible initial selection, absence of hidden names in navigation, interaction with the visible sheet, and a clean document state after selecting it.

This is the UI follow-up to CORPUS-009. It does not repair the core serialization of hidden/very-hidden distinctions or create an unhide workflow. Files with every sheet hidden are invalid Excel inputs and are outside this test.

### UI-008. Fixed in inherited commit: a malformed percent escape breaks Markdown images

Sample: `tests/stress/samples/S2-markdown-percent-path.md`. Run `node tests/stress/ui/nasty.mjs markdown-percent` in each engine.

Opening an image path containing a stray percent sign previously threw during URI decoding and broke the editor. The guarded path decoder keeps the Markdown tab usable. Missing image requests in this fixture are expected; an unhandled application error is not.

### UI-009. Fixed: zero-byte Office input is treated as a new document

Samples: `tests/stress/samples/S2-empty.docx`, `S2-empty.pptx`, and `S2-empty.xlsx`. Run `node tests/stress/ui/nasty.mjs empty` in each engine.

A zero-byte input previously reached constructors that treat empty bytes as a new document. The frontend now distinguishes an existing empty Office file from a new untitled document and reports that it contains no Office document. Empty CSV remains allowed. The stress suite requires a refusal and verifies that opening never writes the sample.

## Low severity

### UI-010. Open: a zero-page PDF has no useful empty/error state

Sample: `tests/stress/samples/S2-pdf-no-pages.pdf`, a PDF whose page tree contains zero pages.

Run `node tests/stress/ui/nasty.mjs pdf-no-pages` in each engine. The PDF opens without a page or a useful explanation. The stricter suite waits up to 15 seconds, reports `loading`, and fails its nonblank-result assertion. This result does not mean the engine is still loading; it means the UI provided neither a rendered page nor an explanatory message. Other tabs still respond. The fixture and screenshot distinguish this from an app crash.

### UI-011. Fixed in inherited commit: damaged images show a browser decode exception

Samples: `tests/stress/samples/S2-random.png` and `S2-empty.png`. Run `node tests/stress/ui/nasty.mjs .png` in each engine.

The bundled Images plugin now reports that the file is damaged or not a supported picture. The suite requires a message, no writable image editor, no write, and a responsive existing text tab.

## Verification and reproduction

The final verification uses fresh WASM. Remote clock skew can make preserved rsync mtimes older than existing outputs. On bbb, Rust sources in this isolated workspace were touched and `node scripts/engine.mjs --force` was run. The log shows recompilation of office-core, office-engine, office-slides, office-sheets, office-word, and office-wasm, followed by wasm-bindgen output. No core source was edited by this UI task.

From the isolated checkout on bbb:

```sh
export PATH="$HOME/.local/node24/bin:$HOME/.cargo/bin:$PATH"
cd ~/stress-ui
npm run typecheck
rg --files crates -g '*.rs' -g Cargo.toml -g Cargo.lock | xargs touch
node scripts/engine.mjs --force
VITE_TEST_HOOKS=1 npm run build
# Keep one preview server for this workspace on port 1420.
npm run preview -- --port 1420 --strictPort
```

In another remote shell, with the same PATH and checkout:

```sh
for engine in chromium webkit; do
  for suite in datasafety session hidden-sheets nasty security css-fonts; do
    ENGINE="$engine" timeout 150 node "tests/stress/ui/$suite.mjs"
  done
done
```

`WRITAN_URL` overrides the default `http://localhost:1420/`. The server must have been built with `VITE_TEST_HOOKS=1`. The test bridge needs the existing Playwright links in node_modules, not a local browser workload.

Samples are committed. Regenerate with `~/ws051-venv/bin/python tests/stress/gen/ui-samples.py`. The generator was renamed from `gen/samples.py` to avoid the core delegate's path. It requires python-docx, python-pptx, and openpyxl. Authentic password fixtures additionally use msoffcrypto and Ghostscript when installed. Random corrupt bytes use a seeded generator; Office ZIP metadata/encryption may vary on regeneration.

Final per-suite logs are `/tmp/writan-ui-final-<suite>-<engine>.log` on bbb. Build logs are `/tmp/writan-ui-build.log` and `/tmp/writan-ui-engine-force.log`. Screenshots are under `/tmp/writan-stress/`, including `datasafety-<engine>.png`, `session-<engine>.png`, `hidden-sheets-<engine>.png`, `css-final-<engine>.png`, and `nasty-S2-pdf-no-pages.pdf.png`. They are evidence artifacts, not committed application assets.

Final assertion counts per engine:

- Data safety: 97 pass, 0 fail.
- Session recovery: all pass on Chromium.
- Hidden sheets: 8 pass, 0 fail.
- Malformed files: 113 pass, 1 known failure for UI-010.
- Document links/content: 18 pass, 0 fail, with the PDF interaction gap described above.
- CSS font names: 10 pass, 1 known failure for UI-006.

Typecheck covers both src and plugins. The build includes the bundled Images plugin. The data-safety, session-recovery, hidden-sheet, and document-link suites pass on both engines. The malformed-file suite has the known zero-page-PDF failure. The CSS-font suite retains the known core-borrow failure. These failures must remain visible in the combined report rather than being described as a clean stress run.

Known coverage limits: no exhaustive large-file or low-memory soak, IME/composition tests, real native kill/restart, actual macOS WKWebView, filesystem aliases, other processes writing in the final save window, or full Office/CSV/PDF import/export fidelity. Password-protected Office documents remain unsupported and now explain that limitation. The PDF password fixtures test the prompt, not unlock/edit/save. Broader corpus fidelity and native filesystem/PDF changes are owned by the other delegates.
