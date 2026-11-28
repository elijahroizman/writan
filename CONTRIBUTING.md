# Contributing

Pull requests from outside the project are welcome. Opening one doesn't mean it will be merged, so for a large change, open an issue first and ask whether it fits.

## Bugs and ideas

Report bugs and suggest features in [GitHub issues](https://github.com/elijahroizman/writan/issues). For a bug, say which version of Writan and which system you're on, and attach the file that shows it if you can share it.

## One change per pull request

Each pull request does one thing: a fix, a feature or a cleanup. If you find an unrelated bug on the way, open a separate pull request or an issue for it. A pull request that mixes unrelated changes is hard to review and hard to revert, and it gets sent back to be split. A single title that has to say "and" to describe it is a sign it should be two.

## Pull request titles

Titles follow [Conventional Commits](https://www.conventionalcommits.org), as T3 Code's do: `type(scope): summary`. The type is one of `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`, `ci`, `chore`, `revert` or `style`. The scope is optional. It has only lowercase letters, digits and hyphens, and starts with a letter or digit, such as `excel` or `pdf-export`. A `!` before the colon marks a breaking change. One space follows the colon, then the summary, which can't be blank. For example:

- `fix(excel): keep column widths when a sheet is renamed`
- `perf(collaboration): send each picture once`
- `docs: explain release channels`

The **PR title** check in CI fails until the title matches.

## Pull request descriptions

A reviewer should understand a pull request in a minute or two, so keep each description simple. The [template](.github/pull_request_template.md) has these four sections, and every pull request fills in all of them:

- **What changed.** A few plain bullets on what someone using Writan or working on it will notice, and why if it isn't obvious. Skip the implementation detail the diff already shows.
- **Screenshots.** Before and after pictures of anything you can see, cropped to the part that changed and taken at 2x. The easiest way is to drag them into the description box on GitHub, which uploads them. If you can push to this repository, `scripts/pr-screenshots.sh before.png after.png` uploads them to its `pr-screenshots` branch instead and prints the Markdown to paste. It needs git and push access, and its links point at the commit that added the pictures, so they never change. From a fork, set `PR_SCREENSHOTS_REMOTE` to your fork's remote to upload there. If nothing visible changed, write "No visible change."
- **Evidence.** How you know it works: the commands you ran and what they printed, the tests you added, the files you opened and what you clicked.
- **Known limitations.** What the change doesn't do yet, what you couldn't test, such as Windows or a real update, and anything a reviewer should look at closely. Write "None known." only after you've looked.

Each description ends with these three lines, spelled exactly like this, one per line and in this order. The pull request template has them already.

```
Model: Claude Opus 5.5
Reasoning: high
Harness: Claude Code 2.1 in T3 Code
```

- **Model** is the provider's full name for the model, with its version: `Claude Opus 5.5`, `GPT-6.2 Sol`, `Gemini 4 Neon`. Not a nickname like "Opus" or "the latest Claude".
- **Reasoning** is the effort level the model ran at, in the words the harness uses for it: `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` or `ultrathink`. Write `default` if you didn't set one, and `n/a` if the model has no setting.
- **Harness** is the tool that ran the model, with its version if it shows one, and the app around it if there is one: `Codex 0.172`, `Claude Code 2.1 in T3 Code`, `Devin`.

If several models wrote the change, give each its own three lines. If you wrote the change yourself without a model, write `Model: none` and leave out the other two. A reviewer puts the same three lines at the end of the review.

The **PR description** check in CI fails until the description has all four `## ` headings, each with at least one line of its own under it. HTML comments, code blocks, blank lines and a lone `-` don't count as that line, so the untouched template fails. The model lines must be the last lines of the description: either `Model: none` alone, or one or more complete sets of `Model:`, `Reasoning:` and `Harness:`, each with a value. Blank lines and the `🤖 Generated with` line some tools add may follow them. Pull requests opened by bots, such as Dependabot, skip this check.

## Before you open a pull request

- `npm run typecheck` passes after building the engine with `npm run engine` or `npm run build`. It runs [tsc-rs](https://github.com/pingdotgg/ts-rust), not Microsoft's `tsc`.
- `npm run build` passes.
- If you changed Rust code, `cargo check` passes in `src-tauri`.
- Keyboard shortcuts and labels work on both macOS and Windows. Use `shortcut()` from `src/lib/platform.ts` for any shortcut you show in the UI, and don't hard-code ⌘ or ⌥.

[Development](docs/internals/development.md) explains how to set up, run the app and run the browser tests that CI runs.

## Who opens pull requests

A model on [AGENTS.md](AGENTS.md#which-models-may-open-a-pull-request)'s list opens a pull request and waits until a different model has reviewed it. A model can't review its own pull request. Don't merge until the review is done and its comments have been addressed.

A model that isn't on that list doesn't open a pull request. It hands its work to the maintainer, who decides what happens to it.

## Releases

Stable releases are built from version tags. A nightly build of `main` starts with `gh workflow run nightly.yml --ref main`. [Releases and signing](docs/internals/releasing.md) has the steps.
