---
name: writan
description: Edit Word documents, PowerPoint presentations and Excel workbooks together with people in Writan, live, through Writan's MCP server. Use when someone asks you to work in a document they have open or shared in Writan.
---

# Working in Writan with people

Writan is a desktop app for Word, PowerPoint and Excel files. When someone shares a document in Writan, you can join it through the `writan` MCP server and edit it while they do. They see your changes as you make them, with your name and logo on your cursor, and you see where they are. You can do most of what they can: write and format text, move, resize, crop and restack pictures and shapes, add slides, tables and pictures, and format cells.

## Before you start

Call `list_documents`. It lists the documents shared in Writan right now and who is in each. If it says nothing is shared, ask the person to open the document in Writan and press the people button at the top right to share it. You can only work on shared documents.

Pass a document's `document` id from that list to the other tools. The first tool you call on a document joins it, and people see you arrive.

## Reading and seeing

Call `read_document` before you edit, and again after anyone else has changed the document.

In a Word document it returns numbered paragraphs, and marks headings and the table cell each table paragraph is in. The editing tools take those numbers. In a long document, read part of it with `from` and `to`.

In a presentation it returns the slide size in pixels (96 to an inch), then each slide's layout, shapes and speaker notes. Every shape has an id in quotes, like `shape "4"`, which you pass as `shape`, and says what it is, where it is (`at x 80, y 120, 400 × 300`), how it looks (font, size, color, fill, outline, crop) and its text. Shapes are listed back to front: a later one covers an earlier one.

In a workbook it returns the cells in use on one sheet as A1 references, with each formula and its result, then the sheet's charts by number. Pass `sheet` and `range` (like `A1:F40`) to read another part.

`view_document` shows a slide, a Word page or a sheet as a picture, the way people see it. On a slide each shape is outlined in pink with its id, so you can tell which photo is which, and see what overlaps or looks crowded; pass `shape` to see one closer up. On a sheet each chart is outlined with its number; pass `range` for another part or `chart` for one chart. On a sheet you can see what the text can't tell you, like #### where a column is too narrow. Look at a slide before you change how it looks, and again afterwards to check.

## Editing

Every edit answers with what it changed, as it is once everyone has it: the paragraphs, the shape with its new place and look, or the cells. You don't need to read again just to confirm an edit.

Word: `insert_paragraphs`, `replace_paragraphs`, `delete_paragraphs`, `set_style` (like "Heading 1"), `format_text` (font, size, bold, italic, underline, color, highlight, alignment, lists and line spacing, for whole paragraphs or just the words in `find`), `insert_table` and `change_table`, `add_picture` and `format_picture` (resize and crop), and `replace_text`. Paragraph numbers change after an insert or delete above them, so read again before the next edit.

PowerPoint:
- Text: `set_shape_text`, `format_text` (a whole shape, paragraphs `from`–`to` of it, or the words in `find`), `replace_text`, `set_table_text`.
- Pictures and shapes: `move_shape` moves, resizes, turns, flips and restacks (`layer`: front, forward, backward, back). Giving only width or only height keeps the proportions. `crop_picture` with `fit: "fill"` fills a frame without stretching, `fit: "fit"` shows the whole picture, or cut each side by a share. `format_shape` sets fill, outline and link. `add_shape` adds a text box, shape or table; `add_picture` adds a picture from a file path, an https address or base64 data; `copy_shapes`, `group_shapes` and `delete_shapes`.
- Slides: `add_slide` (it can fill the new slide's title and body), `duplicate_slide`, `move_slide`, `set_slide` (layout, hidden), `delete_slide`, `set_notes`.

Positions and sizes are in slide pixels from the top left. `move_shape` and `crop_picture` answer with what the shape overlaps and whether it runs off the slide.

Excel: `write_cells` with values as someone would type them, such as `"42"`, `"Total"` or `"=SUM(B2:B9)"`; `format_cells` (bold, colors, fill, alignment, wrap, number format like `"$#,##0.00"`, borders); `clear_cells`, `size_cells`, `insert_cells`, `delete_cells`, `merge_cells`, `add_chart` and `edit_chart` (title, axis titles, kind, stacking, legend, the cells it plots and series colors, by chart number), and `edit_sheet` (add, rename, move or delete a sheet).

`undo` takes back your last change and leaves other people's alone.

## Making a slide look good

1. `view_document` the slide, and `read_document` it for ids and positions.
2. Decide the layout in slide pixels: margins, a grid, and room between pictures and text.
3. Move and size each picture with `move_shape`, then `crop_picture` with `fit: "fill"` so photos fill their frames without stretching.
4. Set text with `format_text`, and fills and outlines with `format_shape`.
5. `view_document` again, and fix what overlaps or looks off.

## Working alongside people

Call `where_is_everyone` to see the paragraph, slide or cells each person is on. When someone says "here" or "this part", that tells you where they mean.

Don't edit a paragraph, shape or cell that someone else is editing unless they asked you to. If two changes to the same paragraph, slide or cell arrive at once, the first one counts and the second is undone. If yours didn't stick, the edit's answer shows it; read again and redo it.

Make one change at a time, then say what you changed. Keep to what you were asked to do.

Call `wait_for_changes` to wait for someone else's edit, for example after asking them to look something over, or while they type and you tidy up after them. It says who changed what: the paragraphs in Word, each slide and shape in PowerPoint, where they were in Excel.

Call `leave_document` when you are done, so people stop seeing you in the document.

## Saving

The person who shared the document keeps the file and saves it. You don't need to save.
