# Editing together: limits and what's left

Everything sharing doesn't do yet, in one list: what people notice, the gaps in how copies stay the same, and the work a cloud server would need. How it works is in [editing together](collaboration.md). The short version for users is under **Sharing** in [what it doesn't do yet](../user/limitations.md).

## What people notice

- **Links work on the same network only.** Over the internet a guest needs the code, so a code that has stopped working means asking for a new one, and an agent can't join a document someone joined over the internet: its copy has no invite to reuse (`inviteOf`). On the network, a guest finds the sharer by its six-digit code, which reaches only computers on the guest's /24 and its broadcast domain (see [join codes](collaboration.md#join-codes-on-the-network)). Elsewhere the link has to be sent by hand, and Writan doesn't open `ws://` links on its own; people paste them into **Join a shared document…**.
- **Relays run by someone else.** When two computers can't connect directly, iroh goes through n0's free public relays, which can't read what they pass on but aren't Writan's. The code directory runs on one Cloudflare account's workers.dev address.
- **The sharer has to stay.** The session lives in the sharer's app. Closing the tab, stopping sharing, quitting or reloading ends it for everyone. The others are told, and their tabs close unless they keep an unsaved copy.
- **Firewalls and macOS's local network permission.** The relay listens on port 47811 (or any free port). macOS and Windows ask the first time, and a firewall that says no stops guests from joining. macOS 15 and later also ask whether an app may reach the local network. Sharing and joining start with an empty mDNS query (`collab_local_network`), because that's something macOS checks and a connection to the router isn't, so the question comes right then. macOS can't be asked for the answer, so a guest whose connection fails with "no route to host" is shown what to do and tries again every second for a minute (`src/collab/network.ts`). Tried on macOS 27.3: the query brought up the question, and until it was answered connections to other machines failed in 2 ms with error 65.
- **No encryption on the LAN.** When the internet can't be reached, the relay speaks plain `ws://`. Anyone on the network who can see the traffic can read the document, and anyone with the link or the code can edit while it's shared. The token in the link is 192 random bits, compared in constant time. The code is six digits, so it leans on a limit on wrong guesses instead (20 a minute from one address, 100 from everyone).
- **No reconnecting.** A copy that loses its connection stops following and says so. Reconnecting needs the engine to send again the change that was out with the relay when the connection dropped, unless the relay already has it. Joining again opens a new tab with the current document; changes made in the old one after the drop aren't sent.
- **Cursors only where they're cheap to place.** Word draws others' carets in the body only, not in headers, footers or notes. PowerPoint draws them on the slide shown, and only marks the others' slides in the list. A caret is placed from what the sender last said, so for a moment after someone else's change above it, it can sit a line off until the sender's next presence arrives.
- **Word comments don't travel.** Comments are kept by the page (`src/editors/docx/commentXml.ts`), outside the engine's undo steps. Ones added, answered or resolved while sharing stay on the copy they were made on. The host's file only gets the host's.
- **Conflicts lose the second change.** When two people change the same paragraph, slide or cell at the same moment, the first change counts and the second is undone on the copy that made it, without a message. Anything that copy did after it, before hearing back, goes too.
- **Slides are checked whole.** Two people editing different shapes on one slide at the same moment: the second edit is dropped.
- **Guests' copies are unsaved documents.** A guest can save their copy with Save As, but it isn't linked to the host's file.

## Gaps in keeping copies the same

These need two people to do the same rare thing within the time a change takes to reach the relay and back (milliseconds on a LAN, more through a server). The copies still agree with each other except where noted.

- **New Word lists at the same moment** can get the same numbering id on different copies. The copies then disagree about which list it is. A fix sends a list's definition with an identity of its own and maps it to a local id.
- **New headers or footers at the same moment** can get the same story index. Same fix: name the story by section and slot, not by index.
- **Notes typed while someone reorders slides** can land on the slide that took its place, since notes are found by index and, failing that, by their own fingerprint (empty notes all look alike). A fix sends the slide's identity with the notes step.
- **Adding notes to a duplicated slide** names its part. Taking that change back leaves the name, which can make a concurrent deck change fail on that copy alone.
- **A change of several Word splices** moves each one past others' changes on its own, not past its own earlier splices too. Its intent can be off near structural edits; the fingerprint check stops it landing on content that differs.
- **Origin-only changes don't count as conflicts.** Fingerprints leave out each node's unread XML, so a stale text edit can replace a paragraph whose only change was in markup the model doesn't read.
- **Changes past a lost one are dropped, not rebased.** The reviewer suggested moving queued edits past the lost change instead. Dropping is simpler and can't misplace text; rebasing would keep more of what people typed.

## The relay and the connection

- **Compaction.** The relay keeps every change since the opening bytes, and a late joiner applies them all. Replacing them with fresh bytes needs every copy's document to equal what it reads back from those bytes, which hasn't been checked.
- **Message size.** The relay refuses frames over 64 MiB. A change that big (a huge picture, a whole deck changed at once) shows only on the copy that made it. Changes aren't size-checked before they're sent.
- **Slow peers.** Each connection's outgoing queue has no limit, so a guest that stops reading grows the host's memory until a write times out (30 s).
- **Big welcomes.** A late joiner gets the opening bytes and every change as one Tauri event, as JSON. A `tauri::ipc::Channel` would be faster for large documents.
- **Finding addresses.** The invite uses the first LAN address found by opening UDP sockets toward a few private and public addresses. Machines with several adapters may need another address from the list the panel shows.
- **`wss://`** is compiled in (native TLS) but untested, since nothing serves it yet.

## For a cloud server

The relay core (`src-tauri/src/collab/relay.rs`) runs as it is; `Policy::Open` already lets the first hello with a snapshot create a session. A server would add:

- accounts and per-person invites in place of the shared token, and `wss://`;
- storage for the snapshot and changes, so a session outlives the person who started it, and a way to say whose copy is the file;
- compaction (above);
- reconnecting: on reconnect, apply the changes after the last one seen and resend the change that was out;
- moving the relay core into its own crate, out of `src-tauri`.

## Tests and tooling

- `tests/stress/ui/collab.mjs` isn't in `npm run test:browser` or CI, because it needs the relay running. CI could start `cargo run --example collab_relay` first.
- The PowerPoint and Excel layers have no randomized convergence test like Word's `random_edits_converge`.
- The last two review fixes (dropping changes made after a lost one, and still sending what a dropped change added to the layer's tables) have tests but no second review.
