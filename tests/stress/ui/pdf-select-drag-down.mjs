// Dragging a PDF text selection straight down the page grows it line by line: over the gaps
// between short lines (a table) it doesn't reach a longer line further down and come back, and a
// superscript poking out of its line doesn't pull the selection back to the line's start.
import assert from 'node:assert/strict'
import { writeFileSync } from 'node:fs'
import { launch, start, open, done } from './harness.mjs'

// [y, x, size, text]
const RUNS = [
  [700, 72, 12, 'Alpha bravo charlie delta echo foxtrot golf hotel india juliet'],
  [676, 72, 12, 'such that x'],
  [682, 134, 7, '2'],
  [676, 140, 12, 'is a residue kilo lima mike november oscar'],
  [640, 150, 12, '5 1,4'],
  [620, 150, 12, '6 1'],
  [600, 150, 12, '9 1,4,7'],
  [580, 150, 12, '13 1,3,4,9'],
  [550, 72, 12, 'Papa quebec romeo sierra tango uniform victor whiskey xray yankee'],
]

function pdf() {
  const s = RUNS.map(([y, x, size, text]) => `BT /F1 ${size} Tf ${x} ${y} Td (${text}) Tj ET`).join('\n')
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

const file = '/tmp/writan-stress/select-drag-down.pdf'
const { browser, page, logs } = await launch({
  viewport: { width: 1400, height: 1100 },
})
try {
  writeFileSync(file, pdf())
  await start(page)
  await open(page, file)
  await page.waitForFunction(() => document.querySelectorAll('.text-layer span[data-run]').length >= 8)
  const box = await page.evaluate(() =>
    [...document.querySelectorAll('.text-layer span[data-run]')].map((s) => {
      const r = s.getBoundingClientRect()
      return { left: r.left, right: r.right, top: r.top, bottom: r.bottom }
    }),
  )
  const layer = await page.locator('.text-layer').first().boundingBox()
  const length = () => page.evaluate(() => window.getSelection().toString().length)
  const top = box[0]
  const bottom = box[box.length - 1]
  for (const [where, x] of [
    ['the left margin', layer.x + 20],
    ['the middle of the page', top.left + (top.right - top.left) / 2],
    ['the right margin', layer.x + layer.width - 30],
  ]) {
    await page.evaluate(() => window.getSelection().removeAllRanges())
    await page.mouse.move(top.left + 5, (top.top + top.bottom) / 2)
    await page.mouse.down()
    let prev = 0
    for (let y = top.bottom; y < bottom.bottom + 10; y += 2) {
      await page.mouse.move(x, y)
      const n = await length()
      assert.ok(
        n >= prev,
        `dragging down ${where}, the selection shrank from ${prev} to ${n} characters ${Math.round(y - top.bottom)}px below the first line`,
      )
      prev = n
    }
    await page.mouse.up()
    assert.ok(prev > 100, `dragging down ${where} selected ${prev} characters`)
  }
  console.log('PASS  dragging a PDF selection down the page only ever grows it')
} finally {
  await done(browser, logs)
}
