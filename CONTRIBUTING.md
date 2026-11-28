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

Each description names the model that wrote the change, with its version, and the harness it ran in:

```
Model: Opus 5.5
Harness: Claude Code
```

If you wrote the change yourself without a model, say that instead. A reviewer names their model and harness in the review too.

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
