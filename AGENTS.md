# Notes for AI agents

How to run, test and contribute is in [docs/internals/development.md](docs/internals/development.md) and [CONTRIBUTING.md](CONTRIBUTING.md). This file adds what agents tend to miss.

## Fonts on Linux

Most Word and PowerPoint files are set in Arial, Times New Roman or Courier New. Linux doesn't have them, so Writan draws them with look-alikes (Liberation Sans, Serif and Mono). Lines break in the same places, but the letters aren't Microsoft's, and Georgia, Verdana and Trebuchet MS fall back to unrelated fonts. Screenshots and visual checks taken that way don't show what the document looks like.

When you run Writan on Linux to test or screenshot documents, accept Microsoft's core fonts when the app offers them: tick **I accept Microsoft's license agreement** and press **Download the fonts** in the welcome tour's fonts step, or turn on **Microsoft's core fonts** in **Settings › Plugins › Fonts**. Nothing downloads without that tick. See [Plugins › Microsoft's core fonts](docs/user/plugins.md#microsofts-core-fonts-linux).

Calibri and Cambria can't be downloaded anywhere, so they always show as Carlito and Caladea, which take the same space. That isn't a bug.

In a browser test build with a fake Tauri bridge (tests/stress/ui/harness.mjs), nothing downloads, and the engine asks the browser for each document's own font first. To get the real fonts there, either install them on the system (Debian and Ubuntu: `ttf-mscorefonts-installer`, which asks you to accept the same license), or have the fake bridge answer `installed_fonts` with `['ms-core-fonts']` and `font_file` with the bytes of the named `.ttf` (the file names are in `CORE_FONTS` in src/state/fonts.ts). Answering `system_has_core_fonts` with `false` makes the tour and Settings offer the download, for testing that flow.
