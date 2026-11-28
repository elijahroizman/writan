# Contributing

Pull requests from outside the project are welcome. Opening one doesn't mean it will be merged, so for a large change, open an issue first and ask whether it fits.

## Bugs and ideas

Report bugs and suggest features in [GitHub issues](https://github.com/elijahroizman/writan/issues). For a bug, say which version of Writan and which system you're on, and attach the file that shows it if you can share it.

## Pull request titles

Titles follow [Conventional Commits](https://www.conventionalcommits.org), as T3 Code's do: `type(scope): summary`. The scope is optional. The type is one of `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`, `ci`, `chore`, `revert` or `style`. For example:

- `fix(excel): keep column widths when a sheet is renamed`
- `perf(collaboration): send each picture once`
- `docs: explain release channels`

CI checks the title, and the check fails until it matches.

## Pull request descriptions

A reviewer should understand a pull request in a minute or two, so keep each description simple. The [template](.github/pull_request_template.md) has these four sections, and every pull request fills in all of them:

- **What changed.** A few plain bullets on what someone using Writan or working on it will notice, and why if it isn't obvious. Skip the implementation detail the diff already shows.
- **Screenshots.** Before and after pictures of anything you can see, cropped to the part that changed and taken at 2x. `scripts/pr-screenshots.sh before.png after.png` uploads them to the `pr-screenshots` branch and prints the Markdown to paste. If nothing visible changed, write "No visible change."
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

## Before you open a pull request

- `npm run typecheck` passes after building the engine with `npm run engine` or `npm run build`. It runs [tsc-rs](https://github.com/pingdotgg/ts-rust), not Microsoft's `tsc`.
- `npm run build` passes.
- If you changed Rust code, `cargo check` passes in `src-tauri`.
- Keyboard shortcuts and labels work on both macOS and Windows. Use `shortcut()` from `src/lib/platform.ts` for any shortcut you show in the UI, and don't hard-code ⌘ or ⌥.

[Development](docs/internals/development.md) explains how to set up, run the app and run the browser tests that CI runs.

## Who can push to `main`

Fable, Opus, Astra and Sol can commit straight to `main`.

Every other model must open a pull request and wait until a different model has reviewed it. A model can't review its own pull request. Don't merge until the review is done and its comments have been addressed.

AI agents: [AGENTS.md](AGENTS.md) says which models may open a pull request at all.

## Releases

Stable releases are built from version tags. A nightly build of `main` starts with `gh workflow run nightly.yml --ref main`. [Releases and signing](docs/internals/releasing.md) has the steps.
