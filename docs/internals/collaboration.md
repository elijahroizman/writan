# Editing together

Several people can edit one Word document, presentation or workbook at the same time. One person shares it. The others join with the six-digit code they get, from anywhere. This page is about how that works. What it looks like to use is in [features](../user/features.md#sharing).

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

The invite is the relay's address with the token as a query parameter: `ws://192.168.1.20:47811/v1/<session>?token=<token>`. The token is 192 random bits. Anyone with the link can edit while the host's tab is open. People don't see it unless they ask for it; they join with a code.

## Over the internet

In the app, sharing puts the session in Writan's code directory, a Cloudflare Worker with one SQLite Durable Object (`worker/writan-join/`, reached through `src-tauri/src/collab/directory.rs`; `WRITAN_JOIN_URL` points the app at another). The directory picks a six-digit code nobody else has, keeps the host's iroh address and the session id under it for ten minutes, and gives the host a secret that alone can replace the code (`POST /rooms/<code>/rotate`) or remove it (`DELETE`). Anyone can look a code up (`GET /rooms/<code>`), but one address (an IPv6 /64 counts as one) gets 5 misses a minute. It never sees the token, a document or a change.

The host's app runs an [iroh](https://iroh.computer) endpoint while anything is shared this way (`p2p.rs`). It's QUIC, so it's encrypted, and iroh checks that whoever answers holds the key the address names. It connects directly when hole punching works, and otherwise through n0's public relays, which pass on packets they can't read. A guest's `collab_find` asks the directory first and gets a `writan-p2p://` invite (the address, the session and the code), which `collab_connect_p2p` dials. The guest's first frame is `{ session, code, name }`. The host checks the code. If the host asked to let people in, it answers `{ waiting: true }` and the page shows the name with **Let in** and **Deny** for up to a minute, then `{ ok: true, token }`; otherwise `{ ok: true, token }` right away, or `{ error, message }`. From then on each frame is one of the relay's messages, length-prefixed, into the host's relay as if it came over a WebSocket (`server::RelayBridge`), starting with the guest's hello.

The directory knows every code, so it's trusted to hand out the right address; sending the code over iroh tells nobody anything the directory doesn't know. The host limits the rest: a code works for ten minutes, five wrong ones end it early (the panel asks for a new one), at most 16 guests can be connecting at once and 5 can wait to be let in per document. **New code** asks the directory for a new code for the same room, and the old one stops working on both. When the directory can't be reached, the code is for the local network instead (below), and an internet code is never answered there: anyone nearby could hear it asked for, and the local lookup has no approval step.

## Join codes on the network

Without the internet, the session's code is for the local network (`src-tauri/src/collab/codes.rs`). The host's relay answers lookups for it two ways:

- `GET /join/<code>` on the relay's port, refused when the request has an `Origin` (a web page);
- a UDP datagram `writan-join 1 <code>` on port 47811, answered only when the code is there.

Both answer `{ session, token, port }`. When the directory doesn't have the code, the guest's app (`collab_find`) broadcasts the UDP lookup to 255.255.255.255 and each of its networks' `.255`, and at the same time asks `GET /join/<code>` of every address in each of its /24 networks on port 47811, 48 at a time, and of itself. The first answer wins, and the guest builds the invite link from it and joins as before. Both run because some networks drop broadcasts and others are bigger than a /24. After 4 seconds with no answer, the guest is told nobody has that code; if the system refused to reach the network (macOS before it's allowed), it says so, and the page waits as it does for a link.

A code has a million values, so each relay limits wrong guesses: 20 a minute from one address and 100 from everyone, after which it answers nothing until the minute is up. A guest's search reaches every Writan on the network, so a few misses per join are normal. A code ends with its session.

AI agents never use a code. They reach shared documents through the MCP server, which needs its own token (below), and the page hands their copies the host's invite itself.

## Where everyone is

Each copy sends `presence` when its caret or selection moves (at most every 50 ms), after each change it sends, and when someone joins. What's in it is the editor's:

- **Word:** `{ s, a, f }`, the story (`b` for the body) and the selection's ends as `[path, byte offset]` (`WordView::collab_presence`).
- **PowerPoint:** `{ slide, frames, e }`, the slide by part name, the frame ids picked, and while text is edited `{ frame, a, f }`.
- **Excel:** `{ sheet, a, f }`, the sheet by name and the selection's corners as `[row, column]`.

The receiving engine places it in its own document (`remote_overlay` in each layer's view): an end that isn't in this copy yet hides the caret, and an offset past a paragraph's end stops there. The sender sends again after its change, so the caret lands once both copies have it. The page draws them with `src/collab/cursors.tsx`. A person's color comes from the relay, which never gives two people the same one while there are free ones (`relay::COLORS`, the same list as `session.ts`). The color asked for comes from the name, so a person tends to keep theirs.

## AI agents

Agents join through Writan's MCP server, which the relay's server also serves, on the same port (`src-tauri/src/collab/mcp.rs`). It speaks MCP's Streamable HTTP: every JSON-RPC message is a POST to `/mcp/<token>`, answered with one JSON response, and `initialize` starts a session (`Mcp-Session-Id`). A request with an `Origin` header is refused, so a web page can't drive it. The newer sessionless `server/discover` gets "method not found", and clients fall back to `initialize`.

The tools are the page's (`src/collab/agents/tools.ts`), because the page has the engine. The relay's server sends each `tools/list` and `tools/call` to the page over a WebSocket at `/bridge/<key>`, which only this computer can open, and waits for the answer. The page connects to it through `transport.ts`, like a shared document, so the browser reaches the standalone relay (`--mcp <token> --bridge <key>`) the same way.

An agent works on a copy of its own, with no tab: `followAsAgent` in `session.ts` joins the document through the relay like a guest, and the copy's engine opens without a view. Its edits are the editor's own commands, so they reach everyone as ops, and its caret as presence, like anyone's. Its hello carries `agent` (`claude`, `codex`…, or `ai`), which the relay passes on in `join` and `peers`, and its own color, which the relay lets it keep even when someone has it. `src/collab/agents/identity.ts` names each agent, from the URL's `?agent=` (set when Settings › AI installs it) or the MCP client's name, and holds its color and logo. The logos are the companies' own marks, unchanged, shown only to say which agent is which.

The tools live in `src/collab/agents/`: `tools.ts` lists them and runs the shared ones (reading, seeing, waiting, undo), and `word.ts`, `slides.ts` and `sheets.ts` do each kind's editing with the editor's own commands. An edit's answer has an `after` that runs once the relay has the change, so it describes the shared document: the paragraphs, the shape's place and look, or the cells. `view_document` draws a slide or page from the engine's display list onto a canvas (`picture.ts`) and sends it as MCP image content, with each shape outlined and labelled with its id. `wait_for_changes` compares the document before and after someone else's change, after they've stopped for a second, and says which paragraphs, slides and shapes differ. The relay's server tells the page whether a call came from this computer (`local`); only then may `add_picture` read a file or an address.

Settings › AI installs the server and the skill (`src/collab/agents/SKILL.md`) into an agent (`src-tauri/src/agents.rs`): Claude Code through its CLI, the others by changing only Writan's entry in their config files.

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
