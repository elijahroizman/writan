# Privacy

Writan works on your computer. Your documents stay there unless you share one in a collaboration session or let an AI agent work on it.

This page lists everything Writan sends over the internet and why. If something here is unclear or wrong, [open an issue](https://github.com/elijahroizman/writan/issues).

## Usage data

If you allow it, Writan sends anonymous usage data to [PostHog](https://posthog.com), an analytics service, on its US servers. The data shows which file types and features people use, how fast documents open, what goes wrong, and which versions and systems are in use. It guides what gets fixed and built next.

You can turn it on or off at any time in Settings › General › Help make Writan better.

### What it records

- When Writan starts: whether it's the first time on this computer, and how long it took.
- Each day you use Writan, once.
- An update: the version you had and the one you have now.
- Opening a document: its type, such as Word, PDF or Markdown, its extension, its size in rough steps such as "1 to 10 MB", and how long it took to show.
- A document that didn't open: its type and the step that failed, such as reading the file.
- Saving: the document's type, and whether autosave saved it.
- Exporting or converting: from which type to which format, and whether the copy went to a new tab, a file or the clipboard.
- Features you use, from a fixed list: split view, focus mode, typewriter scrolling, signing a PDF, filling in a PDF form, opening a PDF in Word, text recognition, version history, the command palette, find, find and replace, the theme editor, compiling LaTeX, presenting slides, printing, opening a folder, creating a document and combining files into one PDF.
- Sharing a document: its type, whether you shared it or joined, whether only AI agents were in it, the most people in it at once, and roughly how long it lasted.
- An AI agent connecting: which product it is, such as Claude Code or Codex, and whether it runs on this computer.
- Turning the LaTeX plugin on or off.
- An error: which part of the app it happened in, and the kind of error from a fixed list of standard names, such as "TypeError". Never the error's message.

Each of these also carries:

- a random ID made for this install, which isn't linked to you, your account or your computer's hardware
- Writan's version and update channel
- whether your system is macOS, Windows or Linux, and its processor type
- the language your system is set to, such as en-US

### What it never records

- your documents or anything in them
- file names, folder names or paths
- what you type, search for or paste
- links in your documents
- your name, or the names of people you share with
- what you or AI agents say in a shared document's chat
- error messages
- your location

Like every server you connect to, PostHog receives your IP address. Writan asks PostHog not to work out a location from it, and Writan's PostHog project is set to discard IP addresses. Writan also tells PostHog not to build a profile for the install ID.

### When it's on

Usage data is on by default everywhere except the European Union, the European Economic Area, the United Kingdom and Switzerland. Outside those places, Writan sends it from the first launch. The last page of the welcome screen shows the switch, and you can turn it off there or at any time in **Settings › General**.

In those European countries it starts off, and Writan sends nothing unless you turn it on. What happens before you choose waits in memory and goes only if you turn it on during that run. It's dropped if you leave it off or quit.

Writan works out which applies on your computer from its time zone, without looking anything up. If the time zone is set to UTC and so names no place, it goes by your language settings instead.

Turning it off stops sending at once, drops anything not yet sent and deletes the install ID from your computer. Events sent before can't be linked to you after that. If you turn it on again, Writan makes a new ID.

Copies built without Writan's analytics key, such as ones you build yourself, never send usage data. Neither does the version that runs in a web browser.

## Other connections

Writan connects to these services, and to no others.

### Updates

About 10 seconds after it starts, and every 4 hours after that, Writan asks GitHub whether there's a new version and reads the release notes. On a Mac or Windows PC a new version downloads from GitHub in the background and installs when you choose. On Linux, Writan's package comes from a repository on GitHub, and your system's package manager checks it on its own schedule. These requests carry nothing about you or your documents.

### Downloads you choose

- The LaTeX plugin downloads from GitHub when you turn it on. If a newer Writan needs a newer plugin, Writan downloads it when it starts.
- Text recognition for scanned PDFs and pictures has English built in. Other languages download from GitHub when you pick them in Settings, or when a scan needs one you picked that hasn't downloaded yet.
- Installing TeX for the LaTeX plugin uses Homebrew or a CTAN mirror on a Mac, winget or a CTAN mirror on Windows, and your package manager on Linux. On Windows, MiKTeX can download LaTeX packages a document needs while it compiles.
- Adding a theme from Settings searches Open VSX and downloads the theme from there.

### Sharing a document

Sharing sends the document, its name, every change and everyone's cursor to the people you let in. Your display name goes with it.

On the same network, the computers connect to each other directly. That connection isn't encrypted, so use it only on a network you trust. To find a session from its code, Writan asks the other computers on the network.

Over the internet, the code people type to join is kept by a small directory service on Cloudflare, run by Writan's developer. It stores the address the sharing computer can be reached at, which includes its IP addresses, for the 10 minutes the code works. The entry is deleted when you stop sharing, or within 24 hours after the code expires. The service also keeps the IP addresses of computers that create codes or type wrong ones for about 10 minutes, to stop guessing. It never receives the document, its name or anyone's name.

The document goes from computer to computer, encrypted end to end. When the computers can't reach each other directly, the traffic goes through relay servers run by n0, the company behind the networking library Writan uses. The relays can't read it. While a session is open, Writan also publishes a key and the address of its relay to n0's directory, so the other computers can find it.

### AI agents

When Settings › AI lets agents in, which is the default, Writan listens for agents such as Claude Code and Codex on port 47811, or another port if that one is taken. An agent needs the secret address Writan gives it when you set it up. An agent can read and change the Word documents, presentations and workbooks you have open. What it does with them, including what it sends to its own provider, is up to the agent.

An agent can also ask Writan to download a picture from a web address to put in a document.

### Videos and links

Inserting a YouTube video in a slide downloads its thumbnail from YouTube. Playing a video loads YouTube's or Vimeo's player, and those services can see that you played it. Links you click open in your web browser.

## Questions and deletion

Open an issue at [github.com/elijahroizman/writan/issues](https://github.com/elijahroizman/writan/issues). Your events can only be found by the install ID, and Writan doesn't show it anywhere, so turning usage data off is the surest way to cut the link between you and what was sent.
