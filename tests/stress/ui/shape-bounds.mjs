// Shapes, SmartArt and lines are SVGs sized to what they draw, not 1px boxes that draw through
// overflow: WebView2 without a GPU left every shape out of those.
// node tests/stress/ui/shape-bounds.mjs   (ENGINE=webkit for WebKit)
import { resolve } from 'node:path'
import { launch, start, open, check, done } from './harness.mjs'

const S = resolve(import.meta.dirname, '../samples')
const D = resolve(import.meta.dirname, '../../../crates/office-core/src/formats/test_data')
const files = [
  [`${S}/plain-shapes.pptx`, '.slide-surface', 6],
  [`${D}/poi-SmartArt.pptx`, '.slide-surface', 5],
  [`${D}/aspose-SmartArt.docx`, '.docx-page-content', 12],
  [`${D}/docx4j-smartart.docx`, '.docx-page-content', 1],
]

const { browser, page, logs } = await launch()
try {
  await start(page)
  for (const [file, surface, least] of files) {
    await open(page, file)
    const name = file.split('/').pop()
    const shown = await page.waitForFunction((sel) => document.querySelectorAll(`${sel} svg.pa`).length, surface, { timeout: 30000 }).then(() => true, () => false)
    check(`${name} paints shapes`, shown)
    if (!shown) continue
    const r = await page.evaluate((sel) => {
      const bad = []
      let count = 0
      for (const svg of document.querySelectorAll(`${sel} svg.pa`)) {
        if (!svg.getClientRects().length) continue
        count++
        const box = svg.getBoundingClientRect()
        const drawn = svg.firstElementChild.getBoundingClientRect()
        const inside = drawn.left >= box.left - 0.5 && drawn.top >= box.top - 0.5 && drawn.right <= box.right + 0.5 && drawn.bottom <= box.bottom + 0.5
        if (!inside || box.width * box.height <= 1) bad.push(`${svg.outerHTML.slice(0, 160)} svg ${box.width}x${box.height}`)
      }
      return { count, bad }
    }, surface)
    check(`${name} draws ${least}+ shapes`, r.count >= least, `${r.count}`)
    check(`${name} shapes sit inside their SVG boxes`, r.bad.length === 0, r.bad.slice(0, 3).join('\n'))
  }
} finally {
  await done(browser, logs)
}
