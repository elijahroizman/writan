// ⌘R (Ctrl+R) reloads the file from disk: at once when nothing would be lost, after asking when
// there are unsaved changes. Run against a VITE_TEST_HOOKS build; writes stay in the fake bridge.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { launch, start, open, check, done } from './harness.mjs'

const path = resolve(import.meta.dirname, '../samples/S2-text-utf8-bom.txt')
const { browser, page, logs } = await launch({ init: () => {
  localStorage.setItem('workspace.settings', JSON.stringify({ version: 2, autosave: { enabled: false, delay: 2, kinds: {} } }))
} })
const text = () => page.locator('.view[data-active=true] .cm-content').innerText()
const dirty = () => page.evaluate(() => {
  const w = window.__workspace.useWorkspace.getState()
  return w.tabs.find((t) => t.id === w.activeId)?.dirty
})
const dialog = page.locator('.dialog')

try {
  await start(page)
  await open(page, path)
  await page.locator('.view[data-active=true] .cm-content').waitFor({ state: 'visible' })
  const original = await text()

  await page.locator('.view[data-active=true] .cm-content').click()
  await page.keyboard.press('Control+End')
  await page.keyboard.insertText(' unsaved words')
  check('the edit is there and unsaved', (await text()).includes('unsaved words') && (await dirty()))

  await page.keyboard.press('Control+r')
  await dialog.waitFor({ state: 'visible' })
  check('with unsaved changes it asks first', /Reload .* from disk\? Your unsaved changes will be lost\./.test(await dialog.textContent()), await dialog.textContent())
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click()
  await dialog.waitFor({ state: 'detached' })
  check('Cancel keeps the changes', (await text()).includes('unsaved words') && (await dirty()))

  await page.keyboard.press('Control+r')
  await dialog.getByRole('button', { name: 'Reload', exact: true }).click()
  await page.waitForFunction(() => !document.querySelector('.view[data-active=true] .cm-content')?.textContent?.includes('unsaved words'))
  check('Reload puts back what is on disk', (await text()) === original && !(await dirty()))

  // Changed on disk by something else, with nothing unsaved here: it reloads without asking.
  await page.evaluate(([p, bytes]) => {
    const b = Uint8Array.from(bytes)
    b.mtime = ++window.__fake.clock
    window.__fake.files.set(p, b)
  }, [path, [...Buffer.concat([readFileSync(path), Buffer.from('\nAdded elsewhere.')])]])
  await page.locator('.view[data-active=true] .cm-content').click()
  await page.keyboard.press('Control+r')
  await page.waitForFunction(() => document.querySelector('.view[data-active=true] .cm-content')?.textContent?.includes('Added elsewhere.'))
  check('a clean tab reloads at once, with what changed on disk', (await dialog.count()) === 0 && (await text()).includes('Added elsewhere.'))
} finally {
  await done(browser, logs)
}
