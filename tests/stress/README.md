# Writan stress regression suite

This suite began on 2026-10-24. Tests target data loss first, then malformed inputs and visual fidelity. No release was made. Reports distinguish baseline observations, verified fixes, open findings, and work that needs native hardware.

Start with the severity-sorted findings in [BUGS.md](BUGS.md) and the [final verification](VERIFICATION.md). Detailed evidence and reproduction commands are in:

- [Native saves and PDF](BUGS-native.md), with [browser checks](native/README.md).
- [Office compiler and editing engines](BUGS-core.md).
- [Editors and shell](BUGS-ui.md).
- [Real document corpus](BUGS-corpus.md), with [fetch and test instructions](corpus/README.md). This is the original baseline report; the consolidated list records subsequent fixes.
- [Corpus fix verification](FIXES-corpus.md).

Small reproductions are committed under `samples/`. Environment-dependent cases create their own files in Rust tests or embed a fake filesystem in a browser script. Large or third-party samples have generator scripts or a URL and SHA-256 in `corpus/manifest.tsv`. The fetcher verifies those hashes. Full corpus originals and reference renders stay on bbb rather than entering the repository without redistribution permission.

## Run the checks

The native test workload runs locally. Keep it sequential on the smaller machine:

```sh
cargo test --manifest-path src-tauri/Cargo.toml -- --test-threads=1
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

The curated browser regressions run locally and in CI with pinned Playwright and headless Chromium. Install Node 22, Rust's `wasm32-unknown-unknown` target and `wasm-bindgen-cli` 0.2.129 first:

```sh
npm ci
npx playwright install --with-deps chromium
VITE_TEST_HOOKS=1 npm run build
npm run test:browser
```

The runner owns a preview server on `127.0.0.1:4173`; leave that port free. It runs CSS-font safety, data safety, hidden sheets, malformed files, security, slide clipboard and save conflicts sequentially, with a 180-second timeout per script and a nonzero exit on failure. The curated list and exclusion reasons live in `scripts/test-browser.mjs`. Session recovery observations and the external signed-PDF test remain manual; follow the component reports for those commands and fixtures.

CI checks changed components on pull requests, and everything on `main` or manual runs: Office formatting/Clippy/tests/WASM builds and LibreOffice samples, native Linux Clippy/tests, and the frontend build/typecheck/browser suite. Engine changes also trigger the frontend. Run `cargo test --workspace --all-features` in `crates/` for the Office tests locally.

The native browser scripts fake IPC for save behavior; the PDF script uses the real PDF worker. The corpus scripts compare ZIP parts, formula caches, signature validation, text, page counts, and reference renders. Opening without an exception is not a fidelity pass.

## Coverage limits

Chromium and WebKit tests exercise browser builds. They do not establish macOS Dock/logout behavior, Windows locks/file associations, actual cloud sync, physical display configurations, or an installer upgrade. Tests did not modify updater feeds or package repositories. Full-disk cleanup has a deterministic partial-write test; a real mounted-full-filesystem test remains ignored because this host disallows the required namespace mount. Unsupported rendering and crash recovery remain explicit findings, not passes.

When copying sources from a machine with an earlier clock, refresh Rust source mtimes on the build host before invoking Cargo. The engine script normally uses mtimes to skip builds, so use `--force` for the final regression build and verify Cargo actually recompiles the changed crates. This pass observed clock skew between the local host and bbb.
