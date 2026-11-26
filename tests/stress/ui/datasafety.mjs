// Run against a VITE_TEST_HOOKS build. All writes stay in the fake native bridge.
// ENGINE=webkit node tests/stress/ui/datasafety.mjs
import { readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { launch, start, open, tabs, check, done, TMP } from './harness.mjs'

const S = resolve(import.meta.dirname, '../samples')
const { browser, page, logs } = await launch({ init: () => {
  localStorage.setItem('workspace.settings', JSON.stringify({ version: 2, autosave: { enabled: false, delay: 2, kinds: {} } }))
} })
await start(page)
const ws = (fn, arg) => page.evaluate(([f, a]) => new Function('ws', 'a', `return (${f})(ws, a)`)(window.__workspace.useWorkspace.getState(), a), [fn.toString(), arg])
const activeTab = () => ws((w) => w.tabs.find((t) => t.id === w.activeId))
const written = async (path) => Buffer.from(await page.evaluate((p) => [...(window.__fake.files.get(p) ?? [])], path))
const save = () => ws((w) => w.save())
const dialog = page.locator('.dialog')
const answer = (label) => dialog.getByRole('button', { name: label, exact: true }).click()
const edit = async (text, where = 'End') => {
  await page.locator('.view[data-active=true] .cm-content').click()
  await page.keyboard.press(`Control+${where}`)
  await page.keyboard.insertText(text)
}
const ready = async (file) => {
  await open(page, join(S, file))
  await page.waitForFunction(() => {
    const w = window.__workspace
    return !!w.getEditor(w.useWorkspace.getState().activeId)
  })
  if (file.endsWith('.md')) await page.evaluate(() => {
    const w = window.__workspace
    w.getEditor(w.useWorkspace.getState().activeId).viewMode('write')
  })
  await page.locator('.view[data-active=true] .cm-content').waitFor({ state: 'visible' })
}
// A pending save/close is deliberately started before answering its app-owned dialog.
const expectDialog = async (job, pattern, label = 'Cancel') => {
  await dialog.waitFor({ state: 'visible' })
  const text = await dialog.textContent()
  check(`dialog: ${pattern}`, pattern.test(text), text)
  await answer(label)
  return await job
}
const external = (path, bytes, stamp) => page.evaluate(([p, b, s]) => {
  const bytes = Uint8Array.from(b)
  bytes.mtime = ++window.__fake.clock
  window.__fake.files.set(p, bytes)
  if (s) window.__fake.infoOverride.set(p, s)
}, [path, [...bytes], stamp])

try {
  for (const f of [
    'S1-text-utf16le-bom.txt', 'S1-text-utf16be-bom.txt',
    'S1-text-windows1252.txt', 'S1-markdown-latin1.md', 'S2-text-utf8-bom.txt',
    'S3-text-binary.txt', 'S3-text-mixed-eol.txt', 'S2-text-cr.txt',
    'S2-markdown-crlf.md', 'S2-markdown-utf16le.md', 'S2-text-double-bom.txt',
    'S3-empty.txt', 'S3-empty.md',
  ]) {
    const path = join(S, f)
    const original = readFileSync(path)
    await ready(f)
    check(`${f}: untouched save succeeds`, await save())
    check(`${f}: untouched save preserves every byte`, (await written(path)).equals(original))
    await edit('Z')
    check(`${f}: editing marks unsaved`, (await activeTab()).dirty)
    await save()
    const after = await written(path)
    const utf16 = original[0] === 0xff || original[0] === 0xfe
    const suffix = utf16 ? (original[0] === 0xff ? Buffer.from([90, 0]) : Buffer.from([0, 90])) : Buffer.from('Z')
    check(`${f}: editing preserves original bytes and encoding`, after.equals(Buffer.concat([original, suffix])), `${original.length} -> ${after.length} bytes`)
    await page.keyboard.press('Control+z')
    await save()
    check(`${f}: undo restores original bytes`, (await written(path)).equals(original))
    await ws((w) => w.closeTab())
  }

  for (const file of ['S2-markdown-malformed-utf8-bom.md', 'S2-text-truncated-utf16.txt', 'S2-text-unpaired-utf16le.txt', 'S2-text-unpaired-utf16be.txt']) {
    const filePath = join(S, file)
    const original = readFileSync(filePath)
    await ready(file)
    check(`${file}: unchanged save succeeds`, await save())
    check(`${file}: unchanged malformed bytes survive`, (await written(filePath)).equals(original))
    await edit('Z')
    const writesBefore = await page.evaluate(() => window.__fake.writes.length)
    check(`${file}: edited malformed save is refused`, await expectDialog(save(), /invalid UTF-(8|16) bytes/, 'OK') === false)
    check(`${file}: refusal preserves disk and dirty state`, (await written(filePath)).equals(original) && (await activeTab()).dirty && await page.evaluate(() => window.__fake.writes.length) === writesBefore)
    await page.keyboard.press('Control+z')
    check(`${file}: undo then save succeeds`, await save())
    check(`${file}: undo restores malformed bytes too`, (await written(filePath)).equals(original))
    await ws((w) => w.closeTab())
  }

  // Insert and delete within a mixed-ending document, rather than only appending.
  const path = join(S, 'S3-text-mixed-eol.txt')
  const original = readFileSync(path)
  await ready('S3-text-mixed-eol.txt')
  await edit('first\n', 'Home')
  await save()
  check('new line uses the existing style; other endings survive', (await written(path)).equals(Buffer.concat([Buffer.from('first\n'), original])))
  await page.keyboard.press('Control+z')
  await save()
  check('undo newline insertion preserves mixed endings', (await written(path)).equals(original))
  await edit('', 'Home')
  await page.keyboard.press('ArrowDown')
  await page.keyboard.press('Home')
  await page.keyboard.press('Backspace')
  await save()
  check('deleting one line break leaves other endings intact', (await written(path)).equals(Buffer.from('unixwindows\r\nold mac\rend\n')))
  await page.keyboard.press('Control+z')
  await save()
  check('undo newline deletion restores mixed endings', (await written(path)).equals(original))
  await ws((w) => w.closeTab())

  await ready('S1-text-windows1252.txt')
  await edit('日本語 😀')
  await save()
  const upgraded = new TextDecoder().decode(await written(join(S, 'S1-text-windows1252.txt')))
  check('new characters outside Windows-1252 survive as UTF-8', upgraded.includes('Grüße, naïve café') && upgraded.includes('日本語 😀') && !upgraded.includes('�'))
  await ws((w) => w.closeTab())

  await ready('S3-text-mixed-eol.txt')
  await edit('unsaved words')
  await page.evaluate(() => (window.__fake.failWrite = 'Read-only file system (os error 30)'))
  const saved = await expectDialog(save(), /Read-only/, 'OK')
  check('failed save keeps the tab unsaved', saved === false && (await activeTab()).dirty)
  const id = (await activeTab()).id
  check('Cancel keeps a changed tab', (await expectDialog(ws((w) => w.closeTab()), /save the changes/i)) === false && (await activeTab()).id === id)
  const closing = ws((w) => w.closeTab())
  await dialog.waitFor()
  await answer('Save')
  await expectDialog(closing, /Read-only/, 'OK')
  check('Save failing in the close prompt keeps the tab', (await tabs(page)).some((t) => t.id === id && t.dirty))
  await page.evaluate(() => window.__fake.emit('tauri://close-requested', null))
  await dialog.waitFor()
  check('closing the window with unsaved changes asks', /save/i.test(await dialog.textContent()))
  await answer('Cancel')
  check('Cancel keeps the window', !await page.evaluate(() => window.__fake.destroyed))
  await page.evaluate(() => (window.__fake.failWrite = null))
  await save()

  const count = (await tabs(page)).length
  await open(page, path)
  check('opening an open file focuses its existing tab', (await tabs(page)).length === count)
  await ws((w) => w.duplicateTab())
  await page.waitForFunction(() => {
    const w = window.__workspace
    return !!w.getEditor(w.useWorkspace.getState().activeId)
  })
  const [a, b] = (await tabs(page)).filter((t) => t.path === path)
  await ws((w, id) => w.activate(id), a.id)
  await edit(' from A')
  await ws((w, id) => w.save(id), a.id)
  const aBytes = await written(path)
  await ws((w, id) => w.activate(id), b.id)
  await edit(' from B')
  const cancelled = await expectDialog(ws((w, id) => w.save(id), b.id), /changed on disk/)
  check('duplicate conflict Cancel preserves disk and unsaved tab', cancelled === false && (await written(path)).equals(aBytes) && (await activeTab()).dirty)
  const quiet = await ws(async (w, id) => {
    try { await w.save(id, { quiet: true }); return 'saved' } catch (e) { return String(e) }
  }, b.id)
  check('quiet conflict rejects without a dialog or write', /changed/.test(quiet) && await dialog.count() === 0 && (await written(path)).equals(aBytes))
  await expectDialog(ws((w, id) => w.save(id), b.id), /changed on disk/, 'Save Anyway')
  check('explicit overwrite writes the chosen copy', (await written(path)).toString().includes('from B'))

  // A real content change with identical size AND timestamp must still be noticed.
  const bBytes = await written(path)
  const stamp = await page.evaluate((p) => ({ exists: true, isDir: false, size: window.__fake.files.get(p).length, modifiedMs: window.__fake.files.get(p).mtime }), path)
  const outside = Buffer.from(bBytes)
  outside[0] = outside[0] === 88 ? 89 : 88
  await external(path, outside, stamp)
  await edit('!')
  await expectDialog(save(), /changed on disk/)
  check('same-size same-timestamp external edits survive Cancel', (await written(path)).equals(outside))
  await page.evaluate(() => (window.__fake.failRead = 'Permission denied reading current contents'))
  await expectDialog(save(), /Permission denied/, 'OK')
  check('failed conflict read never writes', (await written(path)).equals(outside))
  await page.evaluate(() => (window.__fake.failRead = null))
  await page.evaluate((p) => window.__fake.infoOverride.set(p, { exists: false, isDir: false, size: 0, modifiedMs: 0 }), path)
  await expectDialog(save(), /changed on disk/)
  check('external deletion requires confirmation before recreation', (await written(path)).equals(outside))
  await page.evaluate((p) => window.__fake.infoOverride.delete(p), path)
  await expectDialog(save(), /changed on disk/, 'Save Anyway')

  // An external write while a slow export is running must be checked after export finishes.
  const exportPath = path
  const duringExport = Buffer.from('outside during export')
  await page.evaluate(() => {
    const w = window.__workspace
    const editor = w.getEditor(w.useWorkspace.getState().activeId)
    const original = editor.getBytes.bind(editor)
    editor.getBytes = async () => {
      window.__fake.exportStarted = true
      await new Promise((resolve) => { window.__fake.finishExport = resolve })
      editor.getBytes = original
      return original()
    }
  })
  const slowSave = save()
  await page.waitForFunction(() => window.__fake.exportStarted)
  await external(exportPath, duringExport)
  await page.evaluate(() => window.__fake.finishExport())
  await expectDialog(slowSave, /changed on disk/)
  check('external change during export survives Cancel', (await written(exportPath)).equals(duringExport))
  await expectDialog(save(), /changed on disk/, 'Save Anyway')

  // Dock Quit follows the same page confirmation even immediately after an edit.
  await edit(' before Dock Quit')
  await page.evaluate(() => window.__fake.emit('menu', 'quit'))
  await dialog.waitFor()
  check('Dock Quit asks about the current dirty editor', /save/i.test(await dialog.textContent()))
  await answer('Cancel')
  check('Cancel prevents native quit', !await page.evaluate(() => window.__fake.calls.includes('quit_app')))

  // Undo back to the last saved text while a newer snapshot is still being written.
  // markDirty(false) alone must not let the in-flight save mark that newer edit as saved.
  await edit(' delayed snapshot')
  await page.evaluate(() => (window.__fake.writeDelay = 200))
  const writesBefore = await page.evaluate(() => window.__fake.calls.filter((c) => c === 'write_file').length)
  const delayed = save()
  await page.waitForFunction((before) => window.__fake.calls.filter((c) => c === 'write_file').length > before, writesBefore)
  await page.keyboard.press('Control+z')
  await delayed
  check('undo during an in-flight save leaves the changed snapshot unsaved', (await activeTab()).dirty)
  await page.evaluate(() => (window.__fake.writeDelay = 0))
  await save()

  // Two concurrent copies cannot both check the old bytes before either writes.
  await ws((w, id) => w.activate(id), a.id)
  await ws((w, id) => w.revert(id), a.id)
  await page.waitForTimeout(100)
  await edit(' concurrent A')
  await ws((w, id) => w.activate(id), b.id)
  await edit(' concurrent B')
  await page.evaluate(() => (window.__fake.writeDelay = 200))
  const concurrent = ws(async (w, ids) => Promise.all(ids.map((id) => w.save(id))), [a.id, b.id])
  await expectDialog(concurrent, /changed on disk/)
  const concurrentBytes = (await written(path)).toString()
  check('concurrent saves warn before replacing the first write', concurrentBytes.includes('concurrent A') && !concurrentBytes.includes('concurrent B'))
  await page.evaluate(() => (window.__fake.writeDelay = 0))
  // Save As changes the destination before another queued save from the same tab starts.
  await ws((w) => w.newDocument('txt'))
  await page.locator('.view[data-active=true] .cm-content').waitFor()
  await edit('queued Save As')
  const queuedPath = join(TMP, 'queued-save-as.txt')
  await page.evaluate(() => (window.__fake.writeDelay = 200))
  const queuedBefore = await page.evaluate(() => window.__fake.writes.length)
  const callsBefore = await page.evaluate(() => window.__fake.calls.filter((c) => c === 'write_file').length)
  const saveAs = ws((w, path) => w.save(undefined, { to: path, quiet: true }), queuedPath)
  await page.waitForFunction((before) => window.__fake.calls.filter((c) => c === 'write_file').length > before, callsBefore)
  await edit(' plus later edit')
  const queued = save()
  check('queued Save As and subsequent Save both succeed', (await saveAs) && (await queued))
  const destinations = await page.evaluate((before) => window.__fake.writes.slice(before).map((w) => w.path), queuedBefore)
  check('queued Save uses the new destination', destinations.length === 2 && destinations.every((p) => p === queuedPath))
  check('queued Save writes the later edit and leaves a clean tab', (await written(queuedPath)).toString().includes('plus later edit') && !(await activeTab()).dirty)
  await page.evaluate(() => (window.__fake.writeDelay = 0))
  await page.screenshot({ path: join(TMP, `datasafety-${process.env.ENGINE ?? 'chromium'}.png`) })
} finally {
  await done(browser, logs)
}
