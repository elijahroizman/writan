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

Run the checks in [CONTRIBUTING.md](CONTRIBUTING.md#before-you-open-a-pull-request) before you open a pull request.

### Title

Conventional Commits: `type(scope): summary`. The scope is optional. The type is one of `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`, `ci`, `chore`, `revert` or `style`. For example, `fix(excel): keep column widths when a sheet is renamed`. CI fails a title that doesn't match.

### Description

Use [the template](.github/pull_request_template.md) and fill in every section. A person reads it, often in a minute or two, so write plain bullets and leave the implementation detail to the diff.

- **What changed.** What someone using Writan or working on it will notice, and why if it isn't obvious.
- **Screenshots.** Before and after pictures of every visible change, cropped to the part that changed and taken at 2x, as the Screenshots section below describes. Upload them with `scripts/pr-screenshots.sh before.png after.png`, which puts them on the `pr-screenshots` branch and prints the Markdown to paste. If nothing visible changed, write "No visible change."
- **Evidence.** What you ran and what it printed: typecheck, build, `cargo test`, the browser tests, the files you opened and what you clicked. "Tests pass" without the command isn't evidence.
- **Known limitations.** What the change doesn't do yet, what you couldn't test, such as Windows, macOS or a real update, and what a reviewer should look at closely. Write "None known." only after you've looked.

The description ends with these three lines, spelled exactly like this, in this order:

```
Model: Claude Opus 5.5
Reasoning: high
Harness: Claude Code 2.1 in T3 Code
```

- `Model` is the provider's full name for the model you are, with its version, such as `Claude Opus 5.5` or `GPT-6.2 Sol`. Never a nickname like "Opus".
- `Reasoning` is the effort level you ran at, in your harness's words: `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` or `ultrathink`. Write `default` if none was set, and `n/a` if your model has no setting.
- `Harness` is the tool that runs you, with its version if it shows one, and the app around it if there is one, such as `Codex 0.172` or `Claude Code 2.1 in T3 Code`.

If several models wrote the change, give each its own three lines. A review ends with the reviewer's own three lines.

### Commits

Each commit message ends with a `Co-Authored-By:` trailer for the model that wrote it, such as `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Use the trailer your harness gives you.

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
