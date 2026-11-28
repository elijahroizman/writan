# Notes for AI agents

How to run, test and contribute is in [docs/internals/development.md](docs/internals/development.md) and [CONTRIBUTING.md](CONTRIBUTING.md). This file adds the rules for agents and what agents tend to miss.

## Which models may open a pull request

Only these models may open a pull request: Opus, Fable, Sonnet, Sol, Astra, Muse Wonder Full, SWE and Neon.

- Any other model must not open a pull request.
- Neon orchestrator, the orchestrator variant of Neon, must not open one. Only plain Neon may.
- Muse Wonder Full is a model of its own. Other Muse models aren't on the list.
- If you aren't sure which model you are, don't open a pull request.

Fable, Opus, Astra and Sol may also commit straight to `main`. Every other change goes through a pull request that a different model reviews. CONTRIBUTING.md has the review rules.

## Pull requests and commits

- The title follows Conventional Commits: `type(scope): summary`, scope optional, type one of `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`, `ci`, `chore`, `revert` or `style`. For example, `fix(excel): keep column widths when a sheet is renamed`. CI checks the title.
- The description fills in every section of [the template](.github/pull_request_template.md): what changed, screenshots, evidence and known limitations. Write for a person skimming it, in plain bullets. For any visible change, take cropped 2x screenshots as described under Screenshots below and upload them with `scripts/pr-screenshots.sh`, which prints the Markdown to paste. Evidence is what you ran and what it printed, not a claim that it works. Known limitations names what you couldn't test.
- The description ends with these three lines, spelled exactly like this, in this order. Give your exact model, the reasoning level you ran at, and your harness, as [CONTRIBUTING.md](CONTRIBUTING.md#pull-request-descriptions) defines them. Reviews end with the same three lines.

  ```
  Model: Claude Opus 5.5
  Reasoning: high
  Harness: Claude Code 2.1 in T3 Code
  ```

- Each commit message ends with a `Co-Authored-By:` trailer for the model that wrote it, such as `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Use the trailer your harness gives you.
- Run the checks in [CONTRIBUTING.md](CONTRIBUTING.md#before-you-open-a-pull-request) before you open the pull request.

## Releases

- Cut a nightly only when the maintainer asks for one: `gh workflow run nightly.yml --ref main`. Add `-f notes="…"` to say what to try in it.
- A stable release follows [docs/internals/releasing.md](docs/internals/releasing.md).

## Fonts on Linux

Most Word and PowerPoint files are set in Arial, Times New Roman or Courier New. Linux doesn't have them, so Writan draws them with look-alikes (Liberation Sans, Serif and Mono). Lines break in the same places, but the letters aren't Microsoft's, and Georgia, Verdana and Trebuchet MS fall back to unrelated fonts. Screenshots and visual checks taken that way don't show what the document looks like on a Mac or Windows PC. Installing Microsoft's fonts on the system itself (Debian and Ubuntu: `ttf-mscorefonts-installer`) makes documents use them.

Calibri and Cambria can't be downloaded anywhere, so they always show as Carlito and Caladea, which take the same space. That isn't a bug.

Writan's monospace text uses the system's `monospace` font. Without DejaVu (`fonts-dejavu-core`), Linux picks Nimbus Mono, a copy of Courier.

## Screenshots

Take screenshots at 2x. The person reading them is usually on a Retina Mac, where Writan draws every CSS pixel with four. Playwright and other headless browsers default to 1x. At 1x, body text at 78% zoom is 11 pixels tall, and on a Mac it looks faint and blurred. `launch()` in [tests/stress/ui/harness.mjs](tests/stress/ui/harness.mjs) opens the browser at 2x. In a script of your own, pass `deviceScaleFactor: 2` to `browser.newContext()`.

Crop each screenshot to the part it shows, such as the chat panel or one page. The harness's `shot(page, path, selector)` does that. The window has to fit the part: a Letter page at 100% is 1056 pixels tall, so it needs a viewport taller than the default 900. The whole window at 2x is 2800 pixels wide, and chat apps and models shrink a picture that big, which blurs it again.

A screenshot from Linux still differs from the Mac. FreeType draws letters thinner than macOS, and the interface shows in Inter, not San Francisco.
