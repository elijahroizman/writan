#!/usr/bin/env python3
"""Create original, deterministic DOCX regression samples using only the standard library."""
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo

ROOT = Path(__file__).resolve().parent
W = 'http://schemas.openxmlformats.org/wordprocessingml/2006/main'
R = 'http://schemas.openxmlformats.org/package/2006/relationships'


def package(name, body, numbering=None):
    parts = {
        '[Content_Types].xml': '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
        '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
        '<Default Extension="xml" ContentType="application/xml"/>'
        '<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>'
        + ('<Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/>' if numbering else '')
        + '</Types>',
        '_rels/.rels': f'<Relationships xmlns="{R}"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>',
        'word/document.xml': f'<w:document xmlns:w="{W}" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:body>{body}<w:sectPr/></w:body></w:document>',
    }
    if numbering:
        parts['word/numbering.xml'] = f'<w:numbering xmlns:w="{W}">{numbering}</w:numbering>'
        parts['word/_rels/document.xml.rels'] = f'<Relationships xmlns="{R}"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/></Relationships>'
    with ZipFile(ROOT / name, 'w') as z:
        for path, value in parts.items():
            info = ZipInfo(path, (2026, 10, 24, 0, 0, 0))
            info.compress_type = ZIP_DEFLATED
            z.writestr(info, value.encode())


def cell(text, span=1):
    grid = f'<w:gridSpan w:val="{span}"/>' if span > 1 else ''
    return f'<w:tc><w:tcPr><w:tcW w:w="{span * 2400}" w:type="dxa"/>{grid}</w:tcPr><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>'


def control(content, checkbox=False):
    props = '<w14:checkbox><w14:checked w14:val="0"/></w14:checkbox>' if checkbox else '<w:tag w:val="raw row"/>'
    return f'<w:sdt><w:sdtPr>{props}</w:sdtPr><w:sdtContent>{content}</w:sdtContent></w:sdt>'


rows = control('<w:tr>' + cell('Leading raw wrapped row', 3) + '</w:tr>')
rows += '<w:tr>' + cell('Label before checkbox') + control(cell('☐', 2), True) + '</w:tr>'
rows += control('<w:tr>' + cell('Raw wrapped row', 3) + '</w:tr>')
rows += '<w:tr>' + control(cell('☐'), True) + cell('Middle label') + control(cell('☐'), True) + '</w:tr>'
rows += '<!--before merged row--><w:tr>' + cell('Merged label', 2) + control(cell('☐'), True) + '</w:tr>'
rows += '<w:tr>' + cell('First label') + control(cell('☐'), True) + cell('Last label') + '</w:tr>'
rows += control('<w:tr>' + cell('Trailing raw row', 3) + '</w:tr>')
package('table-content-controls.docx', '<w:tbl><w:tblPr/><w:tblGrid>' + '<w:gridCol w:w="2400"/>' * 3 + '</w:tblGrid>' + rows + '</w:tbl>')

body, definitions = '', ''
for i, (fmt, count) in enumerate([('ideographTraditional', 12), ('ideographZodiac', 14)], 1):
    definitions += f'<w:abstractNum w:abstractNumId="{i}"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="{fmt}"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum><w:num w:numId="{i}"><w:abstractNumId w:val="{i}"/></w:num>'
    for n in range(1, count + 1):
        body += f'<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="{i}"/></w:numPr></w:pPr><w:r><w:t>{fmt} item {n}</w:t></w:r></w:p>'
package('cjk-list-markers.docx', body, definitions)

# A right-to-left table: w:bidiVisual puts the first grid column at the right, and its indent
# measures from the right edge. Unequal columns catch a mirroring that only swaps labels.
rtl_rows = '<w:tr>' + cell('first') + cell('second') + '</w:tr>'
rtl_rows += '<w:tr>' + cell('alpha') + cell('beta') + '</w:tr>'
rtl_tbl = (
    '<w:tbl><w:tblPr><w:bidiVisual/><w:tblW w:w="0" w:type="auto"/><w:tblInd w:w="720" w:type="dxa"/>'
    '<w:tblBorders>'
    + ''.join(f'<w:{s} w:val="single" w:sz="4" w:space="0" w:color="auto"/>' for s in
              ['top', 'left', 'bottom', 'right', 'insideH', 'insideV'])
    + '</w:tblBorders></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="4000"/></w:tblGrid>'
    + rtl_rows + '</w:tbl>')
# An ordinary table after it, whose left-to-right order must not change.
ltr_tbl = (
    '<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr>'
    '<w:tblGrid><w:gridCol w:w="2400"/><w:gridCol w:w="2400"/></w:tblGrid>'
    '<w:tr>' + cell('west') + cell('east') + '</w:tr></w:tbl>')
package('rtl-table.docx', rtl_tbl + ltr_tbl)

ins = lambda i, who, text: f'<w:ins w:id="{i}" w:author="{who}" w:date="2026-11-01T09:00:00Z"><w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:ins>'
dele = lambda i, who, text: f'<w:del w:id="{i}" w:author="{who}" w:date="2026-11-01T09:00:00Z"><w:r><w:delText xml:space="preserve">{text}</w:delText></w:r></w:del>'
package('tracked-changes.docx',
        '<w:p><w:r><w:t xml:space="preserve">The fox is </w:t></w:r>' + ins(1, 'Ann', 'quick') + dele(2, 'Bob', 'slow') + '<w:r><w:t>.</w:t></w:r></w:p>'
        '<w:p><w:r><w:rPr><w:b/><w:rPrChange w:id="3" w:author="Ann" w:date="2026-11-01T09:00:00Z"><w:rPr/></w:rPrChange></w:rPr><w:t>Made bold.</w:t></w:r></w:p>')
