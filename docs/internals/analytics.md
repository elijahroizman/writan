# Usage analytics

Writan can send anonymous usage events to PostHog. They answer questions like which formats people open, how long opening takes on real machines, which features get used, what fails, and which versions and systems are live. They never carry content. [PRIVACY.md](../../PRIVACY.md) is the page for users; this one is for people changing the code.

## How it fits together

| File | What it does |
|---|---|
| `src/lib/analytics/events.ts` | Every event and every value its properties can take, plus the helpers that turn sizes, durations and paths into those values. |
| `src/state/analytics.ts` | The consent store, the queue and `track()`. Decides whether anything can be sent. |
| `src/lib/analytics/documents.ts` | Times document opens and records failures. Editors call `documentReady` and `documentFailed`. |
| `src/lib/analytics/usage.ts` | App-wide events: launch, update, daily activity, unhandled errors, shared sessions. Started from `App.tsx`. |
| `src/lib/analytics/launch.ts` | Reads what earlier launches saved before this one writes anything. Imported first in `main.tsx`. |
| `src-tauri/src/analytics.rs` | The `analytics_send` command. Posts a batch to PostHog's `/batch/` endpoint. |

Rust does the posting so the webview's content security policy stays closed to outside hosts. The page owns the queue. It batches in memory, keeps at most 200 events, and flushes every 30 seconds, when the window is hidden and when the app quits. On quit the page waits up to 1.5 seconds, and Rust then holds the exit for up to 2 seconds while a post is still going. A failed post is retried once after 5 seconds, then dropped. The page does the retry, and it checks before every attempt that the setting is still on, so turning it off mid-send stops the batch and never makes a new ID. Rust posts once and keeps no consent state.

Rust adds `os`, `$os` and `arch` to every event. The page adds `distinct_id`, `$process_person_profile: false`, `$geoip_disable: true`, `$lib: "writan"`, `app_version`, `channel` (from `workspace.updateChannel`) and `language` (`navigator.language`).

## When anything is sent

All of these must hold:

- The build has `VITE_POSTHOG_KEY`. `VITE_POSTHOG_HOST` is optional and defaults to `https://us.i.posthog.com`. The release workflow sets both from the repository's Actions variables `POSTHOG_KEY` and `POSTHOG_HOST`.
- It's the desktop app, not the browser build.
- It isn't a dev build (`import.meta.env.DEV`).
- `choice` in `workspace.analytics` is `true`.

Without a key, `track()` returns at once. Nothing is queued, no ID is made and no request goes out.

While `choice` is `null`, the person hasn't decided, and `defaultAnalyticsOn()` stands in for it. Where that's true, events go from the first launch. Where it's false, events queue in memory and are sent only if the person turns it on during that run. Settings and the welcome screen's last page show the switch at the same default, which is off when the time zone is in the EU, the EEA, the UK or Switzerland. A placeless zone, such as UTC or `Etc/*`, falls back to the regions of `navigator.languages`. The time zone wins otherwise, because many people outside Europe use en-GB. Turning it off empties the queue and deletes `workspace.analyticsId`. Turning it on again makes a new ID.

## The rules

These never go into an event, in any form:

- file names, folder names or paths
- document text, titles or anything derived from content
- URLs, search terms, chat messages or agent prompts
- names of people or collaborators
- error messages or stack traces
- anything that locates the person

The type map enforces this. A property is a number, a boolean, or a string from a union in `events.ts`. A call that passes a `string` doesn't compile, so there's nothing to strip at runtime. Turn raw values into an allowed one at the call site with the helpers: `kindOf`, `extensionOf` (reads only the extension, from a fixed list), `sizeBucket`, `minutesBucket`, `roundMs`, `versionOf`, `errorNameOf`, `agentOf` and `pluginOf`.

Durations go through `roundMs`, which rounds to 50 ms under a second, 250 ms under ten seconds and whole seconds after that, capped at ten minutes. Sizes and session lengths go into named steps.

## Adding an event

1. Add the event and its properties to `Events` in `src/lib/analytics/events.ts`. If a property needs a new set of values, add a string union for it. Don't widen anything to `string`.
2. Call `track('your_event', { ... })` at the one place every path to the action goes through. Prefer a store action over a button's `onClick`, so the menu, the palette and shortcuts all count.
3. If the action can repeat many times a second or runs on its own, count it once per document or per session, as the PDF editor and the LaTeX compile do.
4. Add the event to the table below and, in plain words, to the list in [PRIVACY.md](../../PRIVACY.md).

`track` is exported from `src/state/analytics.ts`:

```ts
track<E extends EventName>(event: E, ...[properties]: Props<E>): void
```

An event with no properties takes none: `track('onboarding_finished')`.

## Events

Every event also has the properties listed under "How it fits together".

| Event | Properties | Recorded |
|---|---|---|
| `app_launched` | `first_launch`, `launch_ms` | Once per launch, after the last session is restored. `launch_ms` counts from the page starting to load. |
| `app_active` | none | Once per calendar day, at launch or while the app stays open. Count unique IDs for daily users. |
| `app_updated` | `from_version`, `to_version` | First launch after the version changed. Not sent on the first launch of the first version with analytics. |
| `document_opened` | `kind`, `extension`, `size_bucket`, `source` (`open`, `restore`), `open_ms`, `engine_cold` | When the editor has drawn a document read from disk. `open_ms` runs from the open request, or from first showing a tab opened in the background, and is left out after a PDF password prompt. `engine_cold` is for Word, PowerPoint and Excel only. |
| `document_open_failed` | `kind`, `extension`, `stage` (`read`, `parse`, `render`, `plugin`, `unsupported`) | Reading failed, the editor rejected the file, the editor crashed, a plugin didn't start, or the type isn't one Writan opens. |
| `document_saved` | `kind`, `autosave` | A save that wrote the file. `autosave` is true for quiet saves, which are autosave and agents' saves. |
| `document_exported` | `from_kind`, `to_format`, `delivery` (`open`, `save`, `copy`) | A conversion that opened, saved or copied its result. Copy as Markdown counts as `md` with `copy`. |
| `feature_used` | `feature`, `kind` when there's a document | See the list below. |
| `collab_started` | `kind`, `role` (`host`, `guest`), `agents_only` | A shared session goes live. |
| `collab_ended` | `kind`, `role`, `agents_only`, `peak_participants`, `minutes_bucket` | It stops, disconnects or the tab closes. Participants include this copy. |
| `agent_connected` | `agent`, `local` | The first tool call of each MCP session. |
| `plugin_enabled`, `plugin_disabled` | `plugin` | The LaTeX plugin is turned on or off. |
| `error` | `area`, `name`, `source` (`uncaught`, `rejection`, `render`) | Unhandled errors and rejections, and editor crashes. `area` is the editor in front, `home` or `shell`. At most 20 per launch and one per area, name and source every 10 minutes. |
| `onboarding_finished` | none | Recorded by the welcome screen. |

The features in `feature_used`, and where each is counted:

| Feature | Where |
|---|---|
| `split_view` | `splitWith` in `state/workspace.ts` |
| `focus_mode`, `typewriter_scrolling` | turning them on in `state/focus.ts` |
| `pdf_sign`, `pdf_form_fill` | the PDF editor, once per document |
| `pdf_to_word` | `usePdfToWord.start` |
| `ocr` | the PDF editor's text recognition, once per document, and `recognizeImage` for pictures |
| `version_history` | `useVersionHistory.show` |
| `command_palette` | `usePalette.setOpen(true)` |
| `find`, `find_replace`, `print` | `runCommand` in `App.tsx`, when the editor has them |
| `theme_editor` | `useThemeEditor.open` |
| `latex_compile` | the first compile per LaTeX document |
| `present_slides` | `PresentMode` mounting |
| `open_folder` | `useFolder.open` with a new folder |
| `new_document` | `newDocument` |
| `bundle_pdf` | `bundleTabsToPdf` |

## PostHog project settings

Before the first release with a key, the maintainer turns these on in the PostHog project:

- Project settings › "Discard client IP data". Without it PostHog stores the IP address of every request.
- Data pipeline › Transformations: turn off GeoIP. Every event sends `$geoip_disable: true` already; this covers an event that ever goes without it.
- Person profiles: "Identified only" or "Never". Every event sends `$process_person_profile: false`, so no person profiles get made either way.
- Turn off session replay, autocapture, heatmaps and surveys. Writan doesn't use PostHog's JavaScript library, so none of them could collect anything anyway.

Then add the Actions variables `POSTHOG_KEY` (the project API key, which starts with `phc_` and is meant to be public) and `POSTHOG_HOST` if the project isn't on PostHog's US cloud. The EU cloud is `https://eu.i.posthog.com`.

## Testing

`tests/stress/ui/analytics.mjs` runs with `npm run test:browser`. A test build reads a stand-in key from `window.__analyticsTest`, and the test records `analytics_send` calls on the fake native bridge. It checks that nothing is sent without a key, that an undecided install sends only where the default is on, that a batch has no file names or paths, that off deletes the ID, and the default in Europe. `cargo test analytics` covers the Rust side.
