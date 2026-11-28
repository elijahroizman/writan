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
| copy | `hello { v, token, client, name, color, hostKey?, snapshot?, agent?, owner?, listens? }` | First message. The host's carries the opening bytes (base64) and the host key. `color` is the one asked for. An agent's says which agent, whose, and whether it gets other agents' messages. |
| relay | `welcome { client, color, host, seq, session, snapshot, ops, peers, features, said? }` | The opening bytes and every change so far. `color` is this copy's: the one asked for if nobody has it, else the first free one in `COLORS`. `peers` have `{ client, name, color, host, agent?, owner?, listens?, presence }`. `features` lists what the relay does beyond version 1 (`say`, and `chat` from Writan 0.15.3 on). `said`, for an agent that listens and for a person, is the document's chat so far, its last 100 messages. `gone` (Writan after 0.15.3) names the last 200 who were here and left, `{ client, name, agent?, owner? }`, so the changes in `ops` can be put to names. |
| copy | `op { id, base, steps }` | A change. `base` is the last change this copy had applied. |
| relay | `op { seq, client, id, base, steps }` | The change in its place in the order, to everyone. |
| copy | `presence { data }` | Where this copy's caret is. The relay keeps the last one for newcomers' `welcome` and sends it on as `presence { client, data }`. |
| relay | `join { client, name, color, host }`, `leave { client }` | Someone arrived or left. A `join` for someone already here (a rename, or an agent's `listen`) replaces what was known about them. |
| agent | `say { text, to? }` | A message to the document's chat, up to 8,000 characters. Only an agent from Writan 0.15.2 adds `to`, some agents' copies, which its tools told it people don't see: that one goes to them alone and isn't kept. |
| relay | `say { client, name, owner?, agent, text, to? }` | It, to every agent that listens and every person, never back to the sender, and kept for those who join later. The sender is named, for a message from someone who has left. |
| agent | `listen { on }` | Its person turned other agents' messages on or off. The others get a `join` saying so. |
| host | `end` | The host stopped sharing. The relay tells everyone with `end` and forgets the session. |
| relay | `error { code, message }` | `bad-token`, `not-ready`, `no-session`, `version`, `bad-message`, `not-host`. |

The invite is the relay's address with the token as a query parameter: `ws://192.168.1.20:47811/v1/<session>?token=<token>`. The token is 192 random bits. Anyone with the link can edit while the host's tab is open. In the app nobody sees it: people join with a code, and the app builds the link from what the code finds. Only the browser version, which has no codes, shows the link and takes one in the Join dialog.

## Over the internet

In the app, sharing puts the session in Writan's code directory, a Cloudflare Worker with one SQLite Durable Object (`worker/writan-join/`, reached through `src-tauri/src/collab/directory.rs`; `WRITAN_JOIN_URL` points the app at another). The directory picks a six-digit code nobody else has, keeps the host's iroh address, the session id and a key (below) under it for ten minutes, and gives the host a secret that alone can replace the code (`POST /rooms/<code>/rotate`) or remove it (`DELETE`). Anyone can look a code up (`GET /rooms/<code>`), but one address (an IPv6 /64 counts as one) gets 5 misses a minute. It never sees the token, a document or a change.

The host's app runs an [iroh](https://iroh.computer) endpoint while anything is shared this way (`p2p.rs`). It's QUIC, so it's encrypted, and iroh checks that whoever answers holds the key the address names. It connects directly when hole punching works, and otherwise through n0's public relays, which pass on packets they can't read. What the directory keeps under a code is the address, the session and a key of 128 random bits that's new with each code. A guest's `collab_find` asks the directory first and gets a `writan-p2p://` invite holding those, which `collab_connect_p2p` dials. The guest's first frame is `{ session, key, name }`. The host checks the key and that the code hasn't expired. If the host asked to let people in, it answers `{ waiting: true }` and the page pops up the name with **Let in** and **Deny** in the top right corner (`Arrivals.tsx`) for up to a minute, then `{ ok: true, token, seat }`; otherwise `{ ok: true, token, seat }` right away, or `{ error, message }`. The seat is 128 random bits of the guest's own, which the host keeps with their name for as long as the session lasts (below). From then on each frame is one of the relay's messages, length-prefixed, into the host's relay as if it came over a WebSocket (`server::RelayBridge`), starting with the guest's hello.

The directory knows every code and key, so it's trusted to hand out the right address. Only someone who looked the current code up has its key, so guessing codes is the directory's to limit. The host limits the rest: a code lets people join for ten minutes, though nobody who's in is taken out when it runs out, at most 16 guests can be connecting at once (until their hello), and 5 can wait to be let in per document. **New code** asks the directory for a new code and key for the same room; the old code stops resolving and an invite from it is turned away. A welcome can be bigger than what the host takes from a guest in one frame (64 MiB), so a guest takes up to 1 GiB from the host, as over a WebSocket. Quitting ends what's shared and takes its codes out of the directory, waiting a few seconds at most.

When the host can't reach n0's relays or the directory, the code is for the local network instead (below). The two kinds don't overlap: the directory's codes are 100000 to 999999, and the local network's start with 0. So a guest's `collab_find` asks only the one the code belongs to: an internet code is never broadcast nearby, where anyone could hear it and answer without the host's approval, and a local one never reaches the directory.

## Join codes on the network

Without the internet, the session's code is for the local network, and starts with 0 (`src-tauri/src/collab/codes.rs`). The host's relay answers lookups for it two ways:

- `GET /join/<code>` on the relay's port, refused when the request has an `Origin` (a web page);
- a UDP datagram `writan-join 1 <code>` on port 47811, answered only when the code is there.

Both answer `{ session, token, port }`. For a code that starts with 0, the guest's app (`collab_find`) broadcasts the UDP lookup to 255.255.255.255 and each of its networks' `.255`, and at the same time asks `GET /join/<code>` of every address in each of its /24 networks on port 47811, 48 at a time, and of itself. The first answer wins, and the guest builds the invite link from it and joins as before. Both run because some networks drop broadcasts and others are bigger than a /24. After 4 seconds with no answer, the guest is told nobody has that code; if the system refused to reach the network (macOS before it's allowed), it says so, and the page waits as it does for a link.

A local code has a hundred thousand values, so each relay limits wrong guesses: 20 a minute from one address and 100 from everyone, after which it answers nothing until the minute is up. A guest's search reaches every Writan on the network, so a few misses per join are normal. A code ends with its session.

AI agents never use a code. They reach documents through the MCP server, which needs its own token (below), and the page hands their copies the invite of the tab they join.

## Where everyone is

Each copy sends `presence` when its caret or selection moves (at most every 50 ms), after each change it sends, and when someone joins. What's in it is the editor's:

- **Word:** `{ s, a, f }`, the story (`b` for the body) and the selection's ends as `[path, byte offset]` (`WordView::collab_presence`).
- **PowerPoint:** `{ slide, frames, e }`, the slide by part name, the frame ids picked, and while text is edited `{ frame, a, f }`.
- **Excel:** `{ sheet, a, f }`, the sheet by name and the selection's corners as `[row, column]`.

The receiving engine places it in its own document (`remote_overlay` in each layer's view): an end that isn't in this copy yet hides the caret, and an offset past a paragraph's end stops there. The sender sends again after its change, so the caret lands once both copies have it. The page draws them with `src/collab/cursors.tsx`. A person's color comes from the relay, which never gives two people the same one while there are free ones (`relay::COLORS`, the same list as `session.ts`). The color asked for comes from the name, so a person tends to keep theirs.

## AI agents

Agents join through Writan's MCP server, which the relay's server also serves, on the same port (`src-tauri/src/collab/mcp.rs`). It speaks MCP's Streamable HTTP: every JSON-RPC message is a POST to `/mcp/<token>`, answered with one JSON response, and `initialize` starts a session (`Mcp-Session-Id`). A request with an `Origin` header is refused, so a web page can't drive it. The newer sessionless `server/discover` gets "method not found", and clients fall back to `initialize`.

The tools are the page's (`src/collab/agents/tools.ts`), because the page has the engine. The relay's server sends each `tools/list` and `tools/call` to the page over a WebSocket at `/bridge/<key>`, which only this computer can open, and waits for the answer. The page connects to it through `transport.ts`, like a shared document, so the browser reaches the standalone relay (`--mcp <token> --bridge <key>`) the same way.

An agent works on a copy of its own, with no tab: `followAsAgent` in `session.ts` joins the document through the relay like a guest, and the copy's engine opens without a view. Its edits are the editor's own commands, so they reach everyone as ops, and its caret as presence, like anyone's. Its hello carries `agent` (`claude`, `codex`…, or `ai`) and `owner`, the name of the person whose tab it joined through, which the relay passes on in `join` and `peers`, and its own color, which the relay lets it keep even when someone has it. Each app shows its own agents by their names ("Claude") and everyone else's with the owner's ("Yoav's Claude"; `peerOf` in `session.ts`).

An agent can join any Word document, presentation or workbook open in the app (`documents()` in `tools.ts`):

- **One shared from here**: through the host's own invite on 127.0.0.1.
- **One joined over the network**: through the host's `ws://` invite, like the guest.
- **One joined over the internet**: the agent's copy dials the host like a guest, with the guest's seat in place of the code's key and which agent it is, `{ session, seat, name, agent }`, and the host asks about "Yoav's Claude" like a person. A seat works after the code expires or changes, but an agent's join gets no seat of its own.
- **One not shared**: `openToAgents` shares it with no code (`collab_share` with `private`), so only this app's agents reach it. The tab reloads its bytes as for sharing, and stays unsaved if it was. Its share button shows who's in it and still offers **Share**, which gives the same session a code (`collab_publish`) and keeps the agents in. If the host types a new name then, a `rename` message tells everyone, and their agents' owner with it. Once the last agent leaves, the session ends. `src/collab/agents/identity.ts` names each agent, from the URL's `?agent=` (set when Settings › AI installs it) or the MCP client's name, and holds its color and logo. The logos are the companies' own marks, unchanged, shown only to say which agent is which.

**The agents' chat.** Each document has one chat, and the agents in it, this app's and other people's on other computers, talk to each other there with `send_message` (`messages.ts`). A message is the relay's `say`, so it travels as a change does: over the LAN, or through the iroh connection a guest's copies use, and needs nothing more of the network. It goes to every agent whose hello or last `listen` said it listens, and to every person's copy, whose tab keeps it in `useChat` for the title bar's chat button (`AgentChat.tsx`). Nothing pops up when one comes; the button gets a dot. MCP has no way for a server to interrupt an agent that every client supports (Claude Code's channels need a local stdio server and a flag), so an agent hears of a message in the answer to its next call on the document, whatever the tool: each answer ends with the messages it hasn't seen. `wait_for_changes` ends when one comes, or at once if one is waiting. Settings › AI turns messages off (on by default); then this app's agents don't list `send_message`, their copies send `listen { on: false }`, and other agents see them as having messages off. A relay older than Writan 0.15.2 has no `say` in `features`, and the tools say the host's Writan is too old; one in 0.15.2 passes messages to agents only. Whoever joins later gets the last 100. Each agent's copy keeps at most 200 unread, and each tab's chat 200. Messages come from other people's agents, so the skill and the tool tell the agent to treat them as a colleague's request, not as its person's instructions.

MCP clients give up on a call after about a minute (Claude Code and Codex both do), so no call waits longer than 45 seconds, and `wait_for_changes` answers within 50 even while someone is still typing.

In Word, each agent's copy keeps each paragraph's past (`History` in `word.ts`): who last changed its text, the relay's number for that change, and what its id became. It works this out from the paragraphs that differ after each change, so it's a good guess, not a record. It starts from the welcome's bytes, before the welcome's changes are applied (`onApplied` in `session.ts` hears after each run of one client's changes), so it knows who wrote what before the agent joined, and `gone` names those who have left. A long session's changes get two seconds; past that, the rest count as unknown. `read_document` marks who wrote each paragraph (the agent itself, its person, or anyone else), and `delete_paragraphs` and `replace_paragraphs` refuse someone else's without `confirm`. The agent's own person's paragraphs don't count as someone else's. `read_document` with `since` gives only what changed after a version, the relay's number it read at.

A paragraph's id (`ids.ts`) comes from its text, so every copy gives it the same one with nothing to agree on: six characters of a hash. An empty paragraph has none, and neither does one whose text another paragraph has too, since an id couldn't tell them apart; those go by number. Paragraphs put in or deleted next to ones with the same text can't be told apart either, so who wrote them counts as unclear, and changing them needs `confirm`. Every tool that takes a paragraph number takes an id too. When a paragraph's text changes, the copy's history records what its id became: changed paragraphs between two kept ones pair up in order. So an old id still finds its paragraph, and the answer says what it is now. `replace_paragraphs` and `delete_paragraphs` refuse an old id when someone other than the agent changed the paragraph since. A chat message that names an id shows under that paragraph in `read_document`, which makes it a note that stays with its paragraph while the session lasts. In the people's chat panel, an id that's in the document is a button that goes to it.

When someone comes into a document or leaves it, each agent's copy in it hears (`onComing`). The answer to its agent's next call says so, like a new message, and it ends `wait_for_changes`.

The tools live in `src/collab/agents/`: `tools.ts` lists them and runs the shared ones (reading, seeing, waiting, undo), and `word.ts`, `slides.ts` and `sheets.ts` do each kind's editing with the editor's own commands. An edit's answer has an `after` that runs once the relay has the change, so it describes the shared document: the paragraphs, the shape's place and look, or the cells. `view_document` draws a slide or page from the engine's display list onto a canvas (`picture.ts`) and sends it as MCP image content, with each shape outlined and labelled with its id. `wait_for_changes` compares the document before and after someone else's change, after they've stopped for a second, and says which paragraphs, slides and shapes differ. The relay's server tells the page whether a call came from this computer (`local`); only then may `add_picture` read a file or an address, `open_document` open a file, or `save_document` save one (`files.ts`). `save_document` won't write over a file other than the document's own, and a guest's agent can't save the host's document. `find_text` searches a document without reading it whole, and `redo` puts back what the agent's last undo took back, through the same check as undo.

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
