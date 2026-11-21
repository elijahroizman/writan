# Editing together

Several people can edit one Word document, presentation or workbook at the same time. One person shares it. The others join with the link they get, from a computer on the same network. This page is about how that works. What it looks like to use is in [features](../user/features.md#sharing-on-your-network).

```
 host's app                                         guest's app
 ┌─────────────────────────────┐                    ┌──────────────────────┐
 │ editor ─ engine (wasm)      │                    │ editor ─ engine      │
 │   │ collab/session.ts       │                    │   │ session.ts       │
 │   │ transport.ts            │                    │   │ transport.ts     │
 │ src-tauri/src/collab        │                    │ src-tauri collab     │
 │   client ── ws://127.0.0.1 ─┼─┐                  │   client             │
 │   server: relay             │◀┘◀── ws://LAN ip ──┼───┘                  │
 └─────────────────────────────┘                    └──────────────────────┘
```

## Three parts

**The relay** (`src-tauri/src/collab/relay.rs`) puts changes in order. It gives each change the next sequence number and sends it to every copy, the sender included. It keeps the opening bytes and every change since, so someone joining late gets the bytes and then the changes. It never reads a change, so it doesn't need the engine. `server.rs` runs it as a WebSocket server on port 47811, or any free port if that one's taken. The host's app starts it when the first document is shared and stops it when the last one stops.

**The engine** (`crates/office-engine/src/collab.rs`) keeps the copies the same. Every copy opens the same bytes (the host's tab reloads them too when sharing starts) and applies the changes in the relay's order. A change is a list of the layer's undo steps. Each step goes with a fingerprint of what it replaced and the new content: a range of paragraphs in Word, a slide or its notes in PowerPoint, cells or a sheet in Excel. A copy applies a step only if what's there now has the fingerprint. Two people who change the same paragraph, slide or cell at the same moment both send a step that replaced the old content. The relay orders them, the first applies, and every copy drops the second, since every copy has the same document when the second's turn comes.

A change made here shows at once and stays pending until the relay sends it back. Each copy has one change out with the relay at a time. What it does meanwhile waits, and is written for sending only when the last change comes back, with that as its base. So every change the relay ordered between a change's base and the change itself came from other copies, one after another. When any change arrives, the engine takes the pending ones back, applies the arrival, and puts the pending ones on again. A pending change that no longer fits is dropped, and so is every change made here after it, since they were made on what's gone. What a dropped change added to the layer's own tables (below) is still sent. Undo goes through the same check, so it takes back your change where it is now, or does nothing if someone else changed that content since.

Before checking, a step can move. A Word splice moves past each splice the relay ordered after its base: down by the blocks they added above it, in the same list or a list that holds it. If one of them replaced the same blocks, the splice is dropped instead. A PowerPoint slide step whose slide moved finds it by its fingerprint. Without this, typing on a line while someone presses Enter above it would be dropped.

**The page** (`src/collab/session.ts`) connects an editor's engine to a relay. It sends what `collabTake` returns after every change and passes what arrives to `collabReceive`. In the app every connection goes through Rust (`client.rs`), because the page's content security policy doesn't allow sockets to other machines. In a browser it opens the WebSocket itself.

## What a change carries

Each model node keeps the markup the model doesn't read as its origin (see [the office compiler](office-compiler.md#the-model)). A paragraph without it would save without its unknown properties. The model's JSON leaves origins out, so a change is serialized inside `office_core::model::wire::encode`. That sends an origin that was already in the replaced content by its place there (`{"r": 3}`), and a new one as XML (`{"x": "<w:p…>"}`). The receiver collects the origins of its own replaced content in the same order and uses those, so an untouched paragraph keeps sharing its origin and saves as it was.

Some edits also add to a layer's own tables, outside the undo steps. Each layer sends those along as something to do again, and doing it again where it's been done changes nothing:

- **Word:** a new list's numbering, a built-in style used for the first time, a new header or footer.
- **PowerPoint:** notes copied with a duplicated slide. New slide parts are named after the copy that made them, so two people adding slides at once don't pick the same name.
- **Excel:** a cell's style travels as its format, not its index in the style sheet, and the receiver finds or adds that format. Two people formatting cells at once would otherwise give different formats the same index.
- **Pictures:** a picture's bytes go with the first change that uses it, under an id that names the copy that added it.

## Messages

JSON text frames over WebSocket, at `/v1/<session>`:

| From | Message | Meaning |
| --- | --- | --- |
| copy | `hello { v, token, client, name, color, hostKey?, snapshot? }` | First message. The host's carries the opening bytes (base64) and the host key. `color` is the one asked for. |
| relay | `welcome { client, color, host, seq, session, snapshot, ops, peers }` | The opening bytes and every change so far. `color` is this copy's: the one asked for if nobody has it, else the first free one in `COLORS`. `peers` have `{ client, name, color, host, presence }`. |
| copy | `op { id, base, steps }` | A change. `base` is the last change this copy had applied. |
| relay | `op { seq, client, id, base, steps }` | The change in its place in the order, to everyone. |
| copy | `presence { data }` | Where this copy's caret is. The relay keeps the last one for newcomers' `welcome` and sends it on as `presence { client, data }`. |
| relay | `join { client, name, color, host }`, `leave { client }` | Someone arrived or left. |
| host | `end` | The host stopped sharing. The relay tells everyone with `end` and forgets the session. |
| relay | `error { code, message }` | `bad-token`, `not-ready`, `no-session`, `version`, `bad-message`, `not-host`. |

The invite is the relay's address with the token as a query parameter: `ws://192.168.1.20:47811/v1/<session>?token=<token>`. The token is 192 random bits. Anyone with the link can edit while the host's tab is open.

## Where everyone is

Each copy sends `presence` when its caret or selection moves (at most every 50 ms), after each change it sends, and when someone joins. What's in it is the editor's:

- **Word:** `{ s, a, f }`, the story (`b` for the body) and the selection's ends as `[path, byte offset]` (`WordView::collab_presence`).
- **PowerPoint:** `{ slide, frames, e }`, the slide by part name, the frame ids picked, and while text is edited `{ frame, a, f }`.
- **Excel:** `{ sheet, a, f }`, the sheet by name and the selection's corners as `[row, column]`.

The receiving engine places it in its own document (`remote_overlay` in each layer's view): an end that isn't in this copy yet hides the caret, and an offset past a paragraph's end stops there. The sender sends again after its change, so the caret lands once both copies have it. The page draws them with `src/collab/cursors.tsx`. A person's color comes from the relay, which never gives two people the same one while there are free ones (`relay::COLORS`, the same list as `session.ts`). The color asked for comes from the name, so a person tends to keep theirs.

## Moving to a server

Only the relay's address would change. `relay.rs` uses no sockets and no Tauri, so a server can run it as it is. `Policy::Open` already lets the first hello with a snapshot create a session, which is what the browser version and the test relay use (`cargo run --example collab_relay`). A server would add:

- accounts and invites in place of the shared token, and `wss://` (the client already connects to it);
- saving the snapshot and changes, so a session outlives the host's app;
- compaction. The relay keeps every change since the opening bytes, and a copy that joins late applies them all. Replacing them with new bytes needs every copy's document to equal what it reads back from those bytes, which hasn't been checked yet;
- reconnecting. A copy that loses its connection stops following now, and joining again opens a new tab.

## Known gaps

Everything sharing doesn't do yet, with what's left to build, is in [limits and what's left](collaboration-todo.md). The rarest cases:

These need two people to do the same rare thing within the time a change takes to reach the relay and back (milliseconds on a LAN):

- Two new Word lists, or two new headers or footers, made at once can get the same id on different copies, and the copies then disagree about that list or header.
- Notes typed on a slide while someone else reorders slides can land on the slide that took its place.
- Two people editing different shapes on one slide at once: the second edit is dropped, since a slide is checked whole.

## Tests

- Each layer's `view/collab.rs` has a `Room`: copies of a document and a relay in memory, driven through concurrent edits.
- `src-tauri/src/collab` tests the relay's messages, and the server with real sockets.
- `tests/stress/ui/collab.mjs` opens two browsers on a test build and shares a Word document, a presentation and a workbook from one to the other through the test relay. It isn't part of `npm run test:browser`, because it needs the relay running: `cd src-tauri && cargo run --example collab_relay -- --port 47812`.
