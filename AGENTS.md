# Notes for AI agents

How to run, test and contribute is in [docs/internals/development.md](docs/internals/development.md) and [CONTRIBUTING.md](CONTRIBUTING.md). This file adds what agents tend to miss.

## Fonts on Linux

Most Word and PowerPoint files are set in Arial, Times New Roman or Courier New. Linux doesn't have them, so Writan draws them with look-alikes (Liberation Sans, Serif and Mono). Lines break in the same places, but the letters aren't Microsoft's, and Georgia, Verdana and Trebuchet MS fall back to unrelated fonts. Screenshots and visual checks taken that way don't show what the document looks like on a Mac or Windows PC. Installing Microsoft's fonts on the system itself (Debian and Ubuntu: `ttf-mscorefonts-installer`) makes documents use them.

Calibri and Cambria can't be downloaded anywhere, so they always show as Carlito and Caladea, which take the same space. That isn't a bug.
