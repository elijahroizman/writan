<img src="assets/app-icon.svg" width="72" height="72" alt="">

# Writan

Writan opens Word documents, PowerPoint presentations, Excel workbooks, CSV, PDF, Markdown, plain text and images, each in a tab of one window. An optional plugin adds LaTeX. It runs on macOS, Windows and Linux. Your documents stay on your computer unless you share them, and there's no account to make.

## Why it exists

Word, PowerPoint and Preview are slow to open and heavy to keep running, and most people only need them to read and change a few kinds of file. Writan opens those files quickly, in one window, on your own computer. It's built on [Tauri](https://tauri.app), so it uses the system's own web view instead of shipping a browser, and the installer is about 25 MB. Each document engine loads the first time you open that kind of file, so someone who only writes Markdown never loads the Office engine.

## What it does

- **Live editing together.** Share a Word document, presentation or workbook, and others join with a six-digit code from anywhere. Everyone sees everyone's changes, carets and selections as they type. The file stays on the computer of the person who shared it, and what travels over the internet is encrypted.
- **AI agents in your documents.** Claude Code, Codex, Antigravity, Grok Build, Hermes Agent and Pi can join a document you have open and edit it beside you, through Writan's MCP server. An agent writes and formats paragraphs, builds slides, fills in cells and charts, and looks at pages and slides as pictures to check its work. Its caret and name show like a person's. **Settings › AI** adds Writan to each of those agents in one click, and any other agent that takes a remote MCP server can use the address it shows. Agents in the same document talk in the document's chat to split up the work.
- **Its own Office engine.** Word, PowerPoint and Excel files open in an engine written in Rust in this repository ([crates](crates)) and compiled to WebAssembly. It keeps the parts of a file it doesn't edit, so saving doesn't drop them, and it works out about 140 Excel functions itself.
- **PDF tools.** Edit a PDF as a Word document, sign it with a signature you draw, type or photograph, fill in its forms, and move, rotate, insert and extract pages. Writan reads scanned PDFs and pictures with offline OCR, so Find works on them, and nothing is sent anywhere to do it.
- **LaTeX.** The optional plugin compiles `.tex` files with the TeX on your computer and shows the PDF beside the source. If you don't have TeX, Writan offers to install it.

It also has autosave, version history, split view, a command palette and color themes. [What each editor can do](docs/user/features.md) has the full list.

## Install

Download the latest build from [Releases](https://github.com/elijahroizman/writan/releases/latest).

| | |
| --- | --- |
| Mac | Open `Writan_…_aarch64.dmg` for Apple silicon, M1 and later, or `Writan_…_x64.dmg` for Intel, and drag Writan to Applications. |
| Windows | Run `Writan_…_x64-setup.exe`. The Windows app isn't code-signed yet, so if Windows says it protected your PC, click **More info**, then **Run anyway**. |
| Debian, Ubuntu | `sudo apt install ./Writan_…_amd64.deb`, or `Writan_…_arm64.deb` on ARM. Needs Ubuntu 22.04 or Debian 12 or newer. |
| Fedora | `sudo dnf install ./Writan-…x86_64.rpm`, or `Writan-…aarch64.rpm` on ARM, such as Asahi. |
| Arch Linux, Omarchy | `sudo pacman -U ./writan-…-x86_64.pkg.tar.zst`, or `writan-…-aarch64.pkg.tar.zst` on ARM, such as Arch Linux ARM or Asahi. |

On Linux the package and the terminal command are both called `writan`.

## Updates

Writan keeps itself up to date. On a Mac or a Windows PC, new versions download in the background and install when you quit. On Linux, the package adds Writan's APT, dnf or pacman repository, so new versions arrive with your system's other updates. There are two channels: stable, which gets each release, and nightly, which gets builds of the newest code before they're released. [Updating](docs/user/updating.md) explains both and how to switch.

## Privacy

Writan sends anonymous usage data, and you can turn it off. [PRIVACY.md](PRIVACY.md) lists what it sends and what it never sends.

## Documentation

- [What each editor can do](docs/user/features.md)
- [Keyboard shortcuts](docs/user/keyboard-shortcuts.md)
- [Plugins: images and LaTeX](docs/user/plugins.md)
- [Updating](docs/user/updating.md)
- [What it doesn't do yet](docs/user/limitations.md)
- [Changelog](CHANGELOG.md)

To work on the code, start with [development](docs/internals/development.md), which covers building from source. [Contributing](CONTRIBUTING.md) has the rules for pull requests, [releases and signing](docs/internals/releasing.md) covers how a version ships, and [writing a plugin](plugins/README.md) covers plugins. The rest is in the [docs index](docs/README.md).

## Thanks

Writan is built on open-source engines and fonts: [PDFium](https://pdfium.googlesource.com/pdfium/) through [EmbedPDF](https://github.com/embedpdf/embed-pdf-viewer), [CodeMirror](https://codemirror.net), [markdown-it](https://github.com/markdown-it/markdown-it), the Carlito, Caladea and Liberation fonts, and many more. **Settings › General › Open-source licenses** lists every one with its license. Writan's themes follow [T3 Code](https://github.com/pingdotgg/t3code)'s, by T3 Tools Inc.: the theme picker, the theme editor and its color picker, custom CSS and how it's kept from loading files, the Open VSX theme search, and the tools that let an agent restyle the app. Parts of that code are adapted from T3 Code under its MIT license ([assets/third-party/T3-Code-LICENSE.txt](assets/third-party/T3-Code-LICENSE.txt)).
