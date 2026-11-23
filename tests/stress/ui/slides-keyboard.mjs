// Turning slides with the keyboard in the editor, without presenting.
import { launch, start, check, done } from './harness.mjs'

const { browser, page, logs } = await launch({ tauri: false })
try {
  await start(page)
  await page.evaluate(() => window.__workspace.useWorkspace.getState().newDocument('pptx'))
  const canvas = page.getByRole('textbox', { name: 'Slide', exact: true })
  await canvas.waitFor()
  const run = (command) => page.evaluate((cmd) => window.__slides.run(cmd), command)
  const active = () => page.evaluate(() => window.__slides.state.active)
  const press = async (key) => {
    await page.keyboard.press(key)
    return active()
  }

  // A new deck has a title slide and a content slide.
  await page.waitForFunction(() => window.__slides?.slides?.length === 2)
  await run({ cmd: 'addSlide', after: 1, layout: null })
  await run({ cmd: 'setActive', index: 0 })
  await canvas.focus()
  check('the deck has three slides', (await page.evaluate(() => window.__slides.slides.length)) === 3)

  check('Right arrow goes to the next slide', (await press('ArrowRight')) === 1)
  check('Down arrow goes to the next slide', (await press('ArrowDown')) === 2)
  check('Right arrow stops at the last slide', (await press('ArrowRight')) === 2)
  check('Left arrow goes to the previous slide', (await press('ArrowLeft')) === 1)
  check('Up arrow goes to the previous slide', (await press('ArrowUp')) === 0)
  check('Left arrow stops at the first slide', (await press('ArrowLeft')) === 0)
  check('End goes to the last slide', (await press('End')) === 2)
  check('Home goes to the first slide', (await press('Home')) === 0)
  check('Page Down goes to the next slide', (await press('PageDown')) === 1)
  check('Page Up goes to the previous slide', (await press('PageUp')) === 0)
  check('the toolbar readout follows the keys', (await page.locator('.canvas-bar .readout').first().textContent())?.trim() === '1 / 3')

  // A picked object moves with the arrows instead.
  await run({ cmd: 'insertShape', preset: 'rect' })
  const shape = (await page.evaluate(() => window.__slides.elements)).at(-1)
  await run({ cmd: 'selectFrames', ids: [Number(shape.id)] })
  await canvas.focus()
  check('Right arrow with a shape picked stays on the slide', (await press('ArrowRight')) === 0)
  const moved = (await page.evaluate(() => window.__slides.elements)).find((x) => x.id === shape.id)
  check('Right arrow with a shape picked nudges it', moved.x > shape.x)
  check('Page Down with a shape picked still turns the slide', (await press('PageDown')) === 1)

  // Text being edited keeps the arrows for its caret.
  await run({ cmd: 'setActive', index: 0 })
  await run({ cmd: 'insertTextBox' })
  const box = (await page.evaluate(() => window.__slides.elements)).at(-1)
  await run({ cmd: 'beginEdit', id: Number(box.id), x: null, y: null })
  await canvas.focus()
  await page.keyboard.type('ab')
  check('Left arrow while editing text stays on the slide', (await press('ArrowLeft')) === 0)
  await page.keyboard.type('X')
  check('Left arrow while editing text moves the caret', (await page.evaluate((id) => window.__slides.elements.find((x) => x.id === id)?.text, box.id)) === 'aXb')
  await page.keyboard.press('Escape')
  await page.keyboard.press('Escape')
  check('after leaving the text, Right arrow turns the slide again', (await press('ArrowRight')) === 1)

  // The slide list takes Left and Right as well as Up and Down.
  await page.getByRole('listbox', { name: 'Slides' }).focus()
  check('Right arrow in the slide list goes to the next slide', (await press('ArrowRight')) === 2)
  check('Left arrow in the slide list goes to the previous slide', (await press('ArrowLeft')) === 1)
} finally {
  await done(browser, logs)
}
