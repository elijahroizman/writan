// Crash recovery (UI-001): unsaved edits and untitled tabs survive a renderer reload, which skips
// the quit handler the way a crash does. localStorage and IndexedDB persist across the reload;
// writes stay in the fake native bridge's memory, which the reload clears.
// ENGINE=webkit node tests/stress/ui/session.mjs
import { join, resolve } from 'node:path'
import { launch, start, open, tabs, check, done, TMP } from './harness.mjs'
const path = resolve(import.meta.dirname, '../samples/S1-text-utf16le-bom.txt')
const { browser, page, logs } = await launch({ init: () => {
  localStorage.setItem('workspace.settings', JSON.stringify({ version: 2, autosave: { enabled: false, delay: 3, kinds: {} } }))
} })
const ws = (fn, arg) => page.evaluate(([f, a]) => new Function('ws', 'a', `return (${f})(ws, a)`)(window.__workspace.useWorkspace.getState(), a), [fn.toString(), arg])
const editor = page.locator('.view[data-active=true] .cm-content')
const dialog = page.locator('.dialog')
const answer = (label) => dialog.getByRole('button', { name: label, exact: true }).click()
const stored = () => page.evaluate(() => new Promise((resolve, reject) => {
  const req = indexedDB.open('writan-recovery', 1)
  req.onupgradeneeded = () => req.result.createObjectStore('docs', { keyPath: 'id' })
  req.onerror = () => reject(req.error)
  req.onsuccess = () => {
    const all = req.result.transaction('docs', 'readonly').objectStore('docs').getAll()
    all.onsuccess = () => {
      resolve(all.result.map((d) => ({ id: d.id, path: d.path, title: d.title, text: new TextDecoder(d.bytes[0] === 0xff ? 'utf-16le' : 'utf-8').decode(d.bytes) })))
      req.result.close()
    }
  }
}))
const waitStored = async (test, what) => {
  const until = Date.now() + 15000
  let docs = []
  while (Date.now() < until) {
    docs = await stored()
    if (test(docs)) return docs
    await page.waitForTimeout(200)
  }
  check(what, false, JSON.stringify(docs))
  return docs
}
const show = async (id) => {
  await ws((w, i) => w.activate(i), id)
  await page.waitForFunction((i) => !!window.__workspace.getEditor(i), id)
  await editor.waitFor()
  return editor.textContent()
}
const reload = async (count) => {
  await page.reload()
  await page.waitForFunction((n) => window.__workspace?.useWorkspace.getState().tabs.length === n, count, { timeout: 15000 })
  // Let the restore finish adopting the recovered documents under their new tab ids.
  await page.waitForTimeout(500)
  return tabs(page)
}
try {
  await start(page)
  await open(page, path)
  await editor.waitFor()
  await editor.click()
  await page.keyboard.press('Control+End')
  await page.keyboard.insertText('UNSAVED FILE MARKER')
  await ws((w) => w.newDocument('txt'))
  await editor.waitFor()
  await editor.click()
  await page.keyboard.insertText('UNTITLED RECOVERY MARKER')
  check('setup: both tabs have unsaved content', (await tabs(page)).filter((t) => t.dirty).length === 2)
  const docs = await waitStored((d) => d.length === 2 && d.some((x) => x.text.includes('UNSAVED FILE MARKER')) && d.some((x) => x.text.includes('UNTITLED RECOVERY MARKER')), 'both unsaved documents are kept for recovery')
  check('the kept copy of the file has its path', docs.some((d) => d.path === path && d.text.includes('UNSAVED FILE MARKER')))
  check('no autosave wrote either marker', await page.evaluate(() => window.__fake.writes.length === 0))

  let restored = await reload(2)
  const file = restored.find((t) => t.path === path)
  const untitled = restored.find((t) => t.path === null)
  check('reload reopens the file with its unsaved edits marked unsaved', !!file?.dirty, JSON.stringify(restored))
  check('reload brings back the untitled tab, unsaved', !!untitled?.dirty && untitled.title === 'Untitled Text', JSON.stringify(restored))
  check('tab order is kept', restored[0]?.path === path && restored[1]?.path === null)
  check('the untitled tab is active again', (await ws((w) => w.activeId)) === untitled?.id)
  check('untitled text is recovered', (await show(untitled.id)).includes('UNTITLED RECOVERY MARKER'))
  const text = await show(file.id)
  check('file edits are recovered', text.includes('UNSAVED FILE MARKER'))
  check('recovered UTF-16 text still decodes correctly', text.includes('Grüße') && text.includes('日本語') && !text.includes('�'))
  check('recovery wrote nothing to disk', await page.evaluate(() => window.__fake.writes.length === 0))
  await waitStored((d) => d.length === 2 && d.every((x) => restored.some((t) => t.id === x.id)) && d.some((x) => x.id === file.id && x.text.includes('UNSAVED FILE MARKER')), 'kept copies move to the new tab ids with their recovered text')

  // Back to what the editor loaded is still not what's on disk.
  await editor.click()
  await page.keyboard.press('Control+End')
  await page.keyboard.insertText('UNDONE')
  await waitStored((d) => d.some((x) => x.id === file.id && x.text.includes('UNDONE')), 'an edit after recovery is kept')
  await page.keyboard.press('Control+z')
  check('undoing past recovery keeps the tab unsaved', (await tabs(page)).find((t) => t.id === file.id)?.dirty === true)
  await waitStored((d) => d.some((x) => x.id === file.id && x.text.includes('UNSAVED FILE MARKER') && !x.text.includes('UNDONE')), 'the kept copy drops the undone edit')

  // Changed on disk since the edits started: saving asks instead of writing over it.
  await page.evaluate((p) => {
    const bytes = new Uint8Array([0xff, 0xfe, 0x4f, 0x00, 0x4b, 0x00])
    bytes.mtime = ++window.__fake.clock
    window.__fake.files.set(p, bytes)
  }, path)
  const conflict = ws((w, i) => w.save(i), file.id)
  await dialog.waitFor({ state: 'visible' })
  check('saving over a file changed since the crash asks first', /changed on disk/i.test(await dialog.textContent()))
  await answer('Cancel')
  check('Cancel keeps the recovered edits unsaved', (await conflict) === false && (await tabs(page)).find((t) => t.id === file.id)?.dirty === true)
  await page.evaluate((p) => window.__fake.files.delete(p), path)
  // A recovered file's own bytes now match: the original file, before the conflicting write.
  await ws((w, i) => w.save(i), file.id)
  const writes = await page.evaluate(() => window.__fake.writes.map((w) => w.path))
  check('saving writes the recovered edits', writes.includes(path), JSON.stringify(writes))
  check('a saved tab is clean', (await tabs(page)).find((t) => t.id === file.id)?.dirty === false)
  await waitStored((d) => d.length === 1 && d[0].id === untitled.id, 'saving drops the kept copy')

  // The fake disk forgets the save on reload; only the untitled tab comes back unsaved.
  restored = await reload(2)
  check('a saved file reopens clean', restored.find((t) => t.path === path)?.dirty === false, JSON.stringify(restored))
  const again = restored.find((t) => t.path === null)
  check('the untitled tab survives a second reload', !!again?.dirty && (await show(again.id)).includes('UNTITLED RECOVERY MARKER'))

  // Discarding it on purpose leaves nothing to recover.
  await waitStored((d) => d.length === 1 && d[0].id === again.id, 'kept copy follows the second reload')
  const closing = ws((w, i) => w.closeTab(i), again.id)
  await dialog.waitFor({ state: 'visible' })
  await answer('Don’t Save')
  await closing
  await waitStored((d) => d.length === 0, 'Don’t Save drops the kept copy')
  restored = await reload(1)
  check('a discarded tab stays discarded', restored.length === 1 && restored[0].path === path, JSON.stringify(restored))

  // Quitting, with every unsaved tab discarded, leaves nothing for the next launch.
  await show(restored[0].id)
  await editor.click()
  await page.keyboard.insertText('QUIT MARKER')
  await waitStored((d) => d.length === 1, 'an edit after reload is kept again')
  await page.evaluate(() => window.__fake.emit('tauri://close-requested', null))
  await dialog.waitFor({ state: 'visible' })
  await answer('Don’t Save')
  await page.waitForFunction(() => window.__fake.destroyed)
  await waitStored((d) => d.length === 0, 'quitting after Don’t Save drops the kept copy')
  await page.screenshot({ path: join(TMP, `session-${process.env.ENGINE ?? 'chromium'}.png`) })
} finally {
  await done(browser, logs)
}
