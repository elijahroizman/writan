// Dragging a text selection across the gaps between lines on a PDF page must grow it smoothly:
// with the pointer over empty space it used to jump to the end of the page and back.
// Run against a VITE_TEST_HOOKS build with WRITAN_URL set to its preview URL.
import { fileURLToPath } from 'node:url'
import { check, done, launch, open, start } from './harness.mjs'

const pdf = fileURLToPath(new URL('../../../scripts/tour/Lease.pdf', import.meta.url))

const { browser, page, logs } = await launch({ scale: 1, viewport: { width: 1400, height: 1100 } })
await start(page)
await open(page, pdf)
await page.waitForFunction(() => document.querySelectorAll('.text-layer span[data-run]').length > 20, null, { timeout: 30000 })
await page.waitForTimeout(500)

// The page's spans on screen, in document order, and the lines they sit on.
const spans = await page.evaluate(() => {
  const layer = document.querySelector('.text-layer')
  return [...layer.querySelectorAll('span[data-run]')].map((s) => {
    const r = s.getBoundingClientRect()
    return { x: r.left, y: r.top, r: r.right, b: r.bottom, text: s.textContent }
  })
})
const layerBox = await page.locator('.text-layer').first().boundingBox()
const first = spans[Math.floor(spans.length / 4)]
const longest = Math.max(...spans.map((s) => s.text.length))
const selected = () => page.evaluate(() => window.getSelection()?.toString() ?? '')

// Sweep down the page past the ends of lines and through the right margin. Over a gap, the
// selection should reach the text beside the pointer, not the end of the page.
await page.mouse.move(first.x + 1, first.y + (first.b - first.y) / 2)
await page.mouse.down()
let jumps = 0
let steps = 0
let last = 0
for (let y = first.b + 2; y < Math.min(layerBox.y + layerBox.height - 20, first.b + 400); y += 3) {
  await page.mouse.move(first.x + 40, y, { steps: 2 })
  const inside = (await selected()).length
  await page.mouse.move(layerBox.x + layerBox.width - 30, y, { steps: 2 })
  last = (await selected()).length
  steps++
  if (last - inside > longest + 1) jumps++
}
await page.mouse.up()
check('selection over the margin stays on the line beside the pointer', jumps === 0, `${jumps} of ${steps} rows`)
check('selection grew past the first line', last > first.text.length + 100, `${last} characters`)

// Copying the whole page keeps its words and lines.
const all = await page.evaluate(() => {
  const range = document.createRange()
  range.selectNodeContents(document.querySelector('.text-layer'))
  window.getSelection().removeAllRanges()
  window.getSelection().addRange(range)
  return window.getSelection().toString()
})
check('whole-page selection reads as lines of text', all.includes(spans[0].text + '\n' + spans[1].text) && !/\n\s*\n/.test(all.trim()), JSON.stringify(all.slice(0, 80)))
await done(browser, logs)
