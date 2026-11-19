import { launch, start, check, done } from './harness.mjs'

const { browser, page, logs } = await launch({ tauri: false })
try {
  await start(page)
  await page.evaluate(() => window.__workspace.useWorkspace.getState().newDocument('pptx'))
  const canvas = page.getByRole('textbox', { name: 'Slide', exact: true })
  await canvas.waitFor()
  await canvas.focus()
  const run = (command) => page.evaluate((cmd) => window.__slides.run(cmd), command)
  const elements = () => page.evaluate(() => window.__slides.elements)
  const nativeCopyCutEnabled = () => page.evaluate(() => ['copy', 'cut'].map((cmd) => document.queryCommandEnabled(cmd)))
  const clipboardKey = (key) => page.keyboard.press(`ControlOrMeta+${key}`)

  await run({ cmd: 'insertShape', preset: 'rect' })
  const shape = (await elements()).at(-1)
  await run({ cmd: 'selectFrames', ids: [Number(shape.id)] })
  const initialCount = (await elements()).length
  check('native copy and cut are enabled for a selected slide shape', (await nativeCopyCutEnabled()).every(Boolean))

  await clipboardKey('c')
  await clipboardKey('v')
  const copied = (await elements()).at(-1)
  check('copy and paste duplicates a shape', (await elements()).length === initialCount + 1 && copied.id !== shape.id && copied.type === 'shape')
  await page.keyboard.press('Shift+ArrowRight')
  const moved = (await elements()).find(({ id }) => id === copied.id)
  check('the shape about to be cut differs from the copied shape', moved?.x > copied.x)
  await clipboardKey('x')
  const remaining = await elements()
  check('cut removes only the moved shape', remaining.length === initialCount && remaining.some(({ id }) => id === shape.id) && !remaining.some(({ id }) => id === copied.id))
  check('native copy and cut are disabled without a selection', (await nativeCopyCutEnabled()).every((enabled) => !enabled))
  await clipboardKey('v')
  const restored = await elements()
  check('paste restores the cut shape at its moved position', restored.length === initialCount + 1 && restored.at(-1)?.x === moved?.x)

  await run({ cmd: 'insertTextBox' })
  const textBox = (await elements()).at(-1)
  await run({ cmd: 'beginEdit', id: Number(textBox.id), x: null, y: null })
  await run({ cmd: 'insertText', text: 'slide text' })
  await run({ cmd: 'selectAllText' })
  check('native copy and cut are enabled for selected text inside a frame', (await nativeCopyCutEnabled()).every(Boolean))
  await clipboardKey('x')
  const boxText = async () => (await elements()).find(({ id }) => id === textBox.id)?.text
  check('cut removes text without deleting its frame', await boxText() === '')
  check('native copy and cut are disabled at a collapsed slide caret', (await nativeCopyCutEnabled()).every((enabled) => !enabled))
  await clipboardKey('v')
  check('paste restores text inside the frame', await boxText() === 'slide text')

  const notes = page.getByRole('textbox', { name: 'Speaker notes' })
  await notes.fill('notes text')
  await clipboardKey('a')
  await clipboardKey('x')
  check('cut edits the focused speaker notes, not the slide', await notes.inputValue() === '' && await boxText() === 'slide text')
  await clipboardKey('v')
  check('paste goes to the focused speaker notes', await notes.inputValue() === 'notes text')
} finally {
  await done(browser, logs)
}
