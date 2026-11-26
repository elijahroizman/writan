// Selecting PDF text by dragging follows the pointer to the nearest text: presses and drags in the
// gaps between lines, in the margin, beside a short line or in the space between paragraphs don't
// jump to unrelated lines, and the Highlight tool highlights just the line dragged over.
import assert from 'node:assert/strict'
import { writeFileSync } from 'node:fs'
import { launch, start, open, done } from './harness.mjs'

const LINES = [
  [700, 'Alpha bravo charlie delta echo foxtrot golf hotel'],
  [676, 'India juliet kilo lima mike november oscar papa'],
  [652, 'Short line.'],
  [628, 'Quebec romeo sierra tango uniform victor whiskey'],
  [560, 'Xray yankee zulu one two three four five six'],
]

function pdf() {
  const s = LINES.map(([y, text]) => `BT /F1 12 Tf 72 ${y} Td (${text}) Tj ET`).join('\n')
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>',
    `<< /Length ${s.length} >>\nstream\n${s}\nendstream`,
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
  ]
  let out = '%PDF-1.7\n'
  const offsets = objects.map((o, i) => {
    const at = out.length
    out += `${i + 1} 0 obj\n${o}\nendobj\n`
    return at
  })
  const xref = out.length
  out += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.map((o) => `${String(o).padStart(10, '0')} 00000 n \n`).join('')}`
  out += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`
  return Buffer.from(out, 'latin1')
}

const file = '/tmp/writan-stress/text-select.pdf'
const { browser, page, logs } = await launch({ viewport: { width: 1400, height: 1100 } })
try {
  writeFileSync(file, pdf())
  await start(page)
  await open(page, file)
  await page.waitForFunction((n) => document.querySelectorAll('.text-layer span[data-run]').length === n, LINES.length)
  // Each line's box on screen.
  const box = await page.evaluate(() =>
    [...document.querySelectorAll('.text-layer span[data-run]')].map((s) => {
      const r = s.getBoundingClientRect()
      return { left: r.left, right: r.right, top: r.top, bottom: r.bottom, mid: (r.top + r.bottom) / 2 }
    }),
  )
  const selected = () => page.evaluate(() => window.getSelection().toString())
  const drag = async (from, to) => {
    await page.evaluate(() => window.getSelection().removeAllRanges())
    await page.mouse.move(from.x, from.y)
    await page.mouse.down()
    await page.mouse.move(to.x, to.y, { steps: 12 })
    await page.mouse.up()
    return selected()
  }
  const across = (i, f) => box[i].left + (box[i].right - box[i].left) * f

  // In the gap between lines 1 and 2, nearer line 2, to the middle of line 2.
  let got = await drag({ x: across(1, 0.25), y: box[0].bottom + (box[1].top - box[0].bottom) * 0.7 }, { x: across(1, 0.75), y: box[1].mid })
  assert.ok(got.length > 5 && LINES[1][1].includes(got), `a press between lines selected ${JSON.stringify(got)}`)

  // From the left margin to past the end of line 4: exactly that line.
  got = await drag({ x: box[3].left - 30, y: box[3].mid }, { x: box[3].right + 40, y: box[3].mid })
  assert.equal(got, LINES[3][1], 'a drag from the margin across a whole line')

  // From beside a short line to the middle of the next line: none of the short line, the start of the next.
  got = await drag({ x: box[2].right + 60, y: box[2].mid }, { x: across(3, 0.5), y: box[3].mid })
  assert.ok(!got.includes('Short') && got.trim().length > 5 && LINES[3][1].startsWith(got.trim()), `a press beside a short line selected ${JSON.stringify(got)}`)

  // From the middle of line 4 into the space between paragraphs: the rest of line 4 at most.
  got = await drag({ x: across(3, 0.3), y: box[3].mid }, { x: across(3, 0.8), y: box[3].bottom + 12 })
  assert.ok(got.length > 5 && LINES[3][1].includes(got), `a drag into the space between paragraphs selected ${JSON.stringify(got)}`)

  // A double-click still picks a word.
  await page.mouse.dblclick(across(0, 0.15), box[0].mid)
  assert.equal((await selected()).trim(), 'bravo', 'double-click selects a word')

  // The Highlight tool highlights just the line dragged over and drops the selection. (Picking it
  // with text selected would highlight that text.)
  await page.evaluate(() => window.getSelection().removeAllRanges())
  await page.getByRole('button', { name: 'Highlight', exact: true }).click()
  await drag({ x: box[1].left - 20, y: box[1].mid }, { x: box[1].right + 20, y: box[1].mid + 3 })
  await page.waitForFunction(() => window.__workspace.useWorkspace.getState().tabs[0].dirty)
  assert.equal(await selected(), '', 'the selection is dropped after highlighting')
  const saved = '/tmp/writan-stress/text-select-saved.pdf'
  await page.evaluate(async (to) => {
    if (!(await window.__workspace.useWorkspace.getState().save(undefined, { to, quiet: true }))) throw new Error('not saved')
  }, saved)
  const bytes = Buffer.from(await page.evaluate((p) => btoa(String.fromCharCode(...window.__fake.files.get(p))), saved), 'base64').toString('latin1')
  const highlights = [...bytes.matchAll(/\/Subtype\s*\/Highlight\b/g)]
  assert.equal(highlights.length, 1, 'one highlight')
  const quads = bytes.match(/\/QuadPoints\s*\[([^\]]*)\]/)
  assert.ok(quads, 'the highlight has QuadPoints')
  const ys = quads[1].trim().split(/\s+/).map(Number).filter((_, i) => i % 2 === 1)
  // Line 2's baseline is at 676 with 12pt Helvetica: its letters are from about 673 to 685.
  assert.ok(Math.min(...ys) > 668 && Math.max(...ys) < 692, `the highlight covers line 2 only: ${ys}`)

  // With the Select tool, a plain click on the highlight still picks it.
  await page.getByRole('button', { name: 'Select', exact: true }).click()
  await page.mouse.click(across(1, 0.5), box[1].mid)
  await page.locator('.annot-selection').waitFor()

  // Zoomed far out, where a period is under a pixel wide, a drag past a line's end still takes all of it.
  const zoomOut = page.getByRole('button', { name: /zoom out/i }).first()
  for (let i = 0; i < 25; i++) await zoomOut.click()
  await page.waitForTimeout(300)
  const tiny = await page.evaluate(() => {
    const r = document.querySelectorAll('.text-layer span[data-run]')[2].getBoundingClientRect()
    return { left: r.left, right: r.right, mid: (r.top + r.bottom) / 2 }
  })
  got = await drag({ x: tiny.left - 5, y: tiny.mid }, { x: tiny.right + 5, y: tiny.mid })
  assert.equal(got, LINES[2][1], 'a whole line at the smallest zoom')
  console.log('PASS  PDF text selection follows the pointer, and highlights it')
} finally {
  await done(browser, logs)
}
