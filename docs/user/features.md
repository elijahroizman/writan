# What each editor can do

Every file opens as a tab across the top. Each kind of document gets its own sidebar on the left and an inspector on the right; ⌘\ and ⌥⌘\ hide them.

| Type | Left sidebar | Inspector (right) |
| --- | --- | --- |
| Word | Heading outline, page and word count | Style, font, size, bold/italic/…, color, highlight, alignment, lists, line spacing; insert table, image, link or page break; header and footer with page numbers; comments with replies (rest the pointer on commented text to read one); ⌘-click a link to open it; image size and crop; table rows, columns, shading and borders; page size (A4 for new documents) and margins |
| PowerPoint | Slide thumbnails (drag to reorder, shift- or ⌘-click to pick several, right-click for more); speaker notes under the slide | Insert text boxes, shapes, tables, images, video links and slides; text formatting for one object or several; table styles; fill and line; position, size, rotation, flip and layer order; crop; group and ungroup; links; align; slide layout; transitions; Present |
| Excel | Sheets (add, rename, reorder, delete) | Formula bar; typing `=` lists functions and shows what each takes, and clicking or dragging over cells puts their references in the formula (on other sheets too), Shift-click stretches the last one to a range and ⌘-click (Ctrl-click on Windows and Linux) adds another; charts (column, bar, line, area, pie, doughnut, scatter) made from the selected cells, moved and resized by dragging, with type, stacking, title, cells, colors and legend in the inspector; number format, font, size, bold/italic/underline/strikethrough, color, fill, alignment, wrap, borders; insert and delete rows and columns; merge cells; drag a column or row edge to resize; set how many rows and columns a sheet has in the inspector (saved in the file); pinch or ⌘+ and ⌘− to zoom. CSV files open as one sheet and save as CSV |
| PDF | Page thumbnails (drag to reorder) or the PDF's outline | Highlight, add text, draw, place images; sign with saved signatures and initials, add the date, a check mark or a cross; fill forms (how many fields are filled, jump to the next empty one); save a flattened copy; rotate, move, duplicate, delete, insert and extract pages; document info |
| Markdown | Heading outline | Hidden by default. Read / Edit with preview / Edit source switch at the bottom (⌘E). Front matter, such as an Agent Skill's name and description, shows as a table |
| Text | None | None |

## Find and replace

⌘F finds text in the document in front of you. ⌃H (Ctrl+H on Windows and Linux) opens find with a **Replace with** field, or click the arrow at the left of the find box. **Replace** changes the highlighted match and moves to the next; **All** (⌘↩) changes every one. This works in Word documents, PowerPoint presentations, Markdown and text files. PDFs can be searched but not replaced.

In Word, typing `- `, `* `, `1. ` or `1) ` at the start of a paragraph starts a bulleted or numbered list. ⌘Z takes it back.

## Right-to-left text

Hebrew, Arabic and other right-to-left text runs right to left in Word documents and PowerPoint presentations, mixed with English and numbers the way Word shows it. Right-to-left paragraphs start at the right, with their indents, tabs and list numbers on that side, and a right-to-left table's first column is at the right. In Word, the two paragraph buttons next to the indents make a paragraph left to right or right to left, and typing Hebrew or Arabic into an empty paragraph makes it right to left. The arrow keys move the way the text runs. Word's separate size, bold and italic for Hebrew and Arabic text (and its font for them) show as Word shows them.

## Spelling

Misspelled words get a red wavy underline in Word documents and PowerPoint presentations, and on a Mac or Windows in Markdown and text files too. Right-click one for corrections, **Ignore Spelling** (until Writan quits) or **Learn Spelling** (adds it to your dictionary). Writan uses your computer's spell checker in the language you've set: a Mac's (sharing the words you've taught other apps), Windows', or on Linux the Hunspell dictionaries installed (such as `hunspell-en-au`). On Windows and Linux, words in the Hebrew or Arabic alphabet are checked only when that's the language you've set. In Word the word you're typing is checked once you've moved on from it.

## Signing PDFs and filling in forms

Pick **Sign** in a PDF's inspector. The first time, make a signature: draw it with the trackpad or mouse, type your name in a handwriting style, or use a picture of it (Writan can make the white paper see-through). Make initials the same way. They're kept on this computer only, and you can have a few of each.

Pick one and click on the page to place it; it's selected afterwards, so you can drag it into place or resize it from the corner. **Date** adds today's date as text, and the check mark and cross are for boxes that aren't real form fields. A PDF's own signature fields show **Sign here**: click one and your signature fills it.

PDFs with a form get a **Form** section: how many fields are filled, **Next empty** to jump to the next field you haven't filled in, and **Show fields** to tint every field. **Save flattened copy…** writes a copy where the form's answers, your signature and any marks are part of the page, so whoever gets it can't change them. The PDF you're editing stays as it was.

Placing a signature puts a picture of it on the page. It isn't a cryptographic (certificate) signature.

## Vector PDF export and offline text recognition (OCR)

Word documents, presentations, workbooks and Markdown notes export directly as crisp vector PDFs with selectable text, embedded fonts, vector equations and charts, and clickable web and `mailto:` links.

When you open a PDF whose pages look like scanned images, a banner offers to run offline OCR right on your computer. You can choose which pages to scan (current page, scanned pages only, or all pages), skip pages that already have selectable text, and select any of 12 downloadable language packs in **Settings › General › Offline OCR** or directly in the OCR dialog. Language packs download once from a pinned release with SHA-256 verification and are stored locally for offline use; recognition itself never sends documents over the network.

OCR saves a separate copy and leaves the original file untouched. If a PDF carries a cryptographic digital signature, Writan asks for confirmation first and strips the signature from the separate searchable copy while leaving the signed original unchanged.

You can also open PNG, JPEG, HEIC and WebP pictures in the Images viewer, copy recognized plain text or Markdown, convert PDFs or images into editable Word (`.docx`) documents via **Convert file…**, or save an image as a searchable PDF. Use **File › Recognize Text (OCR)…** or the command palette (`⌥⌘O` on a Mac, `Ctrl+Alt+O` on Windows and Linux).

## Tabs

Open as many files as you like and switch with ⌘1–⌘9 or ⇧⌘[ / ⇧⌘]. Drag tabs to reorder them. Right-click a tab to duplicate it (⇧⌘D), for example to keep a textbook's questions and answers side by side. Open tabs come back the next time you launch. A restored tab loads its document the first time you look at it.

The Home tab's **Recent** list has the files and folders you've opened, and its filters show only Office files, notes (Markdown and text), PDFs, images or LaTeX. Each drops off a few days after you last opened it: 3 days unless you pick 1, 7 or 30 in **Settings › General**.

## Command palette

⌘K opens a search box for everything: commands (with their shortcuts), open tabs and recent files. Type a few letters of what you want (“dark”, “new markdown”, “zoom”) and press return. Commands that need a document, such as Find or Zoom in, are greyed out on Home. Start with `>` to search commands only. Commands for the document in front of you, such as Word's Add comment or PowerPoint's Play from start, are listed while that tab is active. It's also in the View menu.

## Focus mode and typewriter scrolling

**View › Focus Mode** (⌥⌘F) hides the tabs, sidebars, inspector and the bar at the bottom, leaving only the document. Move the pointer to the top of the window to see the tabs and focus mode's controls; leave it with ⌥⌘F, Esc or **Exit**. In Word documents, Markdown and text files, every paragraph but the one you're writing fades (turn that off with **Dim**).

**Typewriter scrolling** (⌥⌘T) keeps the line you're typing at the same height on screen, a little above the middle, so your eyes stay put while the page moves. It works with or without focus mode, and Writan remembers whether it's on.

## Folders

**File › Open…** (⌘O) opens a folder as well as files: on a Mac, pick the folder in the panel; on Windows and Linux, go into it and click **Open Folder**. It opens in a sidebar to the left of everything, like a project in a code editor. It lists the folder's documents, with subfolders you can expand; click a file to open it in a tab. Files Writan can't open are hidden unless you pick **Show all files** from the sidebar's **⋯** menu. Hidden files and folders such as `.git` and `node_modules` are always left out.

Type in **Find a file** to search every document under the folder by name. ⇧⌘E hides and shows the sidebar. The folder (and which subfolders were expanded) comes back the next time you open Writan, and Home's Recent list has the folders you've opened, under All and Folders. Dropping a folder on the window opens it too. The list refreshes when you come back to Writan from another app; the **⋯** menu also has **Refresh**, **Collapse all folders** and **Close folder**. Drag the sidebar's edge to make it wider.

## Split view

**View › Split View** (⇧⌘\) shows two Word documents or PDFs side by side. It asks what goes beside the one in front: another open tab, a recent file, a file from the open folder, **Choose a file…**, or a second copy of the same document, so you can keep a textbook's questions on one side and its answers on the other. Right-click a tab and pick **Open to the side…** to put it beside the document you were looking at. Markdown and LaTeX have their own source-and-preview split instead (⌥⌘2), which ⇧⌘\ switches to.

Click in a pane to work in it: a blue line along its top shows which one, and Save, Find, Zoom and the other commands go to it. Clicking another Word or PDF tab in the tab bar puts it in that pane; any other tab goes back to one pane. Drag the line between the panes to resize them (double-click it for half and half). Each pane starts without its sidebar and inspector, to leave room for both documents; the buttons at its top corners show them in that pane. ⇧⌘\ again, or closing either tab, goes back to one pane, with the sidebars as they were.

## New files

⌘N asks which kind of file to make, from the plugins you have on. The Home tab has a button for each too. New Word documents are A4.

## Autosave

A file you've already saved is written back to the same place a few seconds after you stop editing. That place can be any folder, including one iCloud Drive, Dropbox, OneDrive or another sync app watches; the sync app uploads it. Writan doesn't sign in to anything. A new document isn't saved until you use Save.

In **Settings › Autosave** you can turn it off, change the delay, or leave some file types on manual save. Saving doesn't move a PDF: it stays on the page you're reading. While you're typing into a PDF form field or a text box, or drawing, autosave waits until you finish.

## Version history

Each time a file is saved, by you or by autosave, Writan keeps a copy of that version. **File › Version History…** (or right-click a tab and pick **Version history…**) shows a version as the document itself, with a timeline of every version along the bottom. Drag along the timeline, click it, or press ← and → to scrub through them. Word documents show their pages, presentations their slides, workbooks their sheets (switch sheets along the bottom; you can select and copy cells but not change them), PDFs their pages, Markdown as it reads, and text and LaTeX files as they are. HEIC pictures can be opened or restored but not shown.

- **Restore** puts that version back. What the document was before is kept in the history too, so restoring can be undone.
- Click a version's name to name it. Named versions are always kept, and show as darker dots on the timeline.
- **⋯** opens the version as a copy in a new tab without touching the file, or deletes it.
- **Save version** appears when the document has unsaved changes, and saves it now.

Every save from the last hour is kept, then one per ten minutes for a day, one per day for a month and one per week after that. Identical versions are stored once. The copies live in Writan's own data folder on this computer, not next to your files. The first time Writan saves over a file, it also keeps the file as it was. **Settings › Autosave** turns version history off, shows how much space it takes, and can delete it all. A file that's moved or renamed outside Writan starts a new history.

## Sharing

Several people can edit a Word document, presentation or workbook at the same time, from wherever they are. Click the people button at the right of the title bar (or **Share…** in the command palette or a tab's right-click menu), type the name others will see, and press **Share**. A document that isn't saved yet is saved first (**Save and Share**), since everyone edits the file with you. On a Mac, macOS asks the first time whether Writan may find devices on your local network; choose **Allow**. The panel shows a six-digit code. Give it to the others. They click the people button on Home (or **Join a shared document…** in the command palette), type the code and press **Join**. With **Ask me before letting someone in** on (on unless you turn it off), a note pops up in the top right corner for each person who types the code, and they join once you click **Let in**. The document opens in a new tab on their computer.

A code works for ten minutes, which the panel counts down. That's only for joining: nobody who's in gets taken out when it runs out. **New code** gives another ten minutes with a new code, and the old one stops working. Writan connects the two computers directly when it can, and otherwise through a relay on the internet; either way what's sent is encrypted between them. Writan's code directory, on Cloudflare, keeps only which computer has which code. When Writan can't reach the internet, the code works only on your network, where Writan finds your computer, and the panel says so. Some office and guest networks keep computers from seeing each other, and there a code for your network can't find you.

Everyone's changes show on everyone's screen as they're made, and so does where everyone is: each person has a color, their caret and selection show in it with their name, and in a presentation or workbook the slide or sheet list has a dot in their color where they are. The name shows while they move and fades after a few seconds; point at the top of their caret to see it again. The person sharing keeps the file. Their tab saves and autosaves it as usual, with everyone's changes. Everyone else's tab shows **On <name>'s computer** where the save status usually is, and has nothing to save or lose when it closes. **Save As** keeps a copy of your own. When two people change the same paragraph, slide or cell at the same moment, the first change counts and the second is undone. Undo takes back only your own changes.

**Stop sharing** in the same panel ends it for everyone, and so does closing the tab or quitting. The others are told you stopped, and their tab closes, unless they choose **Keep a Copy** to go on with what they had as an unsaved copy. The panel lists who's in the document. Anyone with the code can ask to join while it works, so give it only to people you mean to. AI agents don't need the code: they join through Writan's MCP server (below). An agent on the computer of someone who joined asks to join like a person does, and you let it in the same way.

A note in the top right corner, under **Share**, says when someone joins or leaves. It stays a few seconds, and clicks go through it.

## AI agents

AI agents can join a Word document, presentation or workbook you have open, shared or not, and edit it with you, the way a person would: everyone sees the agent's caret, its changes, and its name and logo. You see your own agent as Claude; the people you share with see it as yours, like Ada's Claude, and you see theirs that way too. **Let agents into your documents** in **Settings › AI** is on unless you turn it off. The page gives an address, with a secret in it, that agents connect to through MCP. Messages between agents are on unless you turn them off too.

For an agent on this computer, **Add MCP server** sets it up in Claude Code, Codex, Antigravity, Grok Build, Hermes Agent or Pi, and **Add skill** adds Writan's skill, which tells the agent how to work alongside people. Start a new session in the agent afterwards. Pi needs version 0.99 or later, which reads MCP servers itself; it finds Writan's tools with `tool_search` when a conversation needs them. Pi and Codex read the same skill folder, so they share one copy of the skill. Any other agent that takes a remote MCP server can use the address too, and **Copy skill** copies the skill to add by hand. For an agent on another computer on your network, give it the address under **From another computer**.

Then ask the agent to work on a document. One that isn't shared stays between you and your agents, and the share button shows who's in it; sharing it later keeps them in. When the first agent joins a document, the document's undo history starts over, as it does when you share it. It works like another person in the document. It can read it, see slides and pages as pictures, see where everyone is, and wait for someone else's changes and see what they changed. In Word it can write, delete, style and format paragraphs, make lists and tables, and put in, resize and crop pictures. In presentations it can write and format text, move, resize, turn, crop and restack pictures and shapes, add text boxes, shapes, tables and pictures, and add, copy, move, hide and delete slides. In workbooks it can write and format cells, size, insert and delete rows and columns, merge cells, add charts and manage sheets. It can find text, undo and redo its own changes, and each change tells it what it did. On this computer it can also open a file or a new document, and save one that isn't someone else's. An agent on another computer can only send pictures it has; only one on this computer can have Writan open a file or download a picture. It can only work on documents open in Writan, while Writan is open. Anyone with the address can edit them; **New address** makes a new secret, and agents set up with the old one stop connecting.

Agents in the same document, yours and the agents of people you share with, wherever each of you is, talk to each other in the document's chat: to split up the work, or say what they're about to change. Everyone in the document sees the same chat. Once there's something in it, a chat button appears beside the share button, with a dot when there's a message you haven't seen; nothing pops up for each message. Your agent treats another agent's message as a request, not as yours, and asks before deleting or rewriting a paragraph someone else wrote. In Word, agents name paragraphs by short ids like #k3f9x2, which stay the same while others add paragraphs above. Click one in the chat to go to that paragraph. An agent that joins later sees who wrote each paragraph, including people and agents who have left, and the messages about each one. Turn off **Let agents message each other** in **Settings › AI** and your agents neither send nor get messages; other agents are told yours have messages off.

## Fonts

Word documents use open-licensed stand-ins for Calibri, Cambria, Arial, Times New Roman and Courier New, with the same widths, so lines break where Word breaks them. Where the computer has the real font, documents use it. Linux doesn't come with Microsoft's fonts, so unless you install them yourself the stand-ins always show: Liberation Sans, Serif and Mono for Arial, Times New Roman and Courier New, and Carlito and Caladea for Calibri and Cambria.

## Appearance

**Settings › Appearance** picks light, dark or the system's setting (⇧⌘L switches), and a color theme for each: click a theme's sun or moon to use it for just that appearance. **Change theme…** in the command palette (⌃⌘T on a Mac, Ctrl+Alt+A on Windows and Linux) picks one without opening Settings.

### Your own themes

**Create theme** opens the theme editor beside the app, starting from the theme you're using, and the whole window shows your changes as you make them. A theme is an accent and two tones, one for light and one for dark, that tint every surface, border and text. Turn on **Every color** to set any single color for one appearance, like the sidebars or the selection. The pointer button picks a color from the app: click anything in Writan and the editor jumps to the color that paints it. **Custom CSS** changes what colors can't, after Writan's own styles. It can't load files: `@import`, `image-set()` and `url()` other than `data:` are left out, so a theme someone sends you can't send what's on your screen anywhere.

A theme's **⋯** menu edits, duplicates, exports (as a .json file) or deletes it.

### Adding themes

**Add theme** searches community color themes on [Open VSX](https://open-vsx.org), the open registry of VS Code extensions, and adds the ones with an open license. Writan takes their window colors: one with a light and a dark version becomes one theme with both. It also adds theme files: ones exported from Writan or [T3 Code](https://github.com/pingdotgg/t3code), and VS Code color themes.

### The themes folder and agents

Themes saved as `<name>.json` in the themes folder show up in Settings as you save them (**Show folder** opens it). A JPEG, PNG or WebP picture of up to 16 MB beside one, like `dusk.jpg` beside `dusk.json`, becomes its background, which its CSS shows with `var(--theme-background-image)`. A theme file looks like this:

```json
{
  "name": "Dusk",
  "accent": "#7aa2f7",
  "light": "#dfe3f5",
  "dark": "#1a1b26",
  "variants": { "dark": { "panel": "#16161e" } },
  "css": ".titlebar { border-bottom: 1px solid var(--accent); }"
}
```

An agent on this computer with Writan's MCP server can restyle Writan for you: ask it, for example, to "give Writan a solarized theme". It publishes a theme to the themes folder and switches Writan to it. Ask it to go back to Writan's own theme to undo it.

This all follows [T3 Code](https://github.com/pingdotgg/t3code)'s themes.

**App icon** offers Dark (the default), Blue and Light. On macOS, it changes the Dock icon while the app is open. Finder always uses the default, as does the Dock when the app is closed. On Windows and Linux, it changes the window and taskbar icon. The choice is saved and applied each time the app opens.

## Command line

Like `code` for VS Code, `writan` opens things from a terminal:

```sh
writan .              # this folder, in the folder sidebar
writan notes.md       # a file; a new Markdown, text or LaTeX file is created empty
writan a.pdf b.docx   # several files, each in a tab
```

On a Mac, install it from **Settings › General › Command line** or **Writan › Install Command Line Tool…**. It goes in `/usr/local/bin`, which usually asks for your password. It finds Writan by its app ID, so moving the app doesn't break it. On Debian and Ubuntu the .deb puts `writan` on the PATH already. `/usr/bin/workspace` is a wrapper that runs `writan`, so an update started by the old command can restart the app. On Windows and Linux, running it while Writan is open hands the files to the open window instead of starting a second copy.

## Default apps

The welcome tour on first launch ends by asking which file types Writan should open, and **Settings › General › Default apps** can change that later. On Windows, each one is confirmed in Windows' own Default apps settings. Writan also appears in every Open With menu.
