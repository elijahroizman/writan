"""Builds the stress samples into tests/stress/samples/ (or the folder given).

Needs python-docx, python-pptx and openpyxl:  python tests/stress/gen/ui-samples.py [out]
The PDFs are written by hand, so they need nothing.
"""
import io
import os
import sys
import random
import zipfile

from docx import Document
from docx.opc.constants import RELATIONSHIP_TYPE as RT
from docx.oxml.ns import qn
from docx.oxml import OxmlElement
from pptx import Presentation
from pptx.util import Inches
from openpyxl import Workbook
from openpyxl.comments import Comment

RNG = random.Random(1)
OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(__file__), '..', 'samples')
os.makedirs(OUT, exist_ok=True)
out = lambda name: os.path.join(OUT, name)

EVIL = [
    'javascript:alert(document.domain)',
    'JaVaScRiPt:alert(1)',
    ' javascript:alert(1)',
    'vbscript:msgbox(1)',
    'data:text/html,<script>alert(1)</script>',
    'file:///etc/passwd',
    'ms-msdt:/id PCWDiagnostic',
    'writan-evil://x',
    '../../../../etc/passwd',
    'https://example.com/ok',
    'mailto:someone@example.com',
]


def docx_links():
    d = Document()
    d.add_paragraph('Each line is a link. Ctrl-click (Cmd-click) each one.')
    for url in EVIL:
        p = d.add_paragraph()
        r_id = d.part.relate_to(url, RT.HYPERLINK, is_external=True)
        h = OxmlElement('w:hyperlink')
        h.set(qn('r:id'), r_id)
        r = OxmlElement('w:r')
        t = OxmlElement('w:t')
        t.text = url
        t.set(qn('xml:space'), 'preserve')
        r.append(t)
        h.append(r)
        p._p.append(h)
    # HTML in text and in a comment-like place: must show as text.
    d.add_paragraph('<img src=x onerror=alert(1)><script>alert(2)</script>')
    d.core_properties.title = '<img src=x onerror=alert(3)>'
    d.save(out('S1-docx-evil-links.docx'))


def pptx_links():
    p = Presentation()
    s = p.slides.add_slide(p.slide_layouts[6])
    for i, url in enumerate(EVIL):
        box = s.shapes.add_textbox(Inches(0.3 + (i % 3) * 3.2), Inches(0.3 + (i // 3) * 1.2), Inches(3), Inches(1))
        box.text_frame.text = url
        box.click_action.hyperlink.address = url
        run = box.text_frame.paragraphs[0].runs[0]
        run.hyperlink.address = url
    s.notes_slide.notes_text_frame.text = '<img src=x onerror=alert(4)>'
    p.save(out('S1-pptx-evil-links.pptx'))


def xlsx_html():
    wb = Workbook()
    ws = wb.active
    ws.title = '<img src=x onerror=alert(5)>'[:31]
    ws['A1'] = '<img src=x onerror=alert(6)>'
    ws['A2'] = '<script>alert(7)</script>'
    ws['A3'] = '=HYPERLINK("javascript:alert(8)","click")'
    ws['A4'].hyperlink = 'javascript:alert(9)'
    ws['A4'] = 'hyperlink cell'
    ws['B1'].comment = Comment('<img src=x onerror=alert(10)>', 'x<b>y</b>')
    ws2 = wb.create_sheet('"><svg onload=alert(11)>'[:31])
    ws2['A1'] = 'second'
    wb.save(out('S1-xlsx-html-cells.xlsx'))
    with open(out('S1-csv-html-cells.csv'), 'w', encoding='utf-8') as f:
        f.write('name,html\n"<img src=x onerror=alert(12)>","<script>alert(13)</script>"\n=HYPERLINK("javascript:alert(14)"),x\n')


def markdown():
    md = """# Markdown XSS

<script>alert('script')</script>
<img src=x onerror="alert('img')">
<svg><script>alert('svg')</script></svg>
<svg onload="alert('svgload')"></svg>
<iframe src="javascript:alert('iframe')"></iframe>
<iframe srcdoc="<script>parent.alert('srcdoc')</script>"></iframe>
<object data="data:text/html,<script>alert('obj')</script>"></object>
<embed src="data:text/html,<script>alert('embed')</script>">
<a href="javascript:alert('a')">a javascript link</a>
<a href="JAVASCRIPT:alert('A')">upper-case javascript link</a>
<a href="data:text/html,<script>alert(1)</script>">a data link</a>
<a href="file:///etc/passwd">a file link</a>
<a href="vbscript:msgbox(1)">vbscript</a>
<a href="#x" onclick="alert('onclick')">onclick</a>
<details open ontoggle="alert('toggle')"><summary>details</summary></details>
<math><mtext><table><mglyph><style><img src=x onerror=alert('mxss')></style></mglyph></table></mtext></math>
<meta http-equiv="refresh" content="0;url=javascript:alert('meta')">
<base href="javascript:alert('base')//">
<form action="javascript:alert('form')"><button>submit</button></form>
<input autofocus onfocus="alert('focus')">
<video><source onerror="alert('video')"></video>
<div style="background:url(javascript:alert('css'))">css</div>
<a href="writan-evil://x">custom scheme</a>

[md javascript](javascript:alert('md'))
[md spaced](<javascript:alert('md2')>)
[md entity](&#106;avascript:alert('ent'))
![img](x" onerror="alert('alt'))
<javascript:alert('autolink')>
<https://example.com/fine>
[relative](../../../../etc/passwd)
[relative doc](other.docx)
![local](/etc/passwd)

---
title: "<img src=x onerror=alert('fm')>"
---
"""
    front = '---\ntitle: "<img src=x onerror=alert(\'front\')>"\ntags: ["<b>x</b>", "<script>alert(1)</script>"]\n---\n'
    with open(out('S1-markdown-xss.md'), 'w') as f:
        f.write(front + md)


def svg():
    s = b'<svg xmlns="http://www.w3.org/2000/svg" onload="alert(\'svg-file\')" width="200" height="100"><script>alert("svg-script")</script><a href="javascript:alert(\'svg-a\')"><text x="10" y="50">click</text></a><foreignObject width="100" height="50"><img xmlns="http://www.w3.org/1999/xhtml" src="x" onerror="alert(\'fo\')"/></foreignObject></svg>'
    with open(out('S1-svg-script.svg'), 'wb') as f:
        f.write(s)
    return s


def docx_with_svg(svgbytes):
    # A docx whose picture is an SVG with script, plus a PNG fallback. Built by patching a picture in.
    png = (b'\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89'
           b'\x00\x00\x00\rIDATx\x9cc\xf8\xcf\xc0\xf0\x1f\x00\x05\x00\x01\xff\x89\x99=\x1d\x00\x00\x00\x00IEND\xaeB`\x82')
    d = Document()
    d.add_paragraph('Picture below is an SVG with a script in it.')
    d.add_picture(io.BytesIO(png), width=Inches(2))
    buf = io.BytesIO()
    d.save(buf)
    zin = zipfile.ZipFile(io.BytesIO(buf.getvalue()))
    zout_buf = io.BytesIO()
    zout = zipfile.ZipFile(zout_buf, 'w', zipfile.ZIP_DEFLATED)
    for item in zin.infolist():
        data = zin.read(item.filename)
        if item.filename.startswith('word/media/'):
            data = svgbytes
            item = zipfile.ZipInfo(item.filename.rsplit('.', 1)[0] + '.svg')
        if item.filename == 'word/_rels/document.xml.rels':
            data = data.replace(b'.png"', b'.svg"')
        if item.filename == '[Content_Types].xml':
            data = data.replace(b'</Types>', b'<Default Extension="svg" ContentType="image/svg+xml"/></Types>')
        zout.writestr(item, data)
    zout.close()
    with open(out('S1-docx-svg-image.docx'), 'wb') as f:
        f.write(zout_buf.getvalue())


def pdf(objects, name):
    """A PDF from a list of object bodies (1-based), with a correct xref."""
    body = b'%PDF-1.7\n%\xe2\xe3\xcf\xd3\n'
    offsets = []
    for i, o in enumerate(objects, 1):
        offsets.append(len(body))
        body += b'%d 0 obj\n' % i + o + b'\nendobj\n'
    x = len(body)
    body += b'xref\n0 %d\n0000000000 65535 f \n' % (len(objects) + 1)
    for off in offsets:
        body += b'%010d 00000 n \n' % off
    body += b'trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n' % (len(objects) + 1, x)
    with open(out(name), 'wb') as f:
        f.write(body)


def pdf_links():
    content = b'BT /F1 14 Tf 40 760 Td (Click each box. Page has an OpenAction script.) Tj ET\n'
    annots = []
    objs = [None, None, None, None, None]
    y = 700
    links = [b'javascript:alert(1)', b'file:///etc/passwd', b'data:text/html,<script>alert(1)</script>', b'vbscript:x', b'https://example.com/ok', b'writan-evil://x']
    for url in links:
        content += b'BT /F1 12 Tf 60 %d Td (%s) Tj ET\n' % (y + 6, url.replace(b'(', b'').replace(b')', b''))
        objs.append(b'<< /Type /Annot /Subtype /Link /Rect [50 %d 400 %d] /Border [0 0 1] /A << /S /URI /URI (%s) >> >>' % (y, y + 24, url))
        annots.append(len(objs))
        y -= 40
    # A JavaScript action link and a Launch action.
    content += b'BT /F1 12 Tf 60 %d Td (JavaScript action) Tj ET\n' % (y + 6)
    objs.append(b'<< /Type /Annot /Subtype /Link /Rect [50 %d 400 %d] /Border [0 0 1] /A << /S /JavaScript /JS (app.alert\\("js"\\)) >> >>' % (y, y + 24))
    annots.append(len(objs))
    y -= 40
    content += b'BT /F1 12 Tf 60 %d Td (Launch action) Tj ET\n' % (y + 6)
    objs.append(b'<< /Type /Annot /Subtype /Link /Rect [50 %d 400 %d] /Border [0 0 1] /A << /S /Launch /F (/bin/sh) >> >>' % (y, y + 24))
    annots.append(len(objs))
    objs[0] = b'<< /Type /Catalog /Pages 2 0 R /OpenAction << /S /JavaScript /JS (app.alert\\("open"\\)) >> /Names << /JavaScript << /Names [(x) << /S /JavaScript /JS (app.alert\\("names"\\)) >>] >> >> /Outlines 5 0 R >>'
    objs[1] = b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>'
    objs[2] = b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >> /Contents 4 0 R /Annots [%s] >>' % b' '.join(b'%d 0 R' % a for a in annots)
    objs[3] = b'<< /Length %d >>\nstream\n' % len(content) + content + b'endstream'
    objs.append(b'<< /Title (javascript bookmark) /Parent 5 0 R /A << /S /URI /URI (javascript:alert\\(2\\)) >> >>')
    bm1 = len(objs)
    objs.append(b'<< /Title (file bookmark) /Parent 5 0 R /A << /S /URI /URI (file:///etc/passwd) >> >>')
    bm2 = len(objs)
    objs[bm1 - 1] = objs[bm1 - 1].replace(b'>> >>', b'>> /Next %d 0 R >>' % bm2)
    objs[4] = b'<< /Type /Outlines /First %d 0 R /Last %d 0 R /Count 2 >>' % (bm1, bm2)
    pdf(objs, 'S1-pdf-evil-links.pdf')


def text_encodings():
    s = 'Grüße, naïve café — “quotes” € ünïcödé\n日本語のテキスト\nשלום עולם\n'
    with open(out('S1-text-utf16le-bom.txt'), 'wb') as f:
        f.write(b'\xff\xfe' + s.encode('utf-16-le'))
    with open(out('S1-text-utf16be-bom.txt'), 'wb') as f:
        f.write(b'\xfe\xff' + s.encode('utf-16-be'))
    with open(out('S1-text-windows1252.txt'), 'wb') as f:
        f.write('Grüße, naïve café — “quotes” € 1½\r\n'.encode('cp1252'))
    with open(out('S1-markdown-latin1.md'), 'wb') as f:
        f.write('# Café\n\nRésumé of naïve Zoë.\n'.encode('latin-1'))
    with open(out('S2-text-utf8-bom.txt'), 'wb') as f:
        f.write(b'\xef\xbb\xbf' + s.encode('utf-8'))
    with open(out('S2-csv-utf8-bom.csv'), 'wb') as f:
        f.write(b'\xef\xbb\xbfname,city\nZo\xc3\xab,M\xc3\xbcnchen\n')
    with open(out('S2-csv-utf16le-bom.csv'), 'wb') as f:
        f.write(b'\xff\xfe' + 'name\tcity\nZoë\tMünchen\n'.encode('utf-16-le'))
    with open(out('S2-csv-windows1252.csv'), 'wb') as f:
        f.write('name;city\r\nZoë;München\r\n'.encode('cp1252'))
    for name, data in [
        ('S2-markdown-crlf.md', b'# CRLF\r\n\r\nText\r\n'),
        ('S2-markdown-utf16le.md', b'\xff\xfe' + '# Caf\u00e9\r\nText\r\n'.encode('utf-16-le')),
        ('S2-text-cr.txt', b'old mac\rnext\rend\r'),
        ('S2-text-double-bom.txt', b'\xef\xbb\xbf\xef\xbb\xbfText\n'),
        ('S2-markdown-malformed-utf8-bom.md', b'\xef\xbb\xbfA\xffB\r\n'),
        ('S2-text-truncated-utf16.txt', b'\xff\xfeA\x00\xff'),
        ('S2-text-unpaired-utf16le.txt', b'\xff\xfeA\x00\x00\xd8B\x00'),
        ('S2-text-unpaired-utf16be.txt', b'\xfe\xff\x00A\xdc\x00\x00B'),
        ('S3-empty.tsv', b''),
    ]:
        with open(out(name), 'wb') as f:
            f.write(data)
    with open(out('S3-text-binary.txt'), 'wb') as f:
        f.write(bytes(range(256)) * 4)
    with open(out('S3-text-mixed-eol.txt'), 'wb') as f:
        f.write(b'unix\nwindows\r\nold mac\rend\n')
    with open(out('S3-empty.md'), 'wb') as f:
        pass
    with open(out('S3-empty.txt'), 'wb') as f:
        pass


def corrupt():
    pdfbytes = open(out('S1-pdf-evil-links.pdf'), 'rb').read()
    docbytes = open(out('S1-docx-evil-links.docx'), 'rb').read()
    for name, data in [
        ('S2-pdf-renamed.docx', pdfbytes),
        ('S2-docx-renamed.pdf', docbytes),
        ('S2-docx-renamed.pptx', docbytes),
        ('S2-docx-renamed.xlsx', docbytes),
        ('S2-random.docx', RNG.randbytes(4096)),
        ('S2-random.pptx', RNG.randbytes(4096)),
        ('S2-random.xlsx', RNG.randbytes(4096)),
        ('S2-random.pdf', RNG.randbytes(4096)),
        ('S2-empty.docx', b''),
        ('S2-empty.pptx', b''),
        ('S2-empty.xlsx', b''),
        ('S2-empty.pdf', b''),
        ('S2-empty.csv', b''),
        ('S2-truncated.docx', docbytes[: len(docbytes) // 2]),
        ('S2-truncated.pdf', pdfbytes[: len(pdfbytes) // 2]),
        ('S2-random.png', RNG.randbytes(2048)),
        ('S2-empty.png', b''),
    ]:
        with open(out(name), 'wb') as f:
            f.write(data)
    # An Office file encrypted with a password is an OLE compound file, not a zip.
    ole = b'\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1' + b'\x00' * 504 + b'EncryptionInfo'.ljust(512, b'\x00')
    for ext in ('docx', 'pptx', 'xlsx'):
        with open(out('S2-encrypted.' + ext), 'wb') as f:
            f.write(ole)
    # A PDF with no pages at all.
    pdf([b'<< /Type /Catalog /Pages 2 0 R >>', b'<< /Type /Pages /Kids [] /Count 0 >>'], 'S2-pdf-no-pages.pdf')


def encrypted():
    # Really password-protected files (password "user"), when the tools are there.
    try:
        import msoffcrypto
        from msoffcrypto.format.ooxml import OOXMLFile
        for src, ext in (('S1-docx-evil-links.docx', 'docx'), ('S1-pptx-evil-links.pptx', 'pptx'), ('S1-xlsx-html-cells.xlsx', 'xlsx')):
            with open(out(src), 'rb') as f, open(out('S2-password.' + ext), 'wb') as o:
                OOXMLFile(f).encrypt('user', o)
    except Exception as e:
        print('skipped encrypted Office files:', e)
    import shutil, subprocess
    if shutil.which('gs'):
        subprocess.run(['gs', '-q', '-dNOPAUSE', '-dBATCH', '-sDEVICE=pdfwrite', '-sOwnerPassword=owner', '-sUserPassword=user',
                        '-dEncryptionR=3', '-dKeyLength=128', '-o', out('S2-password.pdf'), out('S1-pdf-evil-links.pdf')], check=True)
        subprocess.run(['gs', '-q', '-dNOPAUSE', '-dBATCH', '-sDEVICE=pdfwrite', '-sOwnerPassword=owner',
                        '-dEncryptionR=3', '-dKeyLength=128', '-dPermissions=-3904', '-o', out('S3-pdf-owner-password.pdf'), out('S1-pdf-evil-links.pdf')], check=True)


docx_links()
pptx_links()
xlsx_html()
markdown()
docx_with_svg(svg())
pdf_links()
text_encodings()
corrupt()
encrypted()



def css_fonts():
    # Font names that try to break out of the page's style sheet.
    d = Document()
    names = ['Evil\\', '}*{display:none!important}', 'X"}body{background:red}', 'Y;color:red', 'Z\n}.app{display:none}', 'W</style><script>alert(1)</script>']
    for n in names:
        r = d.add_paragraph().add_run('font named ' + n.replace('\n', ' '))
        r.font.name = n.replace('\n', '')
    d.add_paragraph('If you can read this and the app around it, the fonts were harmless.')
    d.save(out('S2-docx-css-font-names.docx'))
    p = Presentation()
    s = p.slides.add_slide(p.slide_layouts[6])
    for i, n in enumerate(names):
        box = s.shapes.add_textbox(Inches(0.5), Inches(0.5 + i * 0.8), Inches(8), Inches(0.7))
        box.text_frame.text = 'font named ' + n
        box.text_frame.paragraphs[0].runs[0].font.name = n.replace('\n', '')
    p.save(out('S2-pptx-css-font-names.pptx'))
    wb = Workbook()
    from openpyxl.styles import Font
    for i, n in enumerate(names, 1):
        wb.active.cell(i, 1, 'font named ' + n).font = Font(name=n.replace('\n', '')[:31])
    wb.save(out('S2-xlsx-css-font-names.xlsx'))


css_fonts()



# Imported hidden worksheets must never be the initial visible canvas.
wb = Workbook()
wb.active.title = 'Hidden first'
wb.active['A1'] = 'Hidden contents'
wb.active.sheet_state = 'hidden'
very = wb.create_sheet('Very hidden')
very['A1'] = 'Very hidden contents'
very.sheet_state = 'veryHidden'
visible = wb.create_sheet('Visible')
visible['A1'] = 'Visible contents'
wb.active = 2
wb.save(out('S2-xlsx-hidden-first.xlsx'))


# Conditional formatting, data validation and a picture must all show (CORPUS-010).
def excel_visuals():
    from openpyxl.drawing.image import Image as XlImage
    from openpyxl.formatting.rule import CellIsRule, ColorScaleRule, DataBarRule, FormulaRule, IconSetRule
    from openpyxl.styles import Font, PatternFill
    from openpyxl.worksheet.datavalidation import DataValidation
    from PIL import Image as PilImage
    wb = Workbook()
    ws = wb.active
    ws.title = 'Visuals'
    ws['A1'], ws['B1'], ws['C1'], ws['D1'], ws['E1'] = 'Status', 'Score', 'Scale', 'Bar', 'Trend'
    for i, (status, score) in enumerate([('Done', 92), ('Late', 41), ('Done', 77), ('Open', 15)], 2):
        ws.cell(i, 1, status)
        ws.cell(i, 2, score)
        ws.cell(i, 3, score)
        ws.cell(i, 4, score)
        ws.cell(i, 5, score)
    green = PatternFill(start_color='FFC6EFCE', end_color='FFC6EFCE', fill_type='solid')
    red = PatternFill(start_color='FFFFC7CE', end_color='FFFFC7CE', fill_type='solid')
    ws.conditional_formatting.add('A2:A5', FormulaRule(formula=['$A2="Done"'], fill=green))
    ws.conditional_formatting.add('B2:B5', CellIsRule(operator='lessThan', formula=['50'], fill=red, font=Font(bold=True, color='FF9C0006')))
    ws.conditional_formatting.add('C2:C5', ColorScaleRule(start_type='min', start_color='FFF8696B', end_type='max', end_color='FF63BE7B'))
    ws.conditional_formatting.add('D2:D5', DataBarRule(start_type='min', end_type='max', color='FF638EC6'))
    ws.conditional_formatting.add('E2:E5', IconSetRule('3Arrows', 'percent', [0, 33, 67]))
    dv = DataValidation(type='list', formula1='"Open,Late,Done"', allow_blank=True, showErrorMessage=True,
                        errorTitle='Not a status', error='Pick Open, Late or Done', promptTitle='Status', prompt='Pick a status')
    dv.showInputMessage = True
    ws.add_data_validation(dv)
    dv.add('A2:A10')
    img = io.BytesIO()
    PilImage.new('RGB', (96, 64), (220, 60, 40)).save(img, 'PNG')
    picture = XlImage(img)
    picture.anchor = 'G2'
    ws.add_image(picture)
    wb.save(out('S2-xlsx-cf-validation-picture.xlsx'))


excel_visuals()

print('wrote', len(os.listdir(OUT)), 'files to', os.path.abspath(OUT))
