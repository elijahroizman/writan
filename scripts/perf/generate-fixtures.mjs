import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import JSZip from 'jszip'

const root = fileURLToPath(new URL('../../', import.meta.url))
const outDir = resolve(root, '.cache/perf-fixtures')
const force = process.argv.includes('--force')

mkdirSync(outDir, { recursive: true })

function xmlEsc(s) {
  return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
}

function colLetter(idx0) {
  let n = idx0 + 1
  let s = ''
  while (n > 0) {
    const r = (n - 1) % 26
    s = String.fromCharCode(65 + r) + s
    n = Math.floor((n - 1) / 26)
  }
  return s
}

async function makeDocx(filename, targetPages) {
  const outPath = join(outDir, filename)
  if (!force && existsSync(outPath)) return outPath

  const baseBytes = readFileSync(join(root, 'scripts/tour/Library Update.docx'))
  const zip = await JSZip.loadAsync(baseBytes)
  const docXml = await zip.file('word/document.xml').async('string')
  const sectMatch = /<w:sectPr[\s\S]*?<\/w:sectPr>/.exec(docXml)
  const sectPr = sectMatch ? sectMatch[0] : '<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>'
  const docOpenMatch = /^[\s\S]*?<w:body>/.exec(docXml)
  const docPrefix = docOpenMatch ? docOpenMatch[0] : '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>'

  // Remove comments part if present so we test clean large document performance without comment-panel clutter.
  zip.remove('word/comments.xml')
  const relsXml = await zip.file('word/_rels/document.xml.rels')?.async('string')
  if (relsXml) {
    zip.file('word/_rels/document.xml.rels', relsXml.replace(/<Relationship[^>]*comments\.xml[^>]*\/>/g, ''))
  }
  const ctXml = await zip.file('[Content_Types].xml')?.async('string')
  if (ctXml) {
    zip.file('[Content_Types].xml', ctXml.replace(/<Override[^>]*comments\.xml[^>]*\/>/g, ''))
  }

  // ~9.8 rich paragraphs per page -> ~10, ~100, and ~500 pages in office-word.
  const totalParas = Math.round(targetPages * 9.8)
  const chunks = []
  const lorem = 'Quarterly operational telemetry indicates consistent throughput improvements across regional distribution nodes while maintaining strict validation invariants and predictable memory bounds during peak concurrent workloads across desktop and browser environments.'

  for (let i = 0; i < totalParas; i++) {
    if (i % 35 === 0) {
      const sec = Math.floor(i / 35) + 1
      chunks.push(`<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Section ${sec}: Performance Telemetry &amp; Architecture Overview</w:t></w:r></w:p>`)
    } else if (i % 14 === 0) {
      chunks.push(`<w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>Subsection ${i}: Workload Analysis</w:t></w:r></w:p>`)
    } else if (i % 70 === 10) {
      chunks.push(
        `<w:tbl><w:tblPr><w:tblW w:w="9360" w:type="dxa"/><w:tblBorders><w:top w:val="single" w:sz="4" w:space="0" w:color="CCCCCC"/><w:left w:val="single" w:sz="4" w:space="0" w:color="CCCCCC"/><w:bottom w:val="single" w:sz="4" w:space="0" w:color="CCCCCC"/><w:right w:val="single" w:sz="4" w:space="0" w:color="CCCCCC"/><w:insideH w:val="single" w:sz="4" w:space="0" w:color="CCCCCC"/><w:insideV w:val="single" w:sz="4" w:space="0" w:color="CCCCCC"/></w:tblBorders></w:tblPr><w:tblGrid><w:gridCol w:w="2340"/><w:gridCol w:w="2340"/><w:gridCol w:w="2340"/><w:gridCol w:w="2340"/></w:tblGrid>` +
        [0, 1, 2, 3].map((r) =>
          `<w:tr>${[0, 1, 2, 3].map((c) =>
            `<w:tc><w:tcPr><w:tcW w:w="2340" w:type="dxa"/></w:tcPr><w:p><w:r>${r === 0 ? '<w:rPr><w:b/></w:rPr>' : ''}<w:t>${r === 0 ? `Metric ${c + 1}` : `Val ${i}-${r}-${c}`}</w:t></w:r></w:p></w:tc>`
          ).join('')}</w:tr>`
        ).join('') +
        '</w:tbl>'
      )
    } else if (i % 9 === 3) {
      chunks.push(`<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>Checklist item ${i}: verify incremental layout caching and deterministic boundary stability across frames.</w:t></w:r></w:p>`)
    } else {
      chunks.push(`<w:p><w:r><w:t>Paragraph ${i + 1}. ${lorem} Record #${(i * 7919) % 100000} verified at batch ${(i % 12) + 1}.</w:t></w:r></w:p>`)
    }
  }

  zip.file('word/document.xml', `${docPrefix}${chunks.join('')}${sectPr}</w:body></w:document>`)
  const buf = await zip.generateAsync({ type: 'nodebuffer', compression: 'DEFLATE', compressionOptions: { level: 6 } })
  writeFileSync(outPath, buf)
  return outPath
}

async function makePptx(filename, slideCount) {
  const outPath = join(outDir, filename)
  if (!force && existsSync(outPath)) return outPath

  const baseBytes = readFileSync(join(root, 'scripts/tour/Spring Launch.pptx'))
  const zip = await JSZip.loadAsync(baseBytes)

  // Remove sample slides & notes so we build a clean deterministic N-slide deck.
  for (const name of Object.keys(zip.files)) {
    if (/^ppt\/(slides|notesSlides)\//.test(name)) zip.remove(name)
  }

  const palette = ['1E3A8A', '0F766E', '7C2D12', '4C1D95', '1F2937']
  const sldIds = []
  const rels = [
    '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/>',
    '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/printerSettings" Target="printerSettings/printerSettings1.bin"/>',
    '<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/presProps" Target="presProps.xml"/>',
    '<Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/viewProps" Target="viewProps.xml"/>',
    '<Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="theme/theme1.xml"/>',
    '<Relationship Id="rId6" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/tableStyles" Target="tableStyles.xml"/>',
    '<Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesMaster" Target="notesMasters/notesMaster1.xml"/>',
  ]
  const slideOverrides = []
  const slideRelXml = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout7.xml"/></Relationships>'

  for (let i = 1; i <= slideCount; i++) {
    const rid = `rId${10 + i}`
    const sid = 255 + i
    sldIds.push(`<p:sldId id="${sid}" r:id="${rid}"/>`)
    rels.push(`<Relationship Id="${rid}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide${i}.xml"/>`)
    slideOverrides.push(`<Override PartName="/ppt/slides/slide${i}.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>`)

    const accent = palette[i % palette.length]
    const tableXml = i % 5 === 0
      ? `<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="5" name="Table 1"/><p:cNvGraphicFramePr><a:graphicFrameLocks noGrp="1"/></p:cNvGraphicFramePr><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="6800000" y="2100000"/><a:ext cx="4500000" cy="2700000"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl><a:tblPr firstRow="1" bandRow="1"/><a:tblGrid><a:gridCol w="1500000"/><a:gridCol w="1500000"/><a:gridCol w="1500000"/></a:tblGrid>${[0, 1, 2].map((r) => `<a:tr h="700000">${[0, 1, 2].map((c) => `<a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr sz="1400"/><a:t>S${i} R${r}C${c}</a:t></a:r></a:p></a:txBody><a:tcPr><a:solidFill><a:srgbClr val="${r === 0 ? accent : 'F1F5F9'}"/></a:solidFill></a:tcPr></a:tc>`).join('')}</a:tr>`).join('')}</a:tbl></a:graphicData></a:graphic></p:graphicFrame>`
      : ''

    const slideXml = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill><a:effectLst/></p:bgPr></p:bg>
<p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>
<p:sp><p:nvSpPr><p:cNvPr id="2" name="Banner"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="12191695" cy="900000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="${accent}"/></a:solidFill></p:spPr></p:sp>
<p:sp><p:nvSpPr><p:cNvPr id="3" name="Title"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="600000" y="150000"/><a:ext cx="10800000" cy="600000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr sz="2800" b="1"><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill><a:latin typeface="Calibri"/></a:rPr><a:t>Slide ${i}: Operational Review &amp; Velocity Metrics</a:t></a:r></a:p></p:txBody></p:sp>
<p:sp><p:nvSpPr><p:cNvPr id="4" name="Body"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="600000" y="1300000"/><a:ext cx="5800000" cy="4800000"/></a:xfrm><a:prstGeom prst="roundRect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="F8FAFC"/></a:solidFill><a:ln w="12700"><a:solidFill><a:srgbClr val="CBD5E1"/></a:solidFill></a:ln></p:spPr><p:txBody><a:bodyPr wrap="square"/><a:lstStyle/>
<a:p><a:r><a:rPr sz="1800" b="1"><a:solidFill><a:srgbClr val="0F172A"/></a:solidFill></a:rPr><a:t>Key Takeaways for Milestone ${i}</a:t></a:r></a:p>
<a:p><a:r><a:rPr sz="1600"><a:solidFill><a:srgbClr val="334155"/></a:solidFill></a:rPr><a:t>• Throughput target achieved across segment ${(i % 8) + 1}.</a:t></a:r></a:p>
<a:p><a:r><a:rPr sz="1600"><a:solidFill><a:srgbClr val="334155"/></a:solidFill></a:rPr><a:t>• Render cache efficiency improved by ${12 + (i % 30)}%.</a:t></a:r></a:p>
<a:p><a:r><a:rPr sz="1600"><a:solidFill><a:srgbClr val="334155"/></a:solidFill></a:rPr><a:t>• Baseline frame budget maintained under 16.7ms.</a:t></a:r></a:p>
</p:txBody></p:sp>
${tableXml}
</p:spTree></p:cSld></p:sld>`

    zip.file(`ppt/slides/slide${i}.xml`, slideXml)
    zip.file(`ppt/slides/_rels/slide${i}.xml.rels`, slideRelXml)
  }

  const presXml = await zip.file('ppt/presentation.xml').async('string')
  zip.file('ppt/presentation.xml', presXml.replace(/<p:sldIdLst>[\s\S]*?<\/p:sldIdLst>/, `<p:sldIdLst>${sldIds.join('')}</p:sldIdLst>`))
  zip.file('ppt/_rels/presentation.xml.rels', `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">${rels.join('')}</Relationships>`)
  let ctXml = await zip.file('[Content_Types].xml').async('string')
  ctXml = ctXml.replace(/<Override PartName="\/ppt\/(slides|notesSlides)\/[^"]+"[^>]*\/>/g, '')
  ctXml = ctXml.replace('</Types>', `${slideOverrides.join('')}</Types>`)
  zip.file('[Content_Types].xml', ctXml)

  const buf = await zip.generateAsync({ type: 'nodebuffer', compression: 'DEFLATE', compressionOptions: { level: 6 } })
  writeFileSync(outPath, buf)
  return outPath
}

function buildXlsxBase({ sheets, sharedStrings, stylesXml }) {
  const zip = new JSZip()
  zip.file('[Content_Types].xml',
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
    '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">' +
    '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>' +
    '<Default Extension="xml" ContentType="application/xml"/>' +
    '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>' +
    '<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>' +
    '<Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/>' +
    sheets.map((_, i) => `<Override PartName="/xl/worksheets/sheet${i + 1}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>`).join('') +
    '</Types>')
  zip.file('_rels/.rels',
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' +
    '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>' +
    '</Relationships>')
  zip.file('xl/workbook.xml',
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
    '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">' +
    '<sheets>' + sheets.map((s, i) => `<sheet name="${xmlEsc(s.name)}" sheetId="${i + 1}" r:id="rId${i + 1}"/>`).join('') + '</sheets></workbook>')
  zip.file('xl/_rels/workbook.xml.rels',
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' +
    sheets.map((_, i) => `<Relationship Id="rId${i + 1}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet${i + 1}.xml"/>`).join('') +
    `<Relationship Id="rId${sheets.length + 1}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>` +
    `<Relationship Id="rId${sheets.length + 2}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/>` +
    '</Relationships>')
  zip.file('xl/styles.xml', stylesXml)
  zip.file('xl/sharedStrings.xml',
    `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="${sharedStrings.length}" uniqueCount="${sharedStrings.length}">` +
    sharedStrings.map((s) => `<si><t>${xmlEsc(s)}</t></si>`).join('') + '</sst>')
  sheets.forEach((s, i) => zip.file(`xl/worksheets/sheet${i + 1}.xml`, s.xml))
  return zip.generateAsync({ type: 'nodebuffer', compression: 'DEFLATE', compressionOptions: { level: 6 } })
}

const DEFAULT_STYLES_XML =
  '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
  '<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">' +
  '<numFmts count="2"><numFmt numFmtId="164" formatCode="&quot;$&quot;#,##0.00"/><numFmt numFmtId="165" formatCode="0.00%"/></numFmts>' +
  '<fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><sz val="11"/><name val="Calibri"/></font></fonts>' +
  '<fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills>' +
  '<borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>' +
  '<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>' +
  '<cellXfs count="4">' +
  '<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>' +
  '<xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/>' +
  '<xf numFmtId="164" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>' +
  '<xf numFmtId="165" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>' +
  '</cellXfs>' +
  '<dxfs count="1"><dxf><fill><patternFill patternType="solid"><fgColor rgb="FFFEE2E2"/></patternFill></fill></dxf></dxfs>' +
  '</styleSheet>'

async function makePlainSheet(filename, dataRows) {
  const outPath = join(outDir, filename)
  if (!force && existsSync(outPath)) return outPath

  const sst = ['Category', 'Region', 'Units', 'Revenue', 'Margin', 'North', 'South', 'East', 'West', 'Alpha', 'Beta', 'Gamma', 'Delta']
  const rowsXml = ['<row r="1"><c r="A1" s="1" t="s"><v>0</v></c><c r="B1" s="1" t="s"><v>1</v></c><c r="C1" s="1" t="s"><v>2</v></c><c r="D1" s="1" t="s"><v>3</v></c><c r="E1" s="1" t="s"><v>4</v></c></row>']
  for (let i = 0; i < dataRows; i++) {
    const r = i + 2
    const cat = 9 + (i % 4)
    const reg = 5 + (i % 4)
    const units = 10 + ((i * 7) % 500)
    const rev = ((units * 19.75) + (i % 97)).toFixed(2)
    const margin = (0.12 + ((i % 25) * 0.01)).toFixed(4)
    rowsXml.push(`<row r="${r}"><c r="A${r}" t="s"><v>${cat}</v></c><c r="B${r}" t="s"><v>${reg}</v></c><c r="C${r}"><v>${units}</v></c><c r="D${r}" s="2"><v>${rev}</v></c><c r="E${r}" s="3"><v>${margin}</v></c></row>`)
  }
  const sheetXml =
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
    '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">' +
    `<dimension ref="A1:E${dataRows + 1}"/>` +
    '<cols><col min="1" max="16384" width="12" customWidth="1"/></cols>' +
    `<sheetData>${rowsXml.join('')}</sheetData></worksheet>`
  const buf = await buildXlsxBase({ sheets: [{ name: 'Data', xml: sheetXml }], sharedStrings: sst, stylesXml: DEFAULT_STYLES_XML })
  writeFileSync(outPath, buf)
  return outPath
}

async function makeFormulaSheet(filename) {
  const outPath = join(outDir, filename)
  if (!force && existsSync(outPath)) return outPath

  const N = 1500
  const sst = ['Key', 'Category', 'Price', 'Qty', 'Total', 'Tier', 'LookupTotal', 'CatSum']
  const sstMap = new Map(sst.map((s, i) => [s, i]))
  const intern = (s) => {
    let idx = sstMap.get(s)
    if (idx === undefined) {
      idx = sst.length
      sst.push(s)
      sstMap.set(s, idx)
    }
    return idx
  }
  const cats = ['CatA', 'CatB', 'CatC', 'CatD', 'CatE']
  for (const c of cats) intern(c)
  const highIdx = intern('High')
  const lowIdx = intern('Low')
  const keys = []
  for (let i = 1; i <= N; i++) keys.push(intern(`ITEM-${String(i).padStart(4, '0')}`))

  const catTotals = new Map()
  const rows1 = [
    `<row r="1">${sst.slice(0, 8).map((_, c) => `<c r="${colLetter(c)}1" s="1" t="s"><v>${c}</v></c>`).join('')}</row>`,
  ]
  const totals = []
  for (let i = 1; i <= N; i++) {
    const price = 15 + ((i * 13) % 450)
    const qty = 1 + ((i * 3) % 20)
    const tot = price * qty
    totals.push(tot)
    const cat = cats[i % cats.length]
    catTotals.set(cat, (catTotals.get(cat) ?? 0) + tot)
  }
  for (let i = 1; i <= N; i++) {
    const r = i + 1
    const price = 15 + ((i * 13) % 450)
    const qty = 1 + ((i * 3) % 20)
    const tot = totals[i - 1]
    const cat = cats[i % cats.length]
    const catIdx = sstMap.get(cat)
    const tierIdx = tot > 2500 ? highIdx : lowIdx
    const vlookup = i <= 1000
      ? `<c r="G${r}" s="2"><f>VLOOKUP(A${r},$A$2:$E$${N + 1},5,FALSE)</f><v>${tot}</v></c>`
      : ''
    const sumif = i <= 300
      ? `<c r="H${r}" s="2"><f>SUMIF($B$2:$B$${N + 1},B${r},$E$2:$E$${N + 1})</f><v>${catTotals.get(cat)}</v></c>`
      : ''
    rows1.push(
      `<row r="${r}">` +
      `<c r="A${r}" t="s"><v>${keys[i - 1]}</v></c>` +
      `<c r="B${r}" t="s"><v>${catIdx}</v></c>` +
      `<c r="C${r}"><v>${price}</v></c>` +
      `<c r="D${r}"><v>${qty}</v></c>` +
      `<c r="E${r}" s="2"><f>C${r}*D${r}*Summary!$B$1</f><v>${tot}</v></c>` +
      `<c r="F${r}" t="str"><f>IF(E${r}&gt;2500,"High","Low")</f><v>${tot > 2500 ? 'High' : 'Low'}</v></c>` +
      vlookup + sumif + '</row>'
    )
  }

  const sheet1Xml =
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
    '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">' +
    `<dimension ref="A1:H${N + 1}"/>` +
    `<sheetData>${rows1.join('')}</sheetData>` +
    `<conditionalFormatting sqref="E2:E${N + 1}"><cfRule type="cellIs" dxfId="0" priority="1" operator="greaterThan"><formula>2500</formula></cfRule></conditionalFormatting>` +
    `<conditionalFormatting sqref="C2:C${N + 1}"><cfRule type="colorScale" priority="2"><colorScale><cfvo type="min"/><cfvo type="max"/><color rgb="FFFEE2E2"/><color rgb="FFDCFCE7"/></colorScale></cfRule></conditionalFormatting>` +
    '</worksheet>'

  const grandTotal = totals.reduce((a, b) => a + b, 0)
  const labelMult = intern('Multiplier')
  const labelSum = intern('GrandTotal')
  const labelAvg = intern('AvgOrder')
  const labelHigh = intern('HighCount')
  const sheet2Xml =
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
    '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">' +
    '<dimension ref="A1:B6"/>' +
    '<sheetData>' +
    `<row r="1"><c r="A1" s="1" t="s"><v>${labelMult}</v></c><c r="B1"><v>1</v></c></row>` +
    `<row r="2"><c r="A2" s="1" t="s"><v>${labelSum}</v></c><c r="B2" s="2"><f>SUM(Data!E2:E${N + 1})</f><v>${grandTotal}</v></c></row>` +
    `<row r="3"><c r="A3" s="1" t="s"><v>${labelAvg}</v></c><c r="B3" s="2"><f>AVERAGE(Data!E2:E${N + 1})</f><v>${(grandTotal / N).toFixed(4)}</v></c></row>` +
    `<row r="4"><c r="A4" s="1" t="s"><v>${labelHigh}</v></c><c r="B4"><f>COUNTIF(Data!F2:F${N + 1},"High")</f><v>${totals.filter((x) => x > 2500).length}</v></c></row>` +
    '</sheetData></worksheet>'

  const buf = await buildXlsxBase({
    sheets: [
      { name: 'Summary', xml: sheet2Xml },
      { name: 'Data', xml: sheet1Xml },
    ],
    sharedStrings: sst,
    stylesXml: DEFAULT_STYLES_XML,
  })
  writeFileSync(outPath, buf)
  return outPath
}

function makeCsv(filename, rows) {
  const outPath = join(outDir, filename)
  if (!force && existsSync(outPath)) return outPath
  const lines = ['Category,Region,Units,Revenue,Margin']
  const cats = ['Alpha', 'Beta', 'Gamma', 'Delta']
  const regs = ['North', 'South', 'East', 'West']
  for (let i = 0; i < rows; i++) {
    const units = 10 + ((i * 7) % 500)
    const rev = ((units * 19.75) + (i % 97)).toFixed(2)
    const margin = (0.12 + ((i % 25) * 0.01)).toFixed(4)
    lines.push(`${cats[i % 4]},${regs[i % 4]},${units},${rev},${margin}`)
  }
  writeFileSync(outPath, lines.join('\n') + '\n', 'utf8')
  return outPath
}

function makePdf(filename, pageCount) {
  const outPath = join(outDir, filename)
  if (!force && existsSync(outPath)) return outPath

  // Objects: 1=Catalog, 2=Pages, 3=Font, then per page i (0..pageCount-1): Page=(4+2*i), Content=(5+2*i)
  const parts = ['%PDF-1.4\n']
  let offset = parts[0].length
  const offsets = new Array(3 + pageCount * 2 + 1)

  const pushObj = (id, body) => {
    offsets[id] = offset
    const chunk = `${id} 0 obj\n${body}\nendobj\n`
    parts.push(chunk)
    offset += chunk.length
  }

  const kids = Array.from({ length: pageCount }, (_, i) => `${4 + i * 2} 0 R`).join(' ')
  pushObj(1, '<< /Type /Catalog /Pages 2 0 R >>')
  pushObj(2, `<< /Type /Pages /Kids [${kids}] /Count ${pageCount} >>`)
  pushObj(3, '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>')

  for (let i = 0; i < pageCount; i++) {
    const pageNum = i + 1
    const pageId = 4 + i * 2
    const contentId = pageId + 1
    const lines = []
    for (let l = 0; l < 15; l++) {
      const y = 720 - l * 38
      const extra = pageNum === 490 && l === 7 ? ' NeedlePage490Token' : ''
      const txt = `Page ${pageNum} Line ${l + 1}: Deterministic velocity benchmark text sample.${extra}`
      lines.push(`BT /F1 12 Tf 72 ${y} Td (${txt}) Tj ET`)
    }
    const stream = lines.join('\n')
    pushObj(pageId, `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents ${contentId} 0 R >>`)
    pushObj(contentId, `<< /Length ${stream.length} >>\nstream\n${stream}\nendstream`)
  }

  const xrefOffset = offset
  parts.push(`xref\n0 ${offsets.length}\n0000000000 65535 f \n`)
  for (let id = 1; id < offsets.length; id++) {
    parts.push(`${String(offsets[id]).padStart(10, '0')} 00000 n \n`)
  }
  parts.push(`trailer\n<< /Size ${offsets.length} /Root 1 0 R >>\nstartxref\n${xrefOffset}\n%%EOF\n`)
  writeFileSync(outPath, Buffer.from(parts.join(''), 'latin1'))
  return outPath
}

function makeMarkdown(filename) {
  const outPath = join(outDir, filename)
  if (!force && existsSync(outPath)) return outPath
  const sections = ['# Velocity Benchmark Markdown\n']
  for (let s = 1; s <= 40; s++) {
    sections.push(`\n## Section ${s}: Subsystem Analysis\n\nParagraph ${s}. Performance measurements rely on deterministic workloads, bounded caches, and minimal allocations.\n`)
    sections.push(`\n- Item ${s}.1: Verify rendering pipeline\n- Item ${s}.2: Track memory stabilization\n- Item ${s}.3: Keep typing latency low\n`)
    sections.push(`\n| Metric | Baseline | Target |\n| --- | ---: | ---: |\n| Open ms | ${100 + s} | ${60 + s} |\n| Memory MB | ${40 + s} | ${28 + s} |\n`)
    sections.push('\n```ts\nexport function computeMetric(values: number[]): number {\n  let total = 0\n  for (let i = 0; i < values.length; i++) total += values[i] * 1.05\n  return total / Math.max(1, values.length)\n}\n```\n')
  }
  const md = sections.join('\n')
  writeFileSync(outPath, md, 'utf8')
  return outPath
}

function makeText(filename) {
  const outPath = join(outDir, filename)
  if (!force && existsSync(outPath)) return outPath
  const lines = []
  for (let i = 1; i <= 25000; i++) {
    lines.push(`Line ${String(i).padStart(5, '0')}: Plain text editor rope editing and dirty-check latency benchmark payload sample text.`)
  }
  writeFileSync(outPath, lines.join('\n') + '\n', 'utf8')
  return outPath
}

const generated = [
  await makeDocx('word-10p.docx', 10),
  await makeDocx('word-100p.docx', 100),
  await makeDocx('word-500p.docx', 500),
  await makePptx('slides-20.pptx', 20),
  await makePptx('slides-150.pptx', 150),
  await makePlainSheet('sheets-10k.xlsx', 2000),
  await makePlainSheet('sheets-50k.xlsx', 10000),
  await makeFormulaSheet('sheets-formulas.xlsx'),
  makeCsv('sheets-50k.csv', 10000),
  makePdf('pdf-100p.pdf', 100),
  makePdf('pdf-500p.pdf', 500),
  makeMarkdown('doc-100k.md'),
  makeText('doc-2mb.txt'),
]

for (const p of generated) {
  console.log('Fixture ready:', p)
}
