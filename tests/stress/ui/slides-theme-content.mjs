import assert from 'node:assert/strict'
import { launch } from './harness.mjs'

const { browser, page, logs } = await launch()
const run = (command) => page.evaluate((cmd) => window.__slides.run(cmd), command)
const elements = () => page.evaluate(() => window.__slides.elements)
const pick = async (button, item) => {
  await page.getByRole('button', { name: button, exact: true }).click()
  await page.getByRole('menuitem', { name: item, exact: true }).click()
}
const edit = async (id, lines) => {
  await run({ cmd: 'beginEdit', id: Number(id), x: null, y: null })
  await page.locator('.sc-input').focus()
  for (const [index, line] of lines.entries()) {
    if (index) await page.keyboard.press('Enter')
    await page.keyboard.insertText(line)
  }
  await run({ cmd: 'endEdit' })
}

try {
  await page.goto(process.env.WRITAN_URL ?? 'http://localhost:1420/')
  await page.getByRole('button', { name: 'New presentation', exact: true }).first().click()
  await page.waitForFunction(() => window.__slides?.slides.length === 2)
  const deck = [
    { layout: 'Title Slide', text: [['Launch'], ['Overview']] },
    { layout: 'Title and Content', text: [['Agenda'], ['Review', 'Decide']] },
    { layout: 'Title Slide', text: [['Schedule'], ['Updates']] },
    { layout: 'Title and Content', text: [['Actions'], ['Prepare', 'Share']] },
  ]
  for (const [index, { layout, text }] of deck.entries()) {
    if (index >= 2) await pick('Slide', layout)
    else await run({ cmd: 'setActive', index })
    const frames = await elements()
    assert.equal(frames.length, text.length)
    for (const [i, lines] of text.entries()) await edit(frames[i].id, lines)
  }

  await run({ cmd: 'setActive', index: 0 })
  await page.getByRole('button', { name: 'Text', exact: true }).click()
  const box = (await elements()).at(-1).id
  await edit(box, ['Annotation'])
  const image = await page.evaluate(() => {
    const deck = window.__slides
    const canvas = document.createElement('canvas')
    canvas.width = canvas.height = 20
    const ctx = canvas.getContext('2d')
    ctx.fillStyle = '#df8d36'
    ctx.fillRect(0, 0, 20, 20)
    const bytes = Uint8Array.from(atob(canvas.toDataURL().split(',')[1]), (c) => c.charCodeAt(0))
    const id = deck.doc.insertImage(bytes, 'image/png', 100, 60, 'User image')
    deck.refresh()
    return id
  })
  const geometry = async () => (await elements()).filter((e) => e.id === box || e.id === image)
    .map(({ id, x, y, width, height, media, text }) => ({ id, x, y, width, height, media, text }))
  const originals = await geometry()
  assert.equal(originals.length, 2)
  assert.ok(originals.some((e) => e.id === image && e.media))

  for (const design of ['Graphite', 'Default', 'Copper']) {
    await pick('Theme', design)
    const snapshot = await page.evaluate(() => {
      const deck = window.__slides
      return deck.slides.map((slide, index) => ({
        layout: deck.layouts.find((l) => l.path === slide.layout)?.name,
        content: JSON.parse(deck.doc.elements(index)).slice(0, 2),
      }))
    })
    for (const [index, { layout, text }] of deck.entries()) {
      const { content } = snapshot[index]
      assert.deepEqual(content.map((e) => e.text), text.map((lines) => lines.join('\n')), `${design}: slide ${index + 1} text`)
      if (design !== 'Default') {
        assert.equal(snapshot[index].layout, layout)
        assert.ok(content.every((e) => e.placeholder), `${design}: matching placeholders on slide ${index + 1}`)
        assert.ok(content.every((e) => e.textStyle.fontFamily !== 'Helvetica Neue'), `${design}: inherited type on slide ${index + 1}`)
      }
    }
    assert.deepEqual(await geometry(), originals, `${design}: user text box and image geometry/content`)
    const ink = await page.locator('.sc-slide .slide-surface').getByText('Annotation', { exact: true }).evaluate((el) => getComputedStyle(el).color)
    if (design === 'Graphite') assert.equal(ink, 'rgb(255, 255, 255)')
    if (design === 'Copper') assert.notEqual(ink, 'rgb(255, 255, 255)')
  }
  console.log('ok edited starter and added layouts retain all text and inherit design styling across Graphite → Default → Copper')
  console.log('ok added text box and image retain content and geometry and text uses readable theme ink')
} finally {
  for (const l of logs) console.log(' ', l)
  await browser.close()
}
