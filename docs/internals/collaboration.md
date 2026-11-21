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

A change made here shows at once and stays pending until the relay sends it back. When any change arrives, the engine takes the pending ones back, applies the arrival, and puts the pending ones on again. A pending change that no longer fits is dropped. Undo goes through the same check, so it takes back your change where it is now, or does nothing if someone else changed that content since.

Before checking, a step can move. A Word splice made before someone else's splice above it, in the same list or a list that holds it, moves by the number of blocks they added or removed. A PowerPoint slide step whose slide moved finds it by its fingerprint. Without this, typing on a line while someone presses Enter above it would be dropped.

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
| copy | `hello { v, token, client, name, color, hostKey?, snapshot? }` | First message. The host's carries the opening bytes (base64) and the host key. |
| relay | `welcome { client, seq, session, snapshot, ops, peers }` | The opening bytes and every change so far. |
| copy | `op { id, base, steps }` | A change. `base` is the last change this copy had applied. |
| relay | `op { seq, client, id, base, steps }` | The change in its place in the order, to everyone. |
| copy | `presence { data }` | Sent on to the others. Not used yet. |
| relay | `join`, `leave` | Someone arrived or left. |
| host | `end` | The host stopped sharing. The relay tells everyone with `end` and forgets the session. |
| relay | `error { code, message }` | `bad-token`, `not-ready`, `no-session`, `version`, `bad-message`, `not-host`. |

The invite is the relay's address with the token as a query parameter: `ws://192.168.1.20:47811/v1/<session>?token=<token>`. The token is 192 random bits. Anyone with the link can edit while the host's tab is open.

## Moving to a server

Only the relay's address would change. `relay.rs` uses no sockets and no Tauri, so a server can run it as it is. `Policy::Open` already lets the first hello with a snapshot create a session, which is what the browser version and the test relay use (`cargo run --example collab_relay`). A server would add:

- accounts and invites in place of the shared token, and `wss://` (the client already connects to it);
- saving the snapshot and changes, so a session outlives the host's app;
- compaction. The relay keeps every change since the opening bytes, and a copy that joins late applies them all. Replacing them with new bytes needs every copy's document to equal what it reads back from those bytes, which hasn't been checked yet;
- reconnecting. A copy that loses its connection stops following now, and joining again opens a new tab.

## Tests

- Each layer's `view/collab.rs` has a `Room`: copies of a document and a relay in memory, driven through concurrent edits.
- `src-tauri/src/collab` tests the relay's messages, and the server with real sockets.
- `tests/stress/ui/collab.mjs` opens two browsers on a test build and shares a Word document, a presentation and a workbook from one to the other through the test relay. It isn't part of `npm run test:browser`, because it needs the relay running: `cd src-tauri && cargo run --example collab_relay -- --port 47812`.
