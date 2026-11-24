# Changelog

Every release's notes. The release workflow copies a version's section into its GitHub release, and the app's update panel shows it.

## 0.15.0 — 2026-11-24

### Edit together
- When someone types your code and you let people in yourself, a note pops up in the top right corner with **Let in** and **Deny**, whatever you're doing. You no longer need the Share panel open.
- A code only stops new people from joining after ten minutes. Everyone already in stays, and the Share panel now says so.

### AI agents
- An agent can work in any Word document, presentation or workbook you have open, not just shared ones. One that isn't shared stays between you and your agents, and the share button shows who's in it. Sharing it later keeps them in.
- Someone who joined your document can bring their own agent. It asks to join like a person, and you let it in from the same popup, even after the code has run out.
- Everyone else's agents show with their person's name, like **Yoav's Claude**. Your own agents are still just **Claude** to you.

## 0.14.0 — 2026-11-23

### Edit together
- The six-digit code now works over the internet, not just on your network. The code lasts ten minutes, and the Share panel counts it down; **New code** makes a fresh one and the old one stops working. With **Ask me before letting someone in** on, you see each person who types the code and choose **Let in** or **Deny**. Without an internet connection, the code works on your network as before.
- A code from someone on Writan 0.13 or older doesn’t work here. Join with their link instead.

### AI agents
- Settings › AI sets up Writan in [Pi](https://pi.dev) too (version 0.99 or later). Pi joins shared documents under its own name and logo, like the other agents. Pi and Codex share one copy of Writan's skill.

### Presentations
- The arrow keys move to the next or previous slide in the editor, not only while presenting, when no object is selected. Home and End go to the first and last slide, and Page Up and Page Down turn slides even with an object selected.

### Tabs
- With many files open, tabs keep their width and scroll sideways instead of shrinking to icons, and Home keeps its name.

## 0.13.0 — 2026-11-23

### Convert and export
- **File › Convert File…** turns the open document into another format: Markdown, PDF, Word, PowerPoint, Excel, CSV or plain text, depending on what it is. Open the result in a new tab or save it to disk.
- **Copy as Markdown (for LLMs)** and **Export as Markdown (.md)** give clean headings, lists and tables to paste into an AI chat. **Export as PDF** saves the rendered pages, charts and pictures included.
- **Bundle Open Files to PDF…** merges the open Word, PowerPoint, Excel, Markdown, image and PDF files into one PDF, in the order you pick.

### Linked Excel
- The **Excel** button in Word and Presentations inserts a range or chart from an open Excel workbook and keeps it linked. It updates whenever the workbook changes, or when you click **Update now** in the **Linked Excel** section of the Inspector, which also opens the source workbook and unlinks.

### Equations
- **Insert LaTeX math** (⌥=) types an equation in LaTeX with a live preview and puts it inline in a Word document or on a slide. To change it, double-click it in Word, or right-click it on a slide and pick **Edit equation**. It's saved as a native equation, so Word and PowerPoint can edit it too.

## 0.12.6 — 2026-11-22

### Edit together
- When the person sharing stops, closes the document or quits, everyone else is told, and their tab closes. **Keep a Copy** keeps it open as an unsaved copy instead.
- A person's name sits on top of their caret like a flag, without the little square beside it. The square shows once the name fades, to bring it back.

### Presentations
- **New slide › Title Slide** and **Title and Content** add the same plain slides a new presentation starts with.

## 0.12.5 — 2026-11-22

### Edit together
- Sharing and joining work on Windows. Before, a Windows computer couldn't join a shared document ("WebSocket protocol error: Sending after closing is not allowed"), and a document shared from one never connected.

## 0.12.4 — 2026-11-22

### Edit together
- **Join a shared document…** now says it takes the link as well as the code, so the link from **Copy the link instead** has somewhere to go. Use it when the code can't find the person sharing, for example on another part of an office network.

## 0.12.3 — 2026-11-22

### Edit together
- Sharing gives a six-digit code instead of a long link. The others type it into **Join a shared document…**, and Writan finds your computer on the network. **Copy the link instead** is still there for networks where the code can't find you.
- The note that says someone joined or left is bigger, and shows at the top right under the share button.

### AI agents
- Agents don't need a code: they join shared documents through Writan's MCP server, as before.
- In workbooks, agents can change a chart's title, axis titles, kind, legend, range and colors, and see a sheet, a chart or a range of cells the way people do.
- Settings › AI can remove Writan from an agent, and Writan keeps the setup in agents that already have it up to date when it starts.

### Version history
- Every saved version shows as the document itself: Word pages, every slide, a workbook's sheets, PDFs, Markdown, text and pictures. Before, workbooks, PDFs and pictures couldn't be shown, and the rest showed a diff of their text.
- Opening version history takes the keyboard, so typing no longer changes the document behind it.

### Settings
- The downloadable fonts are gone, and so is the Plugins page: the LaTeX switch is on **Settings › General**. Documents still use the look-alike fonts, or Microsoft's when they're installed.

## 0.12.2 — 2026-11-21

### AI agents
- Agents can see what they're working on: a slide or a Word page comes to them as a picture, with each shape on a slide marked with its id, so they can tell which photo is which and see what overlaps.
- In presentations, agents can move, resize, turn, flip, crop and restack pictures and shapes, format text, fills and outlines, add pictures, text boxes, shapes and tables, and copy, group and delete them. They can also copy, move, hide and delete slides and change their layout.
- In Word documents, agents can format text and paragraphs, make lists, add and change tables, and add, resize and crop pictures. In workbooks they can format cells, size, insert and delete rows and columns, merge cells, add charts, and add, rename, move and delete sheets.
- Each change an agent makes tells it what changed. Waiting for someone else's edit tells it which paragraphs, slides and shapes changed.
- Only an agent on your computer can have Writan open a file or download a picture. One on another computer sends the picture itself.
- Agents no longer lose their connection to Writan when the window reconnects.

### Presentations
- **New presentation** opens a plain deck again, with a title slide and a content slide. The starter-deck gallery is gone.
- Cropping a picture to fill its frame without moving it now sticks.

### Documents
- A picture you just put in can be cropped. Before, cropping it only squashed it.

## 0.12.1 — 2026-11-21

### AI agents
- AI agents can now join a document you share and edit it with you, like another person: everyone sees each agent's caret, changes, name and logo. Turn it on in **Settings › AI**.
- Settings › AI adds Writan to Claude Code, Codex, Antigravity, Grok Build or Hermes Agent in one click, with a skill that tells the agent how to work alongside people. Any other agent that takes an MCP server can use the address it shows, from this computer or another one on your network.
- An agent can read the document, see where everyone is, add, change and delete paragraphs, set styles, replace text, add and fill in slides, write cells and formulas, undo its own changes, and wait for someone else's.

### Edit together
- You can see where everyone is: each person's caret and selection show in their own color with their name, and in presentations and workbooks the slide or sheet list marks where each person is. Your color is the same on everyone's screen.
- A note in the top left corner says when someone joins or leaves.
- A document is saved before it's shared, since everyone edits the file with you.
- On a Mac, Writan asks for access to your local network when you share or join, and waits while macOS asks you.
- A document you joined no longer offers to save or autosave, since the file is on the other person's computer. Closing it or quitting doesn't ask, once your changes have reached them. **Save As** keeps a copy of your own.

### Presentations
- **New presentation** makes a deck right away from your default starter. Templates are one step away: **Templates** on Home, or **New presentation from a template**, in a gallery you can search.
- On Linux, slide thumbnails no longer show white corners.

### Workbooks
- Charts show their axis titles, you can edit them, and they're kept when you save.

## 0.12.0 — 2026-11-21

### Edit together on your network
- Several people can now edit one Word document, presentation or workbook at the same time from computers on the same network. Click the people button in the title bar, press **Share** and send the link. Others join from the people button on Home, or **Join a shared document…** in the command palette.
- Everyone's changes show on everyone's screen as they're made. The person sharing keeps the file and saves it as usual, with everyone's changes. When two people change the same paragraph, slide or cell at the same moment, the first change counts.
- It's a first version: comments don't sync, you can't see the others' cursors, a copy whose connection drops stops following, and the connection isn't encrypted, so share only on a network you trust.

### Version history
- **File › Version History…** shows a version as a page with its changes marked in place, Word headings and tables included, and a timeline of every version along the bottom. Drag along it, click it or press ← and → to scrub through your saves.
- It opens on what the latest save changed. **Compare with now** shows what going back would undo, and an edited phrase is marked as one change rather than word by word.

### Faster
- Formatting cells in a big workbook no longer recalculates every formula.
- Typing in long Word documents and large presentations does much less work per keystroke, and saving them no longer copies the whole document first.
- Reading and saving files no longer holds up the window, and a restored session opens its files in parallel.

### Updates
- The update button in the title bar is now just an icon, and its panel lists the changes in every release you've missed, not only the newest.

## 0.11.4 — 2026-11-20

### Look and feel
- On Windows and Linux the interface uses Inter, a typeface close to the Mac's San Francisco, instead of Segoe UI or the desktop's default font.
- The inspector beside Word, PowerPoint and Excel documents is calmer: section headings sit a step below the panel's title, collapsible sections line up with the rest, and Insert tiles are outlined.
- On Home, Open… is a drop zone you can drag files onto, Recent's dividers show in the light theme, and each recent file's More menu works from the keyboard.
- Menus keep red items such as Delete slide readable when hovered or focused, and focus rings are easier to see in every theme.
- Check for updates has a new icon.

### Fonts on Linux
- Setup and Settings › Plugins › Fonts offer Microsoft's core fonts (Arial, Times New Roman, Courier New, Georgia, Verdana and more) once you accept Microsoft's license, so documents use the real fonts instead of look-alikes. Calibri and Cambria can't be downloaded, so they keep their look-alikes, Carlito and Caladea.

### Word
- The page counter follows the page you've scrolled to, not the page the cursor is on.

### Windows
- Settings › Default apps works again: the Make default buttons are no longer greyed out, Writan reads where Windows 11 24H2 keeps your choice, and a button opens Windows' Default Apps.
- Spell check uses Australian or British English where that's your region before falling back to US English.

### Saving
- An untitled document's title bar offers Save to enable autosave instead of only saying it isn't autosaved.

## 0.11.3 — 2026-11-20

### Word
- New bulleted and numbered lists are indented as Word indents them, so the text no longer jumps half an inch past the bullet or number.

### PowerPoint
- New presentation offers five starter decks (Plain, Harbor, Graphite, Signal and Lagoon) beside Blank.
- New slide layouts set titles in the theme's heading font, and text on accent panels is dark or light, whichever reads better.
- Pressing Enter in a bulleted list on a slide keeps the list going, and the new empty line shows its bullet.
- Linked videos play in front of the slide in the slideshow, and double-clicking a video, or Play video in the Link section, plays it while editing.
- Delete and Backspace remove a selected video frame instead of typing into the empty link field.

## 0.11.2 — 2026-11-20

### Word
- Selecting all the text in a footnote or endnote and typing over it keeps the note's number mark instead of deleting it.

### PowerPoint
- Insert › Shape › Arrow draws an arrow instead of a rectangle.

### Spreadsheets
- Formulas understand Excel table references such as `Table1[Column]` and `Table1[#Totals]`.
- Inserting or deleting rows and columns in an Excel table keeps its calculated-column and totals formulas pointing at the right cells.
- Numbers in cells formatted as Text show instead of rendering blank.

### PDF
- Text notes on rotated pages wrap across the whole note instead of clipping into narrow columns.

## 0.11.1 — 2026-11-19

### Word
- Footnotes and endnotes too long for their page continue onto the next one.
- Footnote and endnote text can be edited in place.

### PowerPoint
- Pasted charts, and charts that were moved or resized, are kept when the presentation is saved.

### Spreadsheets
- Renaming a worksheet keeps formulas, defined names, conditional formatting, data validation, chart series, links and table and pivot sources pointing at the new name.

### Windows
- Shapes draw again: a packaging detail kept the preset shapes from loading on Windows.

## 0.11.0 — 2026-11-19

### Install and updates
- The Mac app is now universal: one download, `Writan_<version>_universal.dmg`, runs natively on Apple silicon and Intel Macs.
- Linux packages now come for ARM as well as x64: the Debian/Ubuntu `.deb`, the Fedora `.rpm` and the Arch `.pkg.tar.zst` all have `arm64`/`aarch64` builds.
- Arch Linux has a Writan repository now, signed like the packages, so `pacman -S writan` installs and upgrades Writan once your `pacman.conf` has the `[writan]` section the package adds.
- The apt, dnf and pacman repositories carry both architectures at once, so ARM Linux machines get updates the same way x64 ones do.

### Reliability
- After a crash or a reload, Writan reopens your unsaved edits and untitled tabs automatically.

### Word
- Footnotes show at the bottom of the page their reference is on, and endnotes show after the document's last line.
- Office Math equations are drawn instead of leaving a blank space.
- Tracked changes can be accepted or rejected all at once from the inspector.
- Block content controls saved in a document show their contents.
- SmartArt shows as a diagram instead of a blank page or an empty box.

### PowerPoint
- Charts saved in a presentation render from their cached data.
- SmartArt shows as a diagram instead of a blank page or an empty box.
- The slideshow plays object animations, and its text builds a paragraph at a time.

### Spreadsheets
- Conditional formatting, data validation dropdowns and pasted pictures show on the sheet, and move along when rows or columns are inserted or deleted.
- Formulas saved by Excel without a cached result, and string formulas saved with an empty one, show their result instead of a blank cell.
- Moving rows or columns keeps other sheets' formulas and defined names pointing at the same cells.
- A circular reference warns instead of saving a made-up value.
- Right-clicking a row or column header no longer scrolls to the last cell.
- Renaming a worksheet tab works again.

### PDF
- Flattened copies keep highlights that were saved without the print flag, and rotating or moving a page keeps the selection on the page you clicked.

## 0.10.10 — 2026-11-19

### Word
- Pressing Return in a numbered or bulleted list continues the list instead of starting a new one at 1, and the paragraph after a list no longer keeps the list's indent.
- Choosing a style such as **Heading 1** on a new document applies it right away, and the heading shows in the outline, without closing and reopening the document.

### PowerPoint
- Copy, Cut and Paste work on selected shapes and text on a slide, from the keyboard and the Edit menu.
- Deleting all the text in a title keeps its font and size, so new typing matches instead of turning into 18 pt Calibri.
- A duplicated slide keeps its speaker notes, and editing one copy's notes leaves the other's alone. Pasted shapes from imported presentations keep their outline and text position.

### Spreadsheets
- Copying or cutting a cell with a formula pastes the formula, not its result. A copy shifts its relative references, as in Excel.
- Cutting cells to another place or sheet keeps the formulas that use them pointing at the moved cells, and the moved formulas keep reading their original inputs.

### PDF
- **Save flattened copy…** works on PDFs with pages that have no annotations, and keeps highlights.

## 0.10.9 — 2026-11-16

### Linux
- Writan has an Arch Linux package for ARM: `writan-…-aarch64.pkg.tar.zst`, for Arch Linux ARM, Asahi Linux and Omarchy on ARM machines such as Apple silicon Macs. Install it with `sudo pacman -U`, as with the x86_64 package.
- On ARM Linux, LaTeX finds a TeX Live installed under `/usr/local/texlive` (its `aarch64-linux` folder).

## 0.10.8 — 2026-11-16

### Linux
- Writan has an Arch Linux package, `writan-0.10.8-1-x86_64.pkg.tar.zst`, on the release page. Install it with `sudo pacman -U`. Arch has no Writan repository yet, so new versions are installed by hand the same way; the Check for updates button shows the release notes.

## 0.10.7 — 2026-11-12

### Spelling
- Misspelled words in Word documents and PowerPoint presentations get a red wavy underline. Right-click one for corrections, **Ignore Spelling** or **Learn Spelling**. Writan uses your computer's spell checker in your language: the Mac's, Windows', or on Linux the installed Hunspell dictionaries. Word and PowerPoint text is drawn by Writan itself, so the system's checker never saw it before; this is why nothing was underlined.

### LaTeX
- Typing `\` lists LaTeX's commands, as VS Code's LaTeX extension does: `\frac{}{}`, `\section{}`, `\alpha` and the rest. Tab or Return puts one in, and Tab moves between its braces. `\begin{` lists environments and adds the matching `\end`; `\ref{` lists the document's labels. The LaTeX plugin updates itself to 1.1.0 when Writan starts.

### Word
- A selected picture shows just its frame and handles, like a picture in a presentation. The highlight no longer reaches above it.

## 0.10.6 — 2026-11-08

### PDF
- Selected text no longer stays highlighted after switching to another tool (Text, Draw, Sign) or clicking the page with one, or after clicking between pages.
- Scrolling away from a selection and back no longer highlights whole pages of text you didn't select.

### Presentations
- Legacy PowerPoint `.ppt` files open in the presentation editor. Saving one saves a `.pptx` copy (asking before replacing an existing file) and never overwrites the `.ppt`. Autosave starts once the deck is saved as `.pptx`.

### Office documents
- A Word, PowerPoint or Excel file whose main part uses an unrecognised XML namespace now shows an error instead of opening empty and losing its contents when saved. Strict Open XML files still open.
- Page and column breaks in Word documents keep their text formatting when saved.
- Deeply nested slide groups and large documents open with fewer copies of their XML in memory.

## 0.10.5 — 2026-10-24

### Safer saves
- Unchanged Excel workbooks keep imported formula results, and unchanged PDFs keep their original bytes and digital signatures. Undoing PDF edits restores the original bytes too.
- Text and Markdown saves preserve UTF-16, UTF-8 byte-order marks, Windows-1252, and untouched line endings. Saving edits to malformed BOM-marked text stops with an explanation before replacing invalid bytes.
- Saving asks before replacing changes from another app or tab, including changes made while a large document exports. Concurrent saves are serialized, and failed native saves preserve the existing file.
- Quitting from the macOS Dock checks the current unsaved edits before exiting.

### Documents and spreadsheets
- Word table rows, checklist cells, content controls, and column grids retain their order and structure when saved. Traditional Chinese and zodiac list markers render correctly.
- Hidden worksheets stay hidden on open. Empty TSV files open like empty CSV files.
- Damaged or mislabeled Office files show an error. Oversized or deeply nested XML, extreme row indices, amplified shared strings, long formulas, and malformed font names are handled without the crashes found in the stress tests.
- Document links are restricted to web and email addresses.

### Linux packages
- The Debian and RPM packages are now named `writan`. Existing `workspace` installations migrate through apt or dnf; the old `/usr/bin/workspace` command forwards to `writan` for compatibility.
- Package repository files and key paths use the Writan name while retaining the existing signing key. Repacked Debian files keep root ownership.

The stress report and reproducible regression samples are in [tests/stress/BUGS.md](https://github.com/elijahroizman/writan/blob/v0.10.5/tests/stress/BUGS.md). Unsaved crash recovery and the rendering gaps listed there remain open.

## 0.10.4 — 2026-10-23

### Spreadsheets
- Choose how many rows and columns a sheet has: **Rows and columns** in the inspector has a field for each, up to Excel's 1,048,576 rows and 16,384 columns. The size is saved in the file, inserting or deleting rows and columns changes it to match, and it never cuts off cells you've used. Excel opens these files as usual.
- Pinch the trackpad to zoom the sheet.
- The Zoom section from 0.10.3 is gone from the inspector, and spreadsheets open at 100% again.

## 0.10.3 — 2026-10-23

### Spreadsheets
- **Zoom** in the inspector makes the sheet bigger: 100%, 125%, 150% or 175%. Every spreadsheet opens at the last zoom you picked, whether in the inspector, with the buttons under the sheet or with ⌘+ and ⌘−.
- New charts are back to 5 by 3 inches, and the chart inspector's Size section is gone.

## 0.10.2 — 2026-10-23

### Spreadsheets
- Building a formula by clicking cells works for as many references as you like. Before, typing a comma or operator after a clicked reference could put it in the wrong place, and the next click left the formula instead of adding the cell.
- New charts are bigger: 7 by 4 inches. The chart inspector has a **Size** section to set the width and height in inches.
- The **Chart** section sits under **Alignment** in the inspector.

## 0.10.1 — 2026-10-23

### Spreadsheets
- While you type a formula, the hint that shows what a function takes no longer gets in the way of the cells under it. Clicking them puts their references in, and the hint fades while the pointer is over it. Before, you could pick only the one or two cells beside it.
- Shift-click stretches the reference you just put in to a range, and ⌘-click (Ctrl-click on Windows and Linux) adds another after a comma, as in Excel.
- **Insert chart** sits near the top of the inspector, under the number format.

### App icon
- The Blue and Light icons are the same size as the default in the Dock and taskbar, with its margin and shadow. Before, they filled the whole space and looked bigger than the apps around them.

### Welcome tour
- The tour's pictures are screenshots of Writan, in light or dark to match the app, instead of drawings. The command palette's shows this computer's shortcuts.

## 0.10.0 — 2026-10-23

### Workspace is now Writan
- The app has a new name. Google's office suite is also called Workspace; Writan is ours. The icon and the look are unchanged.
- Updating keeps your settings, recent files, plugins and default apps. On a Mac the app keeps its place in Applications (a fresh install is Writan.app). On Windows it updates in place and the old shortcuts are replaced. On Linux the package is still `workspace`, so apt and dnf update it as before, and the command is now `writan`.
- **Settings › Appearance › App icon** picks the dark tile (the default), a blue one or a light one.

### Excel files and CSV
- Word, PowerPoint and Excel files now run on Writan's own engine, written in Rust. Excel workbooks and CSV/TSV files open in a new spreadsheet editor: sheets, a formula bar, number formats, fonts, fills, borders, alignment, merged cells, row and column insert and delete, and resizing.
- **Charts.** Select some cells and use **Insert chart** for a column, bar, line, area, pie, doughnut or scatter chart, made the way Excel would from the block. Drag to move or resize it; the inspector changes its type, stacking, title, cells, colors and legend. Charts in files from Excel show and stay as they were unless you change them.
- **Formulas like in Excel.** Typing `=` lists the functions, with what each one takes and a line on what it does; Tab puts the highlighted one in. While you type a formula, clicking or dragging over cells (on any sheet) puts their references in, and each reference gets its own colored box. Brackets left open close when you press Enter.
- About sixty more functions: SUMIFS, COUNTIFS, AVERAGEIFS, MAXIFS, MINIFS, IFS, SWITCH, XLOOKUP, XMATCH, MEDIAN, MODE, STDEV, VAR, LARGE, SMALL, RANK, EDATE, EOMONTH, DATEDIF, NETWORKDAYS, WORKDAY, TEXTBEFORE, TEXTAFTER and more. Newer ones are saved the way Excel expects.

### Home
- Open sits beside a New panel that lists every type you can make, so nothing gets squashed when LaTeX is on.
- Recent's filters are plain pills without colors. Office holds Word, PowerPoint, Excel and CSV/TSV files; Notes holds Markdown and text; PDF; Images holds PNG, JPEG and HEIC; LaTeX holds .tex files.
- In light mode Home shows the light logo. The dots in the top corners are gone.

### Windows
- Updating or uninstalling no longer stops with "Failed to kill Workspace". The installer waits for the app to close and ends it if it doesn't.

## 0.9.3 — 2026-10-20

### A new look
- A new icon: a W made of three blue sheets of paper on a dark tile. It sits beside the name on Home, too.
- Buttons, fields, dialogs and menus come from one set of shared pieces, so every screen matches. Primary buttons keep their blue when you point at them.
- Faint dots fade in from Home's top corners.

### Welcome tour
- The first-launch screen is now a short tour: the command palette, Word documents, presentations, PDFs, fonts and extensions, and split view, with default apps at the end on a Mac or Windows. **Skip tour** leaves it at any step, and **Settings › General › Show the tour again** brings it back.

### Version history
- Redesigned. Changes show as a page of the document: edited paragraphs are marked word by word, added ones in green and removed ones struck through in red. Unchanged stretches fold into a row you can click to open.
- Versions run down a timeline on the right. Right-click one, or use its **⋯** button, to name it, open a copy or delete it.

### Recent
- Home's Recent list filters by **Office**, **Notes** (Markdown and text), **PDF**, **Images** and **LaTeX** instead of one tab per file type. Folders are under **All**.

### Plugins
- Excalidraw, EPUB and Mermaid are gone, and their downloaded files are removed from your computer. Images is built in. LaTeX is still optional and off until you turn it on.
- The open-source licenses list matches: the removed plugins' packages are gone, and the image viewer's are listed with the app's.

### Polish
- The command palette matches words from their start, so "open" no longer finds New presentation. On Home, document commands show greyed out instead of missing.
- Esc leaves focus mode, and the hidden tab bar no longer leaves an empty band.
- Many labels, contrast fixes and sentence-case menus across Home, Settings, Word, PDF and PowerPoint.
- Recent paths read ~/Documents instead of Documents/~.

## 0.9.2 — 2026-10-20

### Find and replace
- **⌃H** (Ctrl+H on Windows and Linux) opens find with a **Replace with** field, in Word documents, presentations, Markdown and text files. **Replace** changes the highlighted match and moves on; **All** changes every one. The arrow at the left of the find box shows and hides the field.

### Lists as you type
- In Word documents, typing `- `, `* `, `1. ` or `1) ` at the start of a paragraph starts a bulleted or numbered list.

### One Open command
- **⌘O** opens files or a folder. On a Mac, pick a folder in the panel; on Windows and Linux, the dialog has an **Open Folder** button. Open Folder no longer has its own shortcut.

### Fixes
- Picking a spelling correction from the right-click menu in a Word document replaces the misspelled word. It used to do nothing, or type the correction at the cursor.
- Printing a Word document in dark mode gave grey text. Pages print black on white.
- On Windows, the title bar and menu bar take the app's colors, so the top of the window is one band instead of three shades.
- On Windows, the options in dropdowns such as the font list were light grey on white. They now use the app's colors.
- Commands in the command palette have their own icons (a sun and moon for appearance, a printer for Print…) instead of the Mac ⌘ symbol on every row.

## 0.9.1 — 2026-10-19

### Fedora
- Writan now comes as an `.rpm` for Fedora: `sudo dnf install ./Workspace-…x86_64.rpm`. It adds Writan's dnf repository, so new versions arrive with your system's other updates, and the **Update** button installs them with dnf.

### Fixes
- The setup screen fits in a short window. Continue no longer sits below the bottom edge until you go full screen.
- On Linux, the update panel names the right package (.deb or .rpm) and package manager.

## 0.9.0 — 2026-10-19

### Command palette
- **⌘K** opens one search box for everything: commands with their shortcuts, open tabs, recent files and the open folder's documents. Start with `>` for commands only.
- In Word, a link is now **⇧⌘K**, since ⌘K opens the palette. Settings › Keyboard Shortcuts can swap them back.

### Folders
- **File › Open Folder…** (⇧⌘O) shows a folder's documents in a sidebar beside the tabs, with search. ⇧⌘E hides it. Its **⋯** menu has Refresh, Collapse All Folders, Show All Files and Close Folder.
- Recent folders are in Home's Recent list, under All and their own Folders tab.

### Recent files expire
- Files and folders leave Home's Recent list 3 days after you last opened them. Pick 1, 7 or 30 days in **Settings › General**.

### Split view for Word and PDF
- **⇧⌘\** puts two Word documents or PDFs side by side, and asks which one goes beside the document in front: an open tab, a recent file, one from the open folder, any file, or a second copy of the same one.
- Each pane starts without its sidebar and inspector; the buttons at its corners show them. Markdown, Mermaid and LaTeX keep their own source-and-preview split, which ⇧⌘\ switches to.

### Version history
- Every save keeps a copy. **File › Version History…** (or right-click a tab) lists them by day, compares any version with now or with the save before it, and restores or opens a copy. Name a version to keep it for good. Settings › Autosave turns it off or clears it.

### PDF signing and forms
- **Sign** in a PDF's inspector places a saved signature or initials (drawn, typed or from a picture), the date or a check mark. PDF form fields can be filled, with a count and **Next empty**. **Save flattened copy…** bakes answers and marks into the page.

### Focus mode and typewriter scrolling
- **⌥⌘F** hides everything but the document and fades all but the paragraph you're writing. **⌥⌘T** keeps the line you're typing at the same height on screen.

### The workspace command
- `workspace .` opens the folder you're in, `workspace notes.md` a file. Install it on a Mac from **Settings › General › Command line**; the Linux package already has it.
- Opening a file or folder from the command line or Finder closes Settings (and the palette or version history) so you see it.

### Images
- **Line** and **arrow** tools. Hold Shift to keep one level, upright or at 45°.

## 0.8.0 — 2026-10-19

### Images
- A new optional plugin opens **PNG, JPEG and HEIC** pictures: zoom, fit to the window, and a few touch-ups (highlight, draw, box, crop, turn) with undo. ⌘C copies the picture, or the part you selected. An unchanged file is saved back exactly as it was.
- HEIC photos open for viewing (Writan can't write HEIC); **Edit a JPEG copy** opens one to mark up. Turn Images on in **Settings › Plugins**.

### Plugins and fonts in one place
- **Settings › Plugins** now has two lists, file types and fonts, each with a switch. The fonts moved there from General, and the list of built-in file types is gone.
- Six more optional fonts for Word and PowerPoint: **Ubuntu**, **JetBrains Mono**, **Atkinson Hyperlegible**, **Courier Prime**, **Instrument Serif** and **Comic Neue**. Each downloads when you turn it on and is checked before use.

### PDFs autosave
- PDFs now autosave like other files. Saving keeps your place, and autosave waits while you're typing into a form field or text box, or drawing. If you never changed the PDF setting, it's now on; turn it off in **Settings › Autosave**.
- Opening a LaTeX document's PDF in a tab again starts at the top; while you edit, it keeps updating where you are.

### Also
- A new app icon.
- The autosave status in the title bar is just a status; clicking it no longer opens Settings.
- Plugin engines (Excalidraw, Mermaid) are checked for new releases every week, and an update arrives as a pull request.

## 0.7.2 — 2026-10-18

> **LaTeX on Windows may still have bugs.** Installing TeX with winget, the MiKTeX fallback and turning the LaTeX plugin off on Windows are still being tested. If something goes wrong, please [open an issue](https://github.com/elijahroizman/writan/issues) with the message Writan shows.

### Linux updates through apt
- The .deb now adds Writan's APT repository, signed with its own key. New versions arrive with the rest of your system's updates (Software Updater, `sudo apt upgrade`).
- **Check for updates** on Linux can install a new version too: it asks for your password, installs with apt, and reopens Writan.
- If you installed 0.7.1 or earlier, install the 0.7.2 .deb once by hand. Updates come through apt after that.

### Dialogs
- Questions and alerts ("Save changes?", "Revert?", errors) are drawn by Writan instead of the system, so they look the same on macOS, Windows and Linux. Enter picks the highlighted button (Cancel when the other choice throws something away, as Revert and Delete do), Esc cancels.
- Deleting a custom theme asks first.

### LaTeX on Windows
- If winget can't install MiKTeX, Writan falls back to MiKTeX's own installer (checked against its published SHA-256). The error now includes winget's exit code and last message.
- Installing or removing TeX can no longer hang Writan: a step that takes longer than 45 minutes is stopped, and checking how TeX was installed gives up after a minute.
- Turning LaTeX off always works. Its question opens at once, and you can turn the plugin off while Writan is still working out whether it can also remove TeX.
- The install window can be hidden while TeX installs. **Settings › Plugins** shows the progress.
- Turning off a plugin with open tabs offers to close them, instead of telling you to close them yourself.

### Fonts
- **Michelangelus**, Microsoft's free typeface based on Michelangelo's handwriting, can be added in **Settings › General**. Microsoft's license doesn't let apps include it, so Writan downloads it from Microsoft and checks each file. It then shows in the Word and PowerPoint font menus.

### Also
- The PDF inspector's Document section is compact: each label and value share a row.
- The open-source licenses page credits T3 Code, which inspired the theme picker.
- Release notes are this changelog rather than a list of commits.
- Shorter README, with local development steps for Mac, Windows and Linux.

## 0.7.1 — 2026-10-18

- LaTeX installs TeX with Homebrew (Mac), winget (Windows) or apt (Debian, Ubuntu) when they're there, and offers to remove it when you turn the plugin off. A PDF opened in its own tab keeps updating. Errors wrap in the sidebar.
- Mermaid and LaTeX show errors in the sidebar only, and their sidebars match the rest of the app.
- Settings always opens on General. You can clear the file history and choose which file types get a tab under Recent.
- Appearance: a System option, color themes (two tones and an accent) and a theme creator.
- macOS window buttons moved down to line up with the tabs.
- A Debian/Ubuntu .deb is part of each release.

## 0.7.0 — 2026-10-18

- **Mermaid** (.mmd, .mermaid) and **LaTeX** (.tex) plugins. Both have a source and preview side by side, with the read / split / write switch Markdown uses (⌘E, ⌥⌘1–3).
- LaTeX compiles with the TeX on your computer (MacTeX, MiKTeX or TeX Live): in the document's folder, rerunning for references and running bibtex or biber. The PDF updates after you stop typing and shows in Writan's PDF viewer. Writan offers to install TeX when there's none.
- **EPUB** reader, a small plugin that comes with the app: one scrolling column, the table of contents in the sidebar, find, zoom, and it remembers your place.
- A plugin template and `npm run plugin:new` for writing plugins.

## 0.6.0 — 2026-10-18

- Plugins. Word, PowerPoint, PDF, Markdown and text are built in. Others are optional and off until you turn them on. **Excalidraw** is the first: it downloads from this repository's releases, and Writan checks its signature before installing it.
- A setup screen on first launch picks plugins and default apps. Settings has General (version, default apps, setup, open-source licenses) and Plugins pages.
- ⌘N asks which kind of file to make.
- Links in documents open in the browser again.
- Full-screen slideshows no longer draw lines twice or clip slide text.

## 0.5.1 — 2026-10-17

- Crop pictures in documents and presentations.
- ⌘-click (Ctrl-click on Windows) opens a Word link in the browser. Text typed after a link at the end of a paragraph no longer takes on the link's look.
- Resting the pointer on commented text shows the comment.
- Several picked slides can be deleted at once, in one undo step.
- New untitled documents take the lowest free number.

## 0.5.0 — 2026-10-17

- Word: headers and footers, and comments (add, reply, resolve, delete) saved in the file.
- PowerPoint: slide transitions with a preview, speaker notes, picking several slides or objects with shift- or ⌘-click, styling several objects at once, group and ungroup, layer order. Zoom keeps the slide in place.
- **Settings › Keyboard Shortcuts** lists every shortcut and lets you change it.

## 0.4.1 — 2026-10-16

- Autosave writes any file that already has a location back to it a few seconds after you stop editing.

## 0.4.0 — 2026-10-16

- The PDF editor runs on PDFium: faster zoom, page edits that apply instantly and can be undone, and less memory.
- The update panel shows the latest release's notes, and on a Mac the updated app reopens in the same Dock spot.
- The Mac app is signed and notarized.
- One Writan in the macOS Open With menu.

## 0.3.0 — 2026-10-16

- Word: links, page breaks, word count, table controls, and A4 for new documents.
- PowerPoint tables fit their frame and can be styled.
- Plain text (.txt) files.
- Embedding a YouTube video no longer crashes.

## 0.2.1 — 2026-10-15

- The downloaded Mac app no longer shows as "damaged".
- A tab's document loads the first time you look at it, so restoring a session is fast.

## 0.2.0 — 2026-10-15

- Windows: every shortcut works, labels say Ctrl/Alt/Shift, and the browser's reload keys are off.
- Automatic updates: checked after launch and every four hours, installed on quit or with **Restart to Update**.
- Pinch zoom follows your fingers.

## 0.1.0 — 2026-10-15

- First release: Word, PowerPoint, PDF and Markdown in tabs, each with its own sidebar and inspector. Native menus, Open With, session restore, printing, and light and dark themes.
