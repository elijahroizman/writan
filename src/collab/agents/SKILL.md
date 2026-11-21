---
name: writan
description: Edit Word documents, PowerPoint presentations and Excel workbooks together with people in Writan, live, through Writan's MCP server. Use when someone asks you to work in a document they have open or shared in Writan.
---

# Working in Writan with people

Writan is a desktop app for Word, PowerPoint and Excel files. When someone shares a document in Writan, you can join it through the `writan` MCP server and edit it while they do. They see your changes as you make them, with your name and logo on your cursor, and you see where they are.

## Before you start

Call `list_documents`. It lists the documents shared in Writan right now and who is in each. If it says nothing is shared, ask the person to open the document in Writan and press the people button at the top right to share it. You can only work on shared documents.

Pass a document's `document` id from that list to the other tools. The first tool you call on a document joins it, and people see you arrive.

## Reading

Call `read_document` before you edit, and again after anyone else has changed the document.

In a Word document it returns numbered paragraphs, and marks headings and paragraphs inside tables. The editing tools take those numbers. In a long document, read part of it with `from` and `to`.

In a presentation it returns each slide's number, layout, shapes and speaker notes. Every shape has an id in quotes, like `shape "4"`. Pass that id as `shape`.

In a workbook it returns the cells in use on one sheet as A1 references, with each formula and its result. Pass `sheet` and `range` (like `A1:F40`) to read another part.

## Editing

Word: `insert_paragraphs`, `replace_paragraphs`, `delete_paragraphs`, `set_style` and `replace_text`. Paragraph numbers change after an insert or delete above them, so read again before the next edit.

PowerPoint: `set_shape_text`, `add_slide` (it can fill the new slide's title and body), `delete_slide`, `set_notes` and `replace_text`.

Excel: `write_cells` with values as someone would type them, such as `"42"`, `"Total"` or `"=SUM(B2:B9)"`.

`undo` takes back your last change and leaves other people's alone.

## Working alongside people

Call `where_is_everyone` to see the paragraph, slide or cells each person is on. When someone says "here" or "this part", that tells you where they mean.

Don't edit a paragraph, shape or cell that someone else is editing unless they asked you to. If two changes to the same paragraph, shape or cell arrive at once, the first one counts and the second is undone. If yours didn't stick, read again and redo it.

Make one change at a time, then say what you changed. Keep to what you were asked to do.

Call `wait_for_changes` to wait for someone else's edit, for example after asking them to look something over. It returns who changed the document. Read it again afterwards.

Call `leave_document` when you are done, so people stop seeing you in the document.

## Saving

The person who shared the document keeps the file and saves it. You don't need to save.
