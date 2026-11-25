# Notes for AI agents

How to run, test and contribute is in [docs/internals/development.md](docs/internals/development.md) and [CONTRIBUTING.md](CONTRIBUTING.md). This file adds what agents tend to miss.

## Fonts on Linux

Most Word and PowerPoint files are set in Arial, Times New Roman or Courier New. Linux doesn't have them, so Writan draws them with look-alikes (Liberation Sans, Serif and Mono). Lines break in the same places, but the letters aren't Microsoft's, and Georgia, Verdana and Trebuchet MS fall back to unrelated fonts. Screenshots and visual checks taken that way don't show what the document looks like on a Mac or Windows PC. Installing Microsoft's fonts on the system itself (Debian and Ubuntu: `ttf-mscorefonts-installer`) makes documents use them.

Calibri and Cambria can't be downloaded anywhere, so they always show as Carlito and Caladea, which take the same space. That isn't a bug.

Writan's monospace text uses the system's `monospace` font. Without DejaVu (`fonts-dejavu-core`), Linux picks Nimbus Mono, a copy of Courier.

## Screenshots

Take screenshots at 2x. The person reading them is usually on a Retina Mac, where Writan draws every CSS pixel with four. Playwright and other headless browsers default to 1x. At 1x, body text at 78% zoom is 11 pixels tall, and on a Mac it looks faint and blurred. `launch()` in [tests/stress/ui/harness.mjs](tests/stress/ui/harness.mjs) opens the browser at 2x. In a script of your own, pass `deviceScaleFactor: 2` to `browser.newContext()`.

Crop each screenshot to the part it shows, such as the chat panel or one page. The harness's `shot(page, path, selector)` does that. The window has to fit the part: a Letter page at 100% is 1056 pixels tall, so it needs a viewport taller than the default 900. The whole window at 2x is 2800 pixels wide, and chat apps and models shrink a picture that big, which blurs it again.

A screenshot from Linux still differs from the Mac. FreeType draws letters thinner than macOS, and the interface shows in Inter, not San Francisco.
