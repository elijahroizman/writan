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

Each time a file is saved, by you or by autosave, Writan keeps a copy of that version. **File › Version History…** (or right-click a tab and pick **Version history…**) lists them by day. Pick one to see what's different: **Compare with now** shows what going back would undo, and **What changed in this save** shows that save's own changes. Text, Markdown and LaTeX files compare line by line; Word documents and PowerPoint presentations compare their text paragraph by paragraph. PDFs and pictures can be opened or restored but not compared.

- **Save version** saves now, with a description if you type one, like a commit message.
- **Restore** puts that version back. What the document was before is kept in the history too, so restoring can be undone.
- **Open copy** opens the old version in a new tab without touching the file.
- Click a version's name to name it. Named versions are always kept.

Every save from the last hour is kept, then one per ten minutes for a day, one per day for a month and one per week after that. Identical versions are stored once. The copies live in Writan's own data folder on this computer, not next to your files. The first time Writan saves over a file, it also keeps the file as it was. **Settings › Autosave** turns version history off, shows how much space it takes, and can delete it all. A file that's moved or renamed outside Writan starts a new history.

## Fonts

Word documents use open-licensed stand-ins for Calibri, Cambria, Arial, Times New Roman and Courier New, with the same widths, so lines break where Word breaks them.

**Settings › Plugins** can add more: Michelangelus, Ubuntu, JetBrains Mono, Atkinson Hyperlegible, Courier Prime, Instrument Serif and Comic Neue. Each downloads when you turn it on and appears in the Word and PowerPoint font menus. See [Plugins › Fonts](plugins.md#fonts).

## Appearance

**Settings › Appearance** picks light, dark or the system's setting (⇧⌘L switches), and a color theme for each. You can make your own themes: two tones and an accent.

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
