#!/usr/bin/env python3
"""Writes the deterministic performance corpus (standard library only)."""
import io
import os
import zipfile

out = os.path.join(os.path.dirname(__file__), "generated")
os.makedirs(out, exist_ok=True)

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
X = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
REL = "http://schemas.openxmlformats.org/package/2006/relationships"
OD = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument"
DECL = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'


def zipped(parts):
    b = io.BytesIO()
    with zipfile.ZipFile(b, "w", zipfile.ZIP_DEFLATED) as z:
        for name, data in parts:
            info = zipfile.ZipInfo(name, (2026, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            z.writestr(info, data.encode())
    return b.getvalue()


def types(overrides):
    d = '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>'
    o = "".join(f'<Override PartName="{p}" ContentType="{t}"/>' for p, t in overrides)
    return f'{DECL}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">{d}{o}</Types>'


def rels(items):
    r = "".join(f'<Relationship Id="{i}" Type="{t}" Target="{target}"/>' for i, t, target in items)
    return f'{DECL}<Relationships xmlns="{REL}">{r}</Relationships>'


def docx(body):
    main = "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"
    return zipped([
        ("[Content_Types].xml", types([("/word/document.xml", main)])),
        ("_rels/.rels", rels([("rId1", OD, "word/document.xml")])),
        ("word/document.xml", f'{DECL}<w:document xmlns:w="{W}" xmlns:r="{R}"><w:body>{body}<w:sectPr/></w:body></w:document>'),
    ])


def xlsx(sheet_data):
    main = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"
    sheet = "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"
    return zipped([
        ("[Content_Types].xml", types([("/xl/workbook.xml", main), ("/xl/worksheets/sheet1.xml", sheet)])),
        ("_rels/.rels", rels([("rId1", OD, "xl/workbook.xml")])),
        ("xl/workbook.xml", f'{DECL}<workbook xmlns="{X}" xmlns:r="{R}"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>'),
        ("xl/_rels/workbook.xml.rels", rels([("rId1", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet", "worksheets/sheet1.xml")])),
        ("xl/worksheets/sheet1.xml", f'{DECL}<worksheet xmlns="{X}"><sheetData>{sheet_data}</sheetData></worksheet>'),
    ])


def para(text, page_break=False):
    br = '<w:r><w:br w:type="page"/></w:r>' if page_break else ''
    return f'<w:p><w:r><w:t xml:space="preserve">{text}</w:t></w:r>{br}</w:p>'


# A long, explicit-page document. Most vocabulary repeats as in real prose, while each paragraph
# carries unique tokens so caches cannot collapse the whole measurement problem away.
vocabulary = "the quick brown fox jumps over a lazy dog while editors measure every line carefully".split()
pages = []
for page in range(300):
    for row in range(8):
        words = [vocabulary[(page * 17 + row * 5 + i) % len(vocabulary)] for i in range(18)]
        words[3] = f"token{page:03d}x{row}"
        words[11] = f"measure{page:03d}y{row}"
        pages.append(para(" ".join(words), page_break=row == 7 and page < 299))
with open(os.path.join(out, "large-document.docx"), "wb") as f:
    f.write(docx("".join(pages)))

# A large numeric/text grid for the spreadsheet viewport and input path.
rows = []
for r in range(1, 20_001):
    cells = []
    for c in range(1, 21):
        col = chr(64 + c)
        if c == 1:
            cells.append(f'<c r="{col}{r}" t="inlineStr"><is><t>row {r}</t></is></c>')
        else:
            cells.append(f'<c r="{col}{r}"><v>{r * 97 + c}</v></c>')
    rows.append(f'<row r="{r}">{"".join(cells)}</row>')
with open(os.path.join(out, "large-sheet.xlsx"), "wb") as f:
    f.write(xlsx("".join(rows)))

print(out)
