# Plugins

Word, PowerPoint, Excel, CSV, PDF, Markdown, text and images are built in. LaTeX is optional and off until you turn it on, in the onboarding on first launch or later in **Settings › General**.

| Plugin | Opens | Size | How it arrives |
| --- | --- | --- | --- |
| LaTeX | `.tex` documents | about 140 KB (TeX itself is separate) | Downloaded |

Writan checks every downloaded plugin's signature before installing it. Turning a downloaded plugin off removes its files; turning it back on downloads it again. Tabs using the plugin close first, and Writan asks about unsaved changes.

## Images

A plain viewer for PNG, JPEG, HEIC and WebP. Pinch or ⌘-scroll to zoom, double-click to switch between fitting the window and 100%, and drag to move around.

The toolbar has a few touch-ups: **highlight**, **draw**, **line**, **arrow**, **box**, **select** (to crop, or to copy part of the picture), and **turn** left or right. Hold Shift while highlighting or drawing for a straight line, or while drawing a line or arrow to keep it level, upright or at 45°. ⌘Z undoes, ⌘C copies the picture (or the selection). Changes save into the same file and format; a file you haven't changed is saved back exactly as it was. **⋯** saves a copy as PNG or JPEG, or prints.

Writan can read HEIC and WebP pictures but can't write them, so they open for viewing. **Edit a JPEG copy** opens a copy in a new tab to mark up and save. Outside Safari's engine on recent Macs, [libheif](https://github.com/strukturag/libheif), under the LGPL-3.0, decodes HEIC. It loads the first time you open one.

**Find** (⌘F, or Ctrl+F on Windows and Linux) finds words in the picture. Writan reads the picture's text on your computer the first time you search it, and highlights every match. **Show Recognized Text** in the **⋯** menu shows the text to copy, or saves it as a Word document or a searchable PDF.

## LaTeX

LaTeX works like Markdown: source and preview side by side, and ⌘E (Ctrl+E on Windows and Linux) switches between the preview, both, and the source.

Typing `\` lists LaTeX's commands, as VS Code's LaTeX extension does: `\frac{}{}`, `\section{}`, `\alpha` and the rest. Keep typing to narrow the list, then press Tab or Return to put one in; Tab then moves between its braces. `\begin{` lists the environments and adds the matching `\end`, `\ref{` lists the document's labels, and commands made with `\newcommand` are listed too.

LaTeX compiles with the TeX installed on your computer a few seconds after you stop typing (set how long in **Settings › General**). The PDF shows in Writan's own viewer, and errors and the outline are in the sidebar. A document compiles from the folder it's saved in, so `\input` and pictures beside it work.

### Getting TeX

If there's no TeX when you turn LaTeX on, Writan offers to install it:

- **Mac:** MacTeX or the smaller BasicTeX, through Homebrew if you have it, otherwise MacTeX's own installer (its signature is checked).
- **Windows:** MiKTeX, just for your user account, through winget. If winget isn't there or fails, MiKTeX's own installer is downloaded and checked against its published SHA-256.
- **Debian, Ubuntu:** TeX Live through apt, after asking for your password.

You can hide the install window while it runs; **Settings › General** shows the progress. When you turn LaTeX off, Writan asks whether to remove TeX too (only if it knows how it was installed). It's unticked unless you choose it.

Installing and removing TeX on Windows are still being tested. If either fails, the message includes what winget or the installer said. Put that in an [issue](https://github.com/elijahroizman/writan/issues).
