# Development

Setup for each system is in the [README](../../README.md#run-it-locally). This page covers the rest. AI agents: also read [AGENTS.md](../../AGENTS.md), which explains why documents on Linux show look-alike fonts.

## Commands

```sh
npm run dev            # UI in a browser at http://localhost:1420
npm run app:dev        # the app, with hot reload
npm run typecheck      # tsc-rs, a Rust port of the TypeScript compiler
npm run build          # licenses, type check, production build into dist/
npm run plugins        # builds the optional plugins into plugins/<id>/dist
cd src-tauri && cargo test
cd crates && cargo test     # the office compiler
```

Type checking uses [tsc-rs](https://github.com/pingdotgg/ts-rust), not Microsoft's `tsc`. In November 2026 it was the fastest checker here on 8 cores: checking the app and the plugins took 0.19 s with tsc-rs 0.8.0, 0.22 s with [tsrs](https://github.com/oxc-project/tsrs), 0.23 s with [tsc-ts](https://github.com/vercel-labs/tsc-ts) and 0.50 s with TypeScript 7.1 beta. On 4 cores tsc-rs and tsrs tie, and on 2 cores tsrs is faster.

## Testing in a browser

In browser mode, `http://localhost:1420/?open=/dev-samples/File.pdf` opens a file from `dev-samples/` (git-ignored) without a dialog.

A production build is lighter than the dev server and exposes test hooks (`window.__workspace`, the store):

```sh
VITE_TEST_HOOKS=1 npx vite build
ln -s ../dev-samples dist/dev-samples
npx vite preview --port 1420
```

Playwright can drive that build. To test code that only runs in the app (saving, autosave), fake `window.__TAURI_INTERNALS__` in `addInitScript`. Test builds also accept `window.__texFake` in place of the system's TeX, and `window.__fsFake` (`{ list(path), isDir(path) }`) as the file system behind Open Folder.

## Native code

Rust lives in `src-tauri/`. `cargo check` there checks the code for the system you're on. Some tests reach the network or need TeX and are marked `#[ignore]`; run them with `cargo test -- --ignored`.

Mac and Windows build on case-insensitive file systems, so never add two paths that differ only in case.

## Where things are

```
src/
  App.tsx                 shell: tabs, menu commands, session restore, drag and drop
  state/                  open tabs and saving, settings, plugins, TeX
  lib/platform.ts         native file dialogs and file I/O (browser fallbacks for dev)
  lib/dialogs.ts          the app's own questions and alerts (components/Dialogs.tsx)
  lib/updater.ts          update checks, the Tauri updater, and apt on Linux
  components/             title bar and tabs, Home, the welcome tour (Setup.tsx), Settings, the three-pane workbench; ui.tsx is the design system the rest are built from
  editors/{docx,pptx,pdf,markdown,text}/   built-in editors, each loaded the first time it's needed
  collab/                 sharing a document for editing together (see collaboration.md)
  collab/agents/          AI agents in shared documents: their tools, the MCP bridge, names and logos, the skill
  plugins/catalog.ts      every plugin and the file types it opens
  plugins/api.ts          the contract an optional plugin's editor implements
plugins/<id>/             plugin bundles (Images ships in the app; LaTeX is downloaded)
scripts/licenses.mjs      collects license texts for Settings (runs in npm run build)
src-tauri/
  src/lib.rs              app setup, Open With handling
  src/plugins.rs          downloads, checks and serves optional plugins (wsplugin: scheme)
  src/tex.rs              finds, runs, installs and removes the system's TeX
  src/update.rs           the latest release's notes, Mac relaunch, apt updates on Linux
  windows/hooks.nsh       closes Writan before a Windows install or uninstall
  src/defaults.rs         makes Writan the default app for its file types
  src/collab/             the relay that orders shared documents' changes, its WebSocket server and client, and the MCP server for agents (mcp.rs)
  src/agents.rs           sets up Writan's MCP server and skill in Claude Code, Codex, Antigravity, Grok Build and Hermes
  src/files.rs            raw-bytes file reading and atomic saving
  src/menu.rs             the native menu bar
  linux/                  the .deb's install scripts and APT signing key
crates/                   the office compiler and engine: Rust, compiled to WebAssembly for the Word, PowerPoint and Excel editors (see office-compiler.md and office-engine.md)
```

## The icon

`assets/app-icon.svg` is the source of every app icon. After changing it, regenerate the sizes the app uses (the command also makes mobile and Windows Store sizes; leave those out):

```sh
npx tauri icon assets/app-icon.svg -o /tmp/icons
cp /tmp/icons/{32x32.png,128x128.png,128x128@2x.png,icon.icns,icon.ico,icon.png} src-tauri/icons/
```

`public/favicon.svg` is the same mark, full bleed for small sizes, used by the browser version.

The runtime choices in Settings use `Logo` in `src/components/ui.tsx` as their source. After changing its artwork, run `node scripts/app-icons.mjs` to regenerate `src-tauri/icons/variants/*.png`. The script needs the npm dependencies; it renders with the Tauri CLI, as for the bundled icons, and puts each tile on the bundled icon's grid with its margin and shadow, so the Dock and taskbar show every choice at the same size. These variants change the running app's icon; the bundled icon stays the default.

## The welcome tour's pictures

`src/components/tour/*.webp` are screenshots of the app, in light and dark, opened on the sample documents in `scripts/tour/`. The palette's comes twice, with Mac and with Windows/Linux shortcuts. After a change that shows in them, serve a test build and run the script (it needs Playwright, which isn't a dependency: `npm i --no-save playwright`):

```sh
VITE_TEST_HOOKS=1 npx vite build && npx vite preview --port 1420
node scripts/tour-shots.mjs http://localhost:1420/          # every picture
node scripts/tour-shots.mjs http://localhost:1420/ palette  # just some
```

## Windows installer hooks

On Linux with `makensis` installed, run `python3 scripts/check-windows-hooks.py`. This compiles the installer and uninstaller hooks against NSIS's real Restart Manager, LogicLib and FileFunc includes. The harness substitutes Tauri's `CheckIfAppIsRunning` macro, which otherwise needs its Windows plugin DLL. It checks the hook code, not a complete installer or a running Windows upgrade.

Release outputs are `Writan.app`, `Writan_<version>_universal.dmg`, `Writan_<version>_x64-setup.exe`, `Writan_<version>_amd64.deb`, `Writan_<version>_arm64.deb`, `Writan-<version>-1.x86_64.rpm`, `Writan-<version>-1.aarch64.rpm`, `writan-<version>-1-x86_64.pkg.tar.zst` and `writan-<version>-1-aarch64.pkg.tar.zst`. The Linux package name is `writan`. See [the rename upgrade paths](releasing.md#rename-to-writan-0100).
