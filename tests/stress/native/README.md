# Native-side stress tests

Playwright scripts that fake the Tauri backend (`window.__TAURI_INTERNALS__`) to test what the app does around the file system. Run them against a test build, with Playwright available (`npm i --no-save playwright`):

```bash
set -e
VITE_TEST_HOOKS=1 npx vite build
# Use a free port so readiness cannot come from an older preview.
if curl -fsS http://127.0.0.1:4173/ >/dev/null 2>&1; then
  echo 'Port 4173 is already in use' >&2
  exit 1
fi
node node_modules/vite/bin/vite.js preview --host 127.0.0.1 --port 4173 --strictPort >/tmp/writan-native-preview.log 2>&1 &
preview_pid=$!
trap 'kill "$preview_pid" 2>/dev/null || true; wait "$preview_pid" 2>/dev/null || true' EXIT
for ((attempt = 0; attempt < 100; attempt++)); do
  kill -0 "$preview_pid"
  if curl -fsS http://127.0.0.1:4173/ >/dev/null 2>&1; then break; fi
  sleep 0.2
done
curl -fsS http://127.0.0.1:4173/ >/dev/null
node tests/stress/native/save-conflict.mjs http://localhost:4173/ chromium
node tests/stress/native/save-conflict.mjs http://localhost:4173/ webkit
```

Run this in a dedicated Bash shell. Run any PDF checks below in that same shell, then exit it to stop the preview. The exit trap also stops it if a check fails.

- `save-conflict.mjs`: a file another app changed after Writan opened it isn't autosaved over, and Save asks before replacing it.

The Rust side (atomic saves, read-only files, symlinks, version history of outside changes) is covered by `cargo test` in `src-tauri`.

- `pdf-unchanged.mjs`: unchanged PDF saves preserve the original bytes, undo restores those bytes, edits persist, and repeated clean saves preserve the saved revision. Fetch the signed GPO PDF using `../corpus/manifest.tsv` and serve it at `/dev-samples/us-budget-2025-appendix.pdf`:

```sh
node tests/stress/native/pdf-unchanged.mjs http://localhost:4173/ chromium
node tests/stress/native/pdf-unchanged.mjs http://localhost:4173/ webkit
pdfsig /tmp/writan-pdf-chromium-unchanged.pdf
```

- `pdf-flatten.mjs`: Save flattened copy on a two-page PDF succeeds when a page has nothing to flatten, bakes highlights into the copy's pages (also shown ones saved without the Print flag or a drawing, but not hidden ones), and a cancelled export writes nothing:

```sh
node tests/stress/native/pdf-flatten.mjs http://localhost:4173/ chromium
node tests/stress/native/pdf-flatten.mjs http://localhost:4173/ webkit
```

The scripts use the repository's documented Playwright workflow on bbb. Browser plugin not available. The PDF script writes its saved revisions and screenshot under `/tmp` on the test host.

`files::tests::partial_write_failure_keeps_original_and_removes_temp` injects a full-disk error after a partial write. It runs without mounting a filesystem. To test real ENOSPC, provide an isolated tiny filesystem in `WRITAN_FULL_DIR` and run the ignored `full_disk_keeps_the_original_and_no_scraps` test. The `unshare` recipe in its source requires a host that permits user namespace mappings and mounts.

On Linux, `python3 tests/stress/native/desktop-open.py` checks the launcher template through GIO using a harmless argument recorder. It creates two sample Markdown files, including names with spaces and Unicode, and verifies that both are forwarded in one launch. It does not launch or install Writan.

## macOS slide clipboard smoke test

Open the packaged Writan app on macOS and create a blank presentation. Run the steps below twice on fresh slides: first use the system menu bar's **Edit > Copy**, **Cut**, and **Paste**, then use **⌘C**, **⌘X**, and **⌘V**. The browser clipboard test does not exercise the macOS menu.

1. Insert and select a shape. Copy and paste it, then move the selected duplicate to a visibly different position. Cut it: only the moved shape should disappear. Paste: it should reappear at the moved position, not on top of the original.
2. Enter text in a slide text box and select a word. Copy and paste it at the end of the text. Select a different word and cut it, then paste it back. Check that the cut removes only that word, and paste restores it rather than inserting the previously copied word.
3. Focus Speaker notes and repeat the text steps with different words. Check that the pasted text stays in the notes and the slide is unchanged.
