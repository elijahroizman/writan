// Run against a VITE_TEST_HOOKS build with WRITAN_URL set to its preview URL.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { inflateSync } from 'node:zlib'
import JSZip from 'jszip'
import { BASE, launch } from './harness.mjs'

const office = 'http://schemas.openxmlformats.org'
const relNs = office + '/officeDocument/2006/relationships'
const drawingNs = office + '/drawingml/2006'
const wordNs = office + '/wordprocessingml/2006/main'
const sheetNs = office + '/spreadsheetml/2006/main'
const presentationNs = office + '/presentationml/2006/main'
const fixedDate = new Date(Date.UTC(2000, 0, 1))
const letter = [612, 792]
const wideSlide = [960, 540]
const landscape = [792, 612]
const jpeg = readFileSync(new URL('../../../crates/office-pdf/tests/fixtures/pixels.jpg', import.meta.url))
// A 32x24 RGB PNG with a red-orange left half and a blue right half.
const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAACAAAAAYCAIAAAAUMWhjAAAAKUlEQVR4nGN44aNAErKpuEESYhi1YNSCUQtGLRi1YNSCUQtGLRgaFgAAUNNQPf/p/PoAAAAASUVORK5CYII=', 'base64')
const escapeXml = (value) => value.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('"', '&quot;')

function relationships(entries) {
  return `<Relationships xmlns="${office}/package/2006/relationships">
    ${entries.map(([id, kind, target, external]) =>
      `<Relationship Id="${id}" Type="${relNs}/${kind}" Target="${escapeXml(target)}"${external ? ' TargetMode="External"' : ''}/>`).join('')}
  </Relationships>`
}

async function packageFile(main, parts) {
  const zip = new JSZip()
  const add = (name, content) => zip.file(name, content, { date: fixedDate, createFolders: false })
  add('_rels/.rels', relationships([['rId1', 'officeDocument', main]]))
  add('[Content_Types].xml', `<Types xmlns="${office}/package/2006/content-types">
    <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
    <Default Extension="png" ContentType="image/png"/><Default Extension="jpg" ContentType="image/jpeg"/>
    <Default Extension="svg" ContentType="image/svg+xml"/>
    ${parts.filter(([, , type]) => type).map(([name, , type]) =>
      `<Override PartName="/${name}" ContentType="application/vnd.openxmlformats-officedocument.${type}+xml"/>`).join('')}
  </Types>`)
  for (const [name, content] of parts) add(name, content)
  return await zip.generateAsync({ type: 'base64' })
}

const run = (text) => `<w:r><w:t xml:space="preserve">${escapeXml(text)}</w:t></w:r>`
const paragraph = (text) => `<w:p>${run(text)}</w:p>`

function word(body, refs = '', parts = []) {
  return packageFile('word/document.xml', [
    ['word/document.xml', `<w:document xmlns:w="${wordNs}" xmlns:r="${relNs}"
      xmlns:m="${office}/officeDocument/2006/math" xmlns:wp="${drawingNs}/wordprocessingDrawing"
      xmlns:a="${drawingNs}/main" xmlns:pic="${drawingNs}/picture">
      <w:body>${body}<w:sectPr>${refs}<w:pgSz w:w="12240" w:h="15840"/>
        <w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720"/>
      </w:sectPr></w:body></w:document>`, 'wordprocessingml.document.main'],
    ...parts,
  ])
}

function table() {
  const borders = ['top', 'left', 'bottom', 'right', 'insideH', 'insideV'].map((side) =>
    `<w:${side} w:val="single" w:sz="12" w:color="CC0000"/>`).join('')
  const cell = (label, shade) => `<w:tc><w:tcPr><w:tcW w:w="2700" w:type="dxa"/>
    ${shade ? '<w:shd w:val="clear" w:fill="00CC00"/>' : ''}</w:tcPr>${paragraph(label)}</w:tc>`
  return word(`<w:tbl><w:tblPr><w:tblW w:w="5400" w:type="dxa"/><w:tblBorders>${borders}</w:tblBorders></w:tblPr>
    <w:tblGrid><w:gridCol w:w="2700"/><w:gridCol w:w="2700"/></w:tblGrid>
    <w:tr>${cell('Shaded cell', true)}${cell('Bordered cell', false)}</w:tr></w:tbl>`)
}

function wordFurniture() {
  return word(
    `<w:p>${run('Note anchor')}<w:r><w:footnoteReference w:id="1"/></w:r></w:p>`,
    '<w:headerReference w:type="default" r:id="header"/><w:footerReference w:type="default" r:id="footer"/>',
    [
      ['word/_rels/document.xml.rels', relationships([
        ['header', 'header', 'header1.xml'], ['footer', 'footer', 'footer1.xml'], ['notes', 'footnotes', 'footnotes.xml'],
      ])],
      ['word/header1.xml', `<w:hdr xmlns:w="${wordNs}">${paragraph('Header sentinel')}</w:hdr>`, 'wordprocessingml.header'],
      ['word/footer1.xml', `<w:ftr xmlns:w="${wordNs}">${paragraph('Footer sentinel')}</w:ftr>`, 'wordprocessingml.footer'],
      ['word/footnotes.xml', `<w:footnotes xmlns:w="${wordNs}"><w:footnote w:id="1">
        <w:p><w:r><w:footnoteRef/></w:r>${run('Footnote sentinel')}</w:p>
      </w:footnote></w:footnotes>`, 'wordprocessingml.footnotes'],
    ],
  )
}

function wordPictures() {
  const picture = (id, rel, crop = '') => `<w:p><w:r><w:drawing><wp:inline>
    <wp:extent cx="1219200" cy="914400"/><wp:docPr id="${id}" name="Picture ${id}"/>
    <a:graphic><a:graphicData uri="${drawingNs}/picture"><pic:pic>
      <pic:nvPicPr><pic:cNvPr id="${id}" name="Picture ${id}"/><pic:cNvPicPr/></pic:nvPicPr>
      <pic:blipFill><a:blip r:embed="${rel}"/>${crop}<a:stretch><a:fillRect/></a:stretch></pic:blipFill>
      <pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1219200" cy="914400"/></a:xfrm>
      <a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr>
    </pic:pic></a:graphicData></a:graphic>
  </wp:inline></w:drawing></w:r></w:p>`
  const link = (id, text) => `<w:p><w:hyperlink r:id="${id}">${run(text)}</w:hyperlink></w:p>`
  return word(
    picture(1, 'png') + picture(2, 'jpg', '<a:srcRect l="25000" t="12500"/>') +
    link('safe', 'Writan homepage') + link('script', 'Script link label') + link('local', 'Local link label'),
    '',
    [
      ['word/_rels/document.xml.rels', relationships([
        ['png', 'image', 'media/chart.png'], ['jpg', 'image', 'media/pixels.jpg'],
        ['safe', 'hyperlink', 'https://writan.app', true], ['script', 'hyperlink', 'javascript:alert(1)', true],
        ['local', 'hyperlink', 'file:///etc/passwd', true],
      ])],
      ['word/media/chart.png', png],
      ['word/media/pixels.jpg', jpeg],
    ],
  )
}

function mathWord() {
  const mathRun = (text) => `<m:r><m:t>${text}</m:t></m:r>`
  const tag = 'writan:latex:' + JSON.stringify({
    v: 1, latex: String.raw`\sqrt{x^2+1}`, display: false, fontSizePt: 16, color: '#CC00CC', w: 90, h: 28,
  })
  return word(`<w:p>${run('Native equation')}
    <m:oMath><m:f><m:num>${mathRun('317')}</m:num><m:den>${mathRun('23')}</m:den></m:f></m:oMath></w:p>
    <w:p>${run('LaTeX equation')}<w:sdt><w:sdtPr><w:tag w:val="${escapeXml(tag)}"/></w:sdtPr>
    <w:sdtContent><m:oMath>${mathRun('0')}</m:oMath></w:sdtContent></w:sdt></w:p>`)
}

function chart(title) {
  return `<c:chartSpace xmlns:c="${drawingNs}/chart" xmlns:a="${drawingNs}/main"><c:chart>
    <c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>${title}</a:t></a:r></a:p></c:rich></c:tx></c:title>
    <c:plotArea><c:layout/><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>
      <c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:v>Volume</c:v></c:tx>
        <c:spPr><a:solidFill><a:srgbClr val="0000CC"/></a:solidFill></c:spPr>
        <c:cat><c:strLit><c:ptCount val="2"/><c:pt idx="0"><c:v>Harbor</c:v></c:pt><c:pt idx="1"><c:v>Summit</c:v></c:pt></c:strLit></c:cat>
        <c:val><c:numLit><c:formatCode>General</c:formatCode><c:ptCount val="2"/>
          <c:pt idx="0"><c:v>5</c:v></c:pt><c:pt idx="1"><c:v>12</c:v></c:pt>
        </c:numLit></c:val>
      </c:ser><c:axId val="10"/><c:axId val="20"/>
    </c:barChart>
    <c:catAx><c:axId val="10"/><c:scaling/><c:axPos val="b"/><c:crossAx val="20"/></c:catAx>
    <c:valAx><c:axId val="20"/><c:scaling/><c:axPos val="l"/><c:crossAx val="10"/></c:valAx>
    </c:plotArea></c:chart></c:chartSpace>`
}

function frame(prefix, id, x, y, w, h) {
  return `<${prefix}:graphicFrame><${prefix}:nvGraphicFramePr>
    <${prefix}:cNvPr id="4" name="Volume chart"/><${prefix}:cNvGraphicFramePr/>
    ${prefix === 'p' ? '<p:nvPr/>' : ''}
    </${prefix}:nvGraphicFramePr><${prefix}:xfrm><a:off x="${x}" y="${y}"/><a:ext cx="${w}" cy="${h}"/></${prefix}:xfrm>
    <a:graphic><a:graphicData uri="${drawingNs}/chart">
      <c:chart xmlns:c="${drawingNs}/chart" r:id="${id}"/>
    </a:graphicData></a:graphic></${prefix}:graphicFrame>`
}

function slides() {
  const shape = (text) => `<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title ellipse"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
    <p:spPr><a:xfrm><a:off x="457200" y="457200"/><a:ext cx="5334000" cy="1143000"/></a:xfrm>
      <a:prstGeom prst="ellipse"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="FFCC00"/></a:solidFill>
    </p:spPr><p:txBody><a:bodyPr anchor="ctr"/><a:lstStyle/><a:p><a:pPr algn="ctr"/>
      <a:r><a:rPr sz="3000"/><a:t>${text}</a:t></a:r></a:p></p:txBody></p:sp>`
  const picture = `<p:pic><p:nvPicPr><p:cNvPr id="3" name="Sample PNG"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>
    <p:blipFill><a:blip r:embed="picture"/><a:stretch><a:fillRect/></a:stretch></p:blipFill>
    <p:spPr><a:xfrm><a:off x="9144000" y="457200"/><a:ext cx="1219200" cy="914400"/></a:xfrm>
      <a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic>`
  const labels = ['Visible slide alpha', 'Hidden slide sentinel', 'Visible slide omega']
  return packageFile('ppt/presentation.xml', [
    ['ppt/presentation.xml', `<p:presentation xmlns:p="${presentationNs}" xmlns:r="${relNs}">
      <p:sldIdLst>${labels.map((_, i) => `<p:sldId id="${256 + i}" r:id="slide${i}"/>`).join('')}</p:sldIdLst>
      <p:sldSz cx="12192000" cy="6858000"/><p:notesSz cx="6858000" cy="9144000"/></p:presentation>`, 'presentationml.presentation.main'],
    ['ppt/_rels/presentation.xml.rels', relationships(labels.map((_, i) => [`slide${i}`, 'slide', `slides/slide${i + 1}.xml`]))],
    ...labels.map((label, i) => [`ppt/slides/slide${i + 1}.xml`, `<p:sld xmlns:p="${presentationNs}"
      xmlns:a="${drawingNs}/main" xmlns:r="${relNs}"${i === 1 ? ' show="0"' : ''}>
      <p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
        <p:grpSpPr/>${shape(label)}${i === 0 ? picture + frame('p', 'chart', 457200, 1828800, 6096000, 3810000) : ''}
      </p:spTree></p:cSld></p:sld>`, 'presentationml.slide']),
    ['ppt/slides/_rels/slide1.xml.rels', relationships([
      ['picture', 'image', '../media/chart.png'], ['chart', 'chart', '../charts/chart1.xml'],
    ])],
    ['ppt/media/chart.png', png],
    ['ppt/charts/chart1.xml', chart('Slide volume chart'), 'drawingml.chart'],
  ])
}

function workbook(chartTitle = 'Workbook volume chart') {
  const textCell = (address, text, style = 0) =>
    `<c r="${address}" s="${style}" t="inlineStr"><is><t>${text}</t></is></c>`
  const edges = ['left', 'right', 'top', 'bottom'].map((edge) =>
    `<${edge} style="thick"><color rgb="FFCC0000"/></${edge}>`).join('')
  return packageFile('xl/workbook.xml', [
    ['xl/workbook.xml', `<workbook xmlns="${sheetNs}" xmlns:r="${relNs}"><sheets>
      <sheet name="Metrics" sheetId="1" r:id="sheet"/></sheets></workbook>`, 'spreadsheetml.sheet.main'],
    ['xl/_rels/workbook.xml.rels', relationships([['sheet', 'worksheet', 'worksheets/sheet1.xml'], ['styles', 'styles', 'styles.xml']])],
    ['xl/styles.xml', `<styleSheet xmlns="${sheetNs}">
      <fonts count="1"><font><sz val="11"/><name val="Arial"/></font></fonts>
      <fills count="3"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill>
        <fill><patternFill patternType="solid"><fgColor rgb="FF00CC00"/><bgColor indexed="64"/></patternFill></fill></fills>
      <borders count="2"><border><left/><right/><top/><bottom/><diagonal/></border><border>${edges}<diagonal/></border></borders>
      <cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
      <cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>
        <xf numFmtId="4" fontId="0" fillId="2" borderId="1" xfId="0" applyNumberFormat="1" applyFill="1" applyBorder="1"/></cellXfs>
    </styleSheet>`, 'spreadsheetml.styles'],
    ['xl/worksheets/sheet1.xml', `<worksheet xmlns="${sheetNs}" xmlns:r="${relNs}">
      <dimension ref="A1:B2"/><cols><col min="1" max="2" width="24" customWidth="1"/></cols>
      <sheetData><row r="1">${textCell('A1', 'Workbook sentinel', 1)}${textCell('B1', 'Revenue')}</row>
      <row r="2"><c r="A2" s="1"><v>1234.5</v></c></row></sheetData><drawing r:id="drawing"/>
    </worksheet>`, 'spreadsheetml.worksheet'],
    ['xl/worksheets/_rels/sheet1.xml.rels', relationships([['drawing', 'drawing', '../drawings/drawing1.xml']])],
    ['xl/drawings/drawing1.xml', `<xdr:wsDr xmlns:xdr="${drawingNs}/spreadsheetDrawing" xmlns:a="${drawingNs}/main" xmlns:r="${relNs}">
      <xdr:oneCellAnchor><xdr:from><xdr:col>0</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>3</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
        <xdr:ext cx="4572000" cy="2667000"/>${frame('xdr', 'chart', 0, 0, 0, 0)}<xdr:clientData/>
      </xdr:oneCellAnchor></xdr:wsDr>`, 'drawing'],
    ['xl/drawings/_rels/drawing1.xml.rels', relationships([['chart', 'chart', '../charts/chart1.xml']])],
    ['xl/charts/chart1.xml', chart(chartTitle), 'drawingml.chart'],
  ])
}

/** A sheet of 80 short rows, which prints on three landscape pages. */
function tallWorkbook() {
  const rows = Array.from({ length: 80 }, (_, i) =>
    `<row r="${i + 1}"><c r="A${i + 1}" t="inlineStr"><is><t>${tallLabel(i + 1)}</t></is></c></row>`).join('')
  return packageFile('xl/workbook.xml', [
    ['xl/workbook.xml', `<workbook xmlns="${sheetNs}" xmlns:r="${relNs}"><sheets>
      <sheet name="Tall" sheetId="1" r:id="sheet"/></sheets></workbook>`, 'spreadsheetml.sheet.main'],
    ['xl/_rels/workbook.xml.rels', relationships([['sheet', 'worksheet', 'worksheets/sheet1.xml']])],
    ['xl/worksheets/sheet1.xml', `<worksheet xmlns="${sheetNs}"><sheetFormatPr defaultRowHeight="15" customHeight="1"/>
      <sheetData>${rows}</sheetData></worksheet>`, 'spreadsheetml.worksheet'],
  ])
}
const tallLabel = (n) => `Line ${String(n).padStart(3, '0')}`

function wordSvg(svg) {
  return word(
    paragraph('Unsupported SVG sentinel') +
    `<w:p><w:r><w:drawing><wp:inline>
      <wp:extent cx="914400" cy="685800"/><wp:docPr id="9" name="Svg"/>
      <a:graphic><a:graphicData uri="${drawingNs}/picture"><pic:pic>
        <pic:nvPicPr><pic:cNvPr id="9" name="Svg"/><pic:cNvPicPr/></pic:nvPicPr>
        <pic:blipFill><a:blip r:embed="svg"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>
        <pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="685800"/></a:xfrm>
        <a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr>
      </pic:pic></a:graphicData></a:graphic>
    </wp:inline></w:drawing></w:r></w:p>`,
    '',
    [
      ['word/_rels/document.xml.rels', relationships([['svg', 'image', 'media/picture.svg']])],
      ['word/media/picture.svg', Buffer.from(svg)],
    ],
  )
}

// These files use ordinary indirect objects and direct stream lengths. Advance by byte
// length so compressed data cannot be mistaken for a dictionary, object, or end marker.
function pdfObjects(bytes) {
  const raw = bytes.toString('latin1')
  assert.match(raw, /^%PDF-1\.\d\r?\n/)
  assert.match(raw, /%%EOF\s*$/)
  const objects = new Map()
  const header = /^(\d+)\s+\d+\s+obj\b/gm
  const boundary = /\b(?:stream\r?\n|endobj\b)/g
  for (let match; (match = header.exec(raw)); ) {
    boundary.lastIndex = header.lastIndex
    const end = boundary.exec(raw)
    assert.ok(end, `Unterminated PDF object ${match[1]}`)
    const dict = raw.slice(header.lastIndex, end.index)
    let stream
    if (end[0] !== 'endobj') {
      const length = dict.match(/\/Length\s+(\d+)\b(?!\s+\d+\s+R)/)
      assert.ok(length, `Missing direct stream length on object ${match[1]}`)
      const streamEnd = boundary.lastIndex + Number(length[1])
      stream = bytes.subarray(boundary.lastIndex, streamEnd)
      const terminator = raw.slice(streamEnd, streamEnd + 40).match(/^\s*endstream\s*endobj\b/)
      assert.ok(terminator, `Incorrect stream length on object ${match[1]}`)
      header.lastIndex = streamEnd + terminator[0].length
    } else {
      header.lastIndex = boundary.lastIndex
    }
    objects.set(Number(match[1]), { dict, stream })
  }
  return objects
}

function streamText(object) {
  assert.ok(object?.stream, 'Missing PDF stream')
  if (/\/FlateDecode\b/.test(object.dict)) return inflateSync(object.stream).toString('latin1')
  assert.doesNotMatch(object.dict, /\/Filter\b/, 'Unexpected page content filter')
  return object.stream.toString('latin1')
}

function inspectBinary(bytes, sizes, imageCount, smaskCount = 0, imageBound = 180) {
  const objects = pdfObjects(bytes)
  const entries = [...objects.values()]
  const pages = entries.filter(({ dict }) => /\/Type\s*\/Page\b/.test(dict))
  assert.equal(pages.length, sizes.length, 'Raw PDF page count')
  // PDFium can renumber or reorder the indirect objects when saving the bundle.
  const mediaBoxes = pages.map(({ dict }) => {
    const box = dict.match(/\/MediaBox\s*\[([^\]]+)\]/)
    assert.ok(box, 'Missing page MediaBox')
    const [x1, y1, x2, y2] = box[1].trim().split(/\s+/).map(Number)
    return [x2 - x1, y2 - y1]
  })
  assert.deepEqual(mediaBoxes.sort(), [...sizes].sort(), 'Raw PDF MediaBox dimensions')
  const content = pages.map(({ dict }) => {
    const refs = dict.match(/\/Contents\s*(\[[^\]]+\]|\d+\s+\d+\s+R)/)
    assert.ok(refs, 'Missing page contents')
    const commands = [...refs[1].matchAll(/(\d+)\s+\d+\s+R/g)]
      .map(([, id]) => streamText(objects.get(Number(id)))).join('\n')
    assert.match(commands, /(?:^|\s)BT\s[\s\S]*?\sET(?:\s|$)/, 'Missing selectable vector text operators')
    assert.doesNotMatch(commands, /(?:^|\s)BI(?:\s|$)/, 'Unexpected inline raster image')
    return commands
  }).join('\n')

  const images = entries.filter(({ dict }) => /\/Subtype\s*\/Image\b/.test(dict))
  assert.equal(images.length, imageCount + smaskCount, 'Unexpected raster image XObjects')
  const draws = [...content.matchAll(/\/\S+\s+Do\b/g)]
  assert.equal(draws.length, imageCount, 'Unexpected raster draw count')
  for (const draw of draws) {
    const transform = content.slice(Math.max(0, draw.index - 150), draw.index)
      .match(/(-?[\d.]+) 0 0 (-?[\d.]+) -?[\d.]+ -?[\d.]+ cm\s*$/)
    assert.ok(transform, 'Missing image placement transform')
    assert.ok(Math.abs(Number(transform[1])) <= imageBound && Math.abs(Number(transform[2])) <= imageBound, `Image exceeds fixture bounds: ${transform[1]} x ${transform[2]}`)
  }
  return { entries, images, content }
}

function checkPngImage(images) {
  const png = images.find(({ dict }) => /\/FlateDecode\b/.test(dict))
  assert.ok(png, 'Missing losslessly compressed PNG XObject')
  assert.match(png.dict, /\/Width\s+32\b/)
  assert.match(png.dict, /\/Height\s+24\b/)
  const left = Buffer.from([232, 76, 32])
  const right = Buffer.from([60, 120, 216])
  const row = Buffer.concat([...Array(16).fill(left), ...Array(16).fill(right)])
  assert.deepEqual(inflateSync(png.stream), Buffer.concat(Array(24).fill(row)), 'PNG pixels should survive export')
}

async function palette(page, label) {
  const modifier = await page.evaluate(() => navigator.platform.startsWith('Mac') ? 'Meta' : 'Control')
  await page.keyboard.press(`${modifier}+k`)
  await page.locator('.palette input').fill(label)
  await page.keyboard.press('Enter')
}

async function openFixture(page, file, base64) {
  const id = await page.evaluate(({ file, base64 }) =>
    window.__workspace.useWorkspace.getState().openUnsaved(Uint8Array.from(atob(base64), (c) => c.charCodeAt(0)), file),
  { file, base64 })
  assert.ok(id, `Could not open fixture ${file}`)
}

async function inspectExport(page, { sizes, expected, contiguous = [], imageCount = 0, smaskCount = 0, imageBound, absent = [], annotations = false, zoom = false }) {
  const view = page.locator('.view[data-active=true] .pdf-pages [data-page-number="1"]')
  await view.waitFor()
  const result = await page.evaluate(async ({ expected, absent, annotations, zoom }) => {
    const { tabs, activeId } = window.__workspace.useWorkspace.getState()
    const tab = tabs.find((candidate) => candidate.id === activeId)
    if (tab?.kind !== 'pdf' || !tab.bytes?.length) throw new Error('No exported PDF in the active tab')
    const viewerEngine = [...window.__pdfEngines][0]
    if (!viewerEngine) throw new Error('No PDFium test engine')
    // Use the same worker API as the viewer and open the exact bytes inspected on the Node side.
    const engine = await viewerEngine.constructor.create()
    const signal = AbortSignal.timeout(15000)
    const call = (method, ...args) => engine.call(method, args, { signal })
    try {
      const { pages } = await call('open', tab.bytes.slice().buffer, '')
      const details = []
      for (const [index, info] of pages.entries()) {
        const runs = await call('text', info.key)
        const hits = {}
        for (const phrase of [...(expected[index] ?? []), ...absent]) {
          const matches = await call('search', info.key, phrase, true)
          hits[phrase] = matches.some((boxes) => boxes.some(({ w, h }) => w > 0 && h > 0))
        }
        const links = annotations ? (await call('annotations', info.key)).filter((annot) => annot.link) : []
        details.push({ info, runs, hits, links })
      }
      let tile
      if (zoom) {
        const rect = { x: 36, y: 36, w: 440, h: 92 }
        const { width, height, bands } = await call('render', pages[0].key, 4, rect)
        const canvas = new OffscreenCanvas(width, height)
        const ctx = canvas.getContext('2d', { willReadFrequently: true })
        for (const { y, height: rows, image, data } of bands) {
          if (image) {
            ctx.drawImage(image, 0, y)
            image.close()
          } else {
            ctx.putImageData(new ImageData(new Uint8ClampedArray(data), width, rows), 0, y)
          }
        }
        const { data } = ctx.getImageData(0, 0, width, height)
        let ink = 0
        let gold = 0
        let white = 0
        for (let i = 0; i < data.length; i += 4) {
          const r = data[i]
          const g = data[i + 1]
          const b = data[i + 2]
          if (r < 30 && g < 30 && b < 30) ink++
          if (r === 255 && g === 204 && b === 0) gold++
          if (r === 255 && g === 255 && b === 255) white++
        }
        tile = { width, height, ink, gold, white }
      }
      return { details, tile, bytes: Array.from(tab.bytes) }
    } finally {
      engine.destroy()
    }
  }, { expected, absent, annotations, zoom })
  const bytes = Buffer.from(result.bytes)
  const binary = inspectBinary(bytes, sizes, imageCount, smaskCount, imageBound)
  assert.deepEqual(result.details.map(({ info }) => [info.width, info.height]), sizes, 'PDFium page dimensions and count')
  for (const [i, { runs, hits }] of result.details.entries()) {
    const missing = {
      search: expected[i].filter((phrase) => !hits[phrase]),
      text: (contiguous[i] ?? []).filter((phrase) => !runs.some(({ text }) => text.includes(phrase))),
    }
    assert.deepEqual(missing, { search: [], text: [] },
      `PDFium search and text on page ${i + 1}. Extracted runs: ${JSON.stringify(runs.map(({ text }) => text))}`)
    for (const phrase of absent) assert.equal(hits[phrase], false, `Found unexpected "${phrase}" on page ${i + 1}`)
  }
  return { ...binary, details: result.details, tile: result.tile, bytes }
}

const basicLabel = 'Selectable vector PDF text'
const wordFirst = 'First page sentinel'
const wordSecond = 'Second page sentinel'
const pageBreak = '<w:p><w:r><w:br w:type="page"/></w:r></w:p>'
const slideLabels = ['Visible slide alpha', 'Visible slide omega']
const sheetLabel = 'Workbook sentinel'
const rtlPhrases = ['שלום עולם', 'مرحبا بالعالم']
const rtl = (text) => `<w:p><w:pPr><w:bidi/></w:pPr><w:r><w:rPr><w:rtl/></w:rPr><w:t>${text}</w:t></w:r></w:p>`

const { browser, page, logs } = await launch()
let failures = 0
try {
  await page.goto(BASE)
  await page.waitForFunction(() => !!window.__workspace)
  const paginated = word(paragraph(wordFirst) + pageBreak + paragraph(wordSecond))
  const deck = slides()
  const book = workbook()
  const cases = [
    {
      name: 'Basic Word', file: 'basic.docx', source: word(paragraph(basicLabel)),
      sizes: [letter], expected: [[basicLabel]],
      check({ bytes, details, entries }) {
        assert.match(bytes.toString('latin1', 0, 10), /^%PDF-1\.7\r?\n/)
        assert.ok(!entries.some(({ dict }) => dict.includes('/DCTDecode')), 'Unexpected JPEG fallback')
        assert.deepEqual(details[0].runs.map(({ text }) => text), [basicLabel])
        for (const { text, map, box } of details[0].runs) {
          assert.equal(map.length, text.length, 'Missing selectable character indices')
          assert.ok(box.w > 0 && box.h > 0, 'Missing selectable text rectangle')
        }
      },
    },
    {
      name: 'Word page break', file: 'pages.docx', source: paginated,
      sizes: [letter, letter], expected: [[wordFirst], [wordSecond]],
      check({ details }) {
        assert.deepEqual(details.map(({ runs }) => runs.map(({ text }) => text)), [[wordFirst], [wordSecond]])
      },
    },
    {
      name: 'Word table borders and shading', file: 'table.docx', source: table(),
      sizes: [letter], expected: [['Shaded cell', 'Bordered cell']],
      check({ content }) {
        assert.match(content, /0 0\.8 0 rg\s+[\d.\s]+ re\s+f\b/, 'Missing green vector cell fill')
        assert.match(content, /0\.8 0 0 RG\b/, 'Missing red vector border stroke')
        assert.match(content, /\bm\s+[\d.\s]+ l\s+S\b/, 'Missing stroked border path')
      },
    },
    {
      name: 'Word headers, footers and footnotes', file: 'notes.docx', source: wordFurniture(),
      sizes: [letter], expected: [['Header sentinel', 'Footer sentinel', 'Footnote sentinel']],
      check({ entries }) {
        for (const { dict } of entries.filter(({ dict }) => /\/Type\s*\/ExtGState\b/.test(dict))) {
          for (const [, alpha] of dict.matchAll(/\/(?:ca|CA)\s+([\d.]+)/g)) {
            assert.equal(Number(alpha), 1, 'Header/footer dimming leaked into export')
          }
        }
      },
    },
    {
      name: 'Word pictures, crop and links', file: 'pictures.docx', source: wordPictures(),
      sizes: [letter], expected: [['Writan homepage', 'Script link label', 'Local link label']],
      imageCount: 2, annotations: true,
      check({ entries, images, content, details }) {
        checkPngImage(images)
        const embedded = images.find(({ dict }) => /\/DCTDecode\b/.test(dict))
        assert.ok(embedded, 'Missing JPEG XObject')
        assert.deepEqual(embedded.stream, jpeg, 'JPEG must be embedded byte for byte')
        assert.match(content, /\bre\s+W\s+n\s+[^qQ]+\bcm\s+\/\S+ Do\b/, 'Missing cropped image clip')
        const links = entries.filter(({ dict }) => /\/Subtype\s*\/Link\b/.test(dict))
        assert.ok(links.length > 0, 'Missing raw link annotations')
        assert.ok(links.every(({ dict }) => /\/URI\s*\(https:\/\/writan\.app\)/.test(dict)), 'Unexpected raw link annotation')
        assert.ok(details[0].links.length > 0, 'PDFium did not expose the link')
        assert.ok(details[0].links.every(({ box, link }) => box.w > 0 && box.h > 0 && link.url === 'https://writan.app'),
          'Expected only clickable HTTPS link annotations')
      },
    },
    {
      name: 'Hebrew, Arabic and mixed direction', file: 'bidi.docx',
      source: word(rtlPhrases.map(rtl).join('') + paragraph('Hello שלום 2026') + rtl('Hello שלום 2026')),
      sizes: [letter], expected: [[...rtlPhrases, 'Hello שלום 2026', 'Hello', '2026']],
      contiguous: [rtlPhrases],
      check({ entries, details }) {
        const texts = details[0].runs.map(({ text }) => text)
        // PDFium may reorder LTR words within the RTL paragraph. Verify each LTR token survives.
        for (const token of ['Hello', '2026']) assert.equal(texts.filter((text) => text.includes(token)).length, 2)
        assert.ok(entries.some(({ dict }) => /\/ToUnicode\b/.test(dict)), 'Missing Unicode mapping')
      },
    },
    {
      name: 'Word OMML and LaTeX equations', file: 'equations.docx', source: mathWord(),
      sizes: [letter], expected: [['Native equation', '317/23', 'LaTeX equation']],
      check({ content, details }) {
        // Word's native OMML display list linearizes the fraction as selectable text.
        assert.ok(details[0].runs.some(({ text }) => text.includes('317/23')), 'Missing native fraction text')
        // The tagged equation uses MathJax outlines, identifiable by their explicit magenta fill.
        assert.match(content, /0\.8 0 0\.8 rg[^qQ]*\bc\s[^qQ]*\bf\b/, 'Missing LaTeX glyph outlines')
      },
    },
    {
      name: 'PowerPoint vectors, chart, image and hidden slide', file: 'deck.pptx', source: deck,
      sizes: [wideSlide, wideSlide], expected: [[slideLabels[0], 'Slide volume chart', 'Harbor', 'Summit'], [slideLabels[1]]],
      absent: ['Hidden slide sentinel'], imageCount: 1, zoom: true,
      check({ content, images, tile }) {
        assert.match(content, /1 0\.8 0 rg[\s\S]*?\bc\s/, 'Missing cubic ellipse path')
        assert.match(content, /0 0 0\.8 rg\s+[\d.\s]+ re\s+f\b/, 'Missing vector slide chart bars')
        checkPngImage(images)
        assert.deepEqual([tile.width, tile.height], [1760, 368], 'Expected a 4x render crop')
        assert.ok(tile.ink > 500 && tile.gold > 10000 && tile.white > 10000, `Missing text, ellipse fill, or background at 4x: ${JSON.stringify(tile)}`)
      },
    },
    {
      name: 'Excel styled cells and vector chart', file: 'book.xlsx', source: book,
      sizes: [landscape], expected: [[sheetLabel, '1,234.50', 'Workbook volume chart', 'Harbor', 'Summit']],
      check({ content }) {
        assert.match(content, /0 0\.8 0 rg\s+[\d.\s]+ re\s+f\b/, 'Missing cell fill')
        assert.match(content, /0\.8 0 0 (?:rg|RG)\b/, 'Missing colored cell border')
        const bars = content.match(/0 0 0\.8 rg\s+[\s\S]*?\bc\s[\s\S]*?\bf\b/g) ?? []
        assert.equal(bars.length, 2, 'Expected two rounded vector chart bars')
      },
    },
    {
      name: 'Word CJK hybrid text renders crisp glyphs cleanly', file: 'cjk.docx',
      source: word(`<w:p><w:r><w:rPr><w:u w:val="single"/></w:rPr><w:t>東京 売上 𠮷</w:t></w:r></w:p>` + paragraph('안녕하세요')),
      sizes: [letter], expected: [['東京 売上', '안녕하세요']], contiguous: [['東京 売上', '안녕하세요']],
      imageCount: 2, smaskCount: 2, zoom: true,
      check({ content, images, tile }) {
        assert.equal(images.length, 4, 'Expected two CJK fallback PNG overlays with two alpha SMasks')
        assert.ok(images.every(({ dict }) => /\/FlateDecode\b/.test(dict)), 'Expected lossless PNG overlays for CJK runs')
        assert.match(content, /\b3 Tr\b/, 'Expected invisible searchable text layer alongside CJK PNG overlays')
        assert.match(content, /\/ActualText\s*<FEFFD842DFB7>/i, 'Expected UTF-16BE /ActualText for supplementary CJK U+20BB7')
        assert.match(content, /\bm\s+[\d.\s]+ l\s+S\b/, 'Expected vector underline stroke on underlined CJK run')
        assert.ok(tile.ink > 100, `Expected visible dark CJK ink pixels at 4x: ${JSON.stringify(tile)}`)
      },
    },
    {
      name: 'Word SVG with hyphenated namespace element triggers raster fallback', file: 'svg-ext.docx',
      source: word(
        paragraph('Hyphenated SVG namespace sentinel') +
        `<w:p><w:r><w:drawing><wp:inline>
          <wp:extent cx="914400" cy="685800"/><wp:docPr id="9" name="ExtSvg"/>
          <a:graphic><a:graphicData uri="${drawingNs}/picture"><pic:pic>
            <pic:nvPicPr><pic:cNvPr id="9" name="ExtSvg"/><pic:cNvPicPr/></pic:nvPicPr>
            <pic:blipFill><a:blip r:embed="svgext"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>
            <pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="685800"/></a:xfrm>
            <a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr>
          </pic:pic></a:graphicData></a:graphic>
        </wp:inline></w:drawing></w:r></w:p>`,
        '',
        [
          ['word/_rels/document.xml.rels', relationships([['svgext', 'image', 'media/ext.svg']])],
          ['word/media/ext.svg', Buffer.from(
            `<svg xmlns="http://www.w3.org/2000/svg" xmlns:svg-ext="http://www.w3.org/2000/svg" width="32" height="24" viewBox="0 0 32 24">` +
            `<svg-ext:image href="data:image/png;base64,${png.toString('base64')}" width="32" height="24"/></svg>`,
          )],
        ],
      ),
      sizes: [letter], expected: [['Hyphenated SVG namespace sentinel']], imageCount: 1,
      check({ images }) {
        const raster = images.find(({ dict }) => /\/FlateDecode\b/.test(dict))
        assert.ok(raster, 'Expected <svg-ext:image> to trigger raster PNG fallback instead of being dropped')
        assert.match(raster.dict, /\/Width\s+128\b/)
        assert.match(raster.dict, /\/Height\s+96\b/)
      },
    },
    {
      name: 'Excel rows split across pages are searchable on one page each', file: 'tall.xlsx', source: tallWorkbook(),
      sizes: [landscape, landscape, landscape], expected: [[tallLabel(1)], [], [tallLabel(80)]],
      check({ details }) {
        const pages = details.map(({ runs }) => runs.map(({ text }) => text).join('\n'))
        for (let n = 1; n <= 80; n++) {
          const found = pages.flatMap((text, page) => text.includes(tallLabel(n)) ? [page + 1] : [])
          assert.equal(found.length, 1, `${tallLabel(n)} is on pages ${JSON.stringify(found)}`)
        }
      },
    },
    {
      name: 'Excel chart with CJK labels is drawn by the browser', file: 'cjk-chart.xlsx', source: workbook('売上 chart'),
      sizes: [landscape], expected: [[sheetLabel]], imageCount: 1, imageBound: 500,
      check({ content, images }) {
        assert.ok(images.every(({ dict }) => /\/FlateDecode\b/.test(dict)), 'Expected a lossless PNG chart')
        assert.doesNotMatch(content, /0 0 0\.8 rg\s+[\s\S]*?\bc\s[\s\S]*?\bf\b/, 'Chart bars should not also be vectors')
      },
    },
    {
      name: 'Word text the bundled fonts lack is drawn by the browser', file: 'scripts.docx',
      source: word(paragraph('Thai สวัสดี') + paragraph('Hindi नमस्ते')),
      sizes: [letter], expected: [['สวัสดี', 'नमस्ते']], imageCount: 2, smaskCount: 2,
      check({ content, images }) {
        assert.ok(images.every(({ dict }) => /\/FlateDecode\b/.test(dict)), 'Expected lossless PNG overlays')
        assert.match(content, /\b3 Tr\b/, 'Expected invisible searchable text under the overlays')
      },
    },
    {
      name: 'Word SVG with a DOCTYPE and a polygon is rasterized', file: 'svg-doctype.docx',
      source: wordSvg('<?xml version="1.0"?><!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">' +
        '<svg xmlns="http://www.w3.org/2000/svg" width="32" height="24"><polygon points="0,0 32,0 16,24" fill="#0000cc"/></svg>'),
      sizes: [letter], expected: [['Unsupported SVG sentinel']], imageCount: 1, smaskCount: 1,
      check({ images }) {
        const raster = images.find(({ dict }) => /\/Width\s+128\b/.test(dict))
        assert.ok(raster, 'Expected the polygon SVG as a 4x PNG')
      },
    },
    {
      name: 'Bundle Word, PowerPoint and Excel',
      sizes: [letter, letter, wideSlide, wideSlide, landscape],
      expected: [[wordFirst], [wordSecond], [slideLabels[0]], [slideLabels[1]], [sheetLabel]],
      imageCount: 1,
      async export() {
        await openFixture(page, 'pages.docx', await paginated)
        await openFixture(page, 'deck.pptx', await deck)
        await openFixture(page, 'book.xlsx', await book)
        await palette(page, 'Bundle open files')
        const dialog = page.getByRole('dialog', { name: 'Bundle open files into one PDF' })
        await dialog.getByRole('button', { name: 'Merge 3 into PDF', exact: true }).click()
      },
      check({ images }) {
        checkPngImage(images)
      },
    },
  ]

  for (const scenario of cases) {
    try {
      if (scenario.export) {
        await scenario.export()
      } else {
        await openFixture(page, scenario.file, await scenario.source)
        await palette(page, 'Export as PDF')
      }
      const result = await inspectExport(page, scenario)
      scenario.check(result)
      console.log(`PASS ${scenario.name}`)
    } catch (error) {
      failures++
      const dialog = await page.locator('.convert-error, [role="alertdialog"]').allTextContents()
      console.error(`FAIL ${scenario.name}`, error, ...dialog)
      await page.keyboard.press('Escape')
    } finally {
      await page.evaluate(async () => {
        const { tabs, closeTab } = window.__workspace.useWorkspace.getState()
        for (const tab of [...tabs].reverse()) await closeTab(tab.id)
      })
    }
  }
  try {
    // A photo converts to a plain PDF without OCR: no language packs, no text layer, and an opaque WebP stays JPEG-sized.
    const photo = await page.evaluate(async () => {
      const canvas = new OffscreenCanvas(64, 48)
      const ctx = canvas.getContext('2d')
      ctx.fillStyle = '#336699'
      ctx.fillRect(0, 0, 64, 48)
      const webp = new Uint8Array(await (await canvas.convertToBlob({ type: 'image/webp', quality: 0.9 })).arrayBuffer())
      const { convert } = await window.__loadPdfOcrTestApi()
      const ws = window.__workspace.useWorkspace.getState()
      ws.openUnsaved(webp, 'photo.webp')
      const tab = window.__workspace.useWorkspace.getState().tabs.find((t) => t.title === 'photo')
      const hint = convert.targetsForKind('image').find((t) => t.format === 'pdf')?.hint
      const { bytes } = await convert.convertTab(tab, 'pdf', { onProgress: (message) => { throw new Error(`Unexpected OCR: ${message}`) } })
      await ws.closeTab(tab.id)
      return { kind: tab.kind, hint, pdf: new TextDecoder('latin1').decode(bytes) }
    })
    assert.equal(photo.kind, 'image')
    assert.doesNotMatch(photo.hint, /OCR|recogni|searchable/i, 'Image to PDF should not promise OCR')
    assert.match(photo.pdf, /\/DCTDecode\b/, 'Expected the opaque WebP photo as JPEG')
    assert.doesNotMatch(photo.pdf, /\/Type\s*\/Font\b/, 'Expected no OCR text layer')
    console.log('PASS Image to PDF is a plain JPEG page')
  } catch (error) {
    failures++
    console.error('FAIL Image to PDF is a plain JPEG page', error)
  }
  assert.ok(!logs.some((line) => line.startsWith('[pageerror]')), logs.join('\n'))
  console.log(`Vector PDF export: ${cases.length + 1 - failures} passed, ${failures} failed`)
  process.exitCode = failures ? 1 : 0
} finally {
  await browser.close()
}
