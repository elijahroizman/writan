// Version history: scrubbing the timeline, the two comparisons, naming, restoring, deleting,
// saving a version, and the files it can't compare. The fake bridge keeps versions in memory the
// way src-tauri/src/history.rs does: write_file with x-history adds one, newest first.
import assert from 'node:assert/strict'
import { mkdirSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { launch, start, open, done } from './harness.mjs'

const dir = '/tmp/writan-stress/history'
mkdirSync(dir, { recursive: true })
const NOTES = `${dir}/Notes.txt`
const FRESH = `${dir}/Fresh.md`
const PDF = fileURLToPath(new URL('../../../scripts/tour/Lease.pdf', import.meta.url))
// Oldest first. The file on disk is the newest.
const drafts = ['Plan\n\nWe open in May.\n', 'Plan\n\nWe open in June.\n', 'Plan\n\nWe open in June.\n\nBring snacks.\n']
writeFileSync(NOTES, drafts.at(-1))
writeFileSync(FRESH, 'Nothing saved yet.\n')

const seed = { [NOTES]: drafts.map((text, i) => ({ text, label: i === 1 ? 'Moved to June' : undefined })), [PDF]: [{}] }

/** Runs in the page after the fake bridge: version history on top of it, seeded with `seed`. */
function fakeHistory(seed) {
  const enc = new TextEncoder()
  const store = new Map()
  const now = Date.now()
  for (const [path, list] of Object.entries(seed)) {
    store.set(
      path,
      list.map((v, i) => ({ version: { id: `${path}#${i}`, hash: String(i), time: now - (list.length - i) * 3_600_000, size: 1, source: i ? 'save' : 'original', label: v.label }, bytes: v.text ? enc.encode(v.text) : null })).reverse(),
    )
  }
  window.__history = { store, labels: [] }
  const inner = window.__TAURI_INTERNALS__.invoke
  window.__TAURI_INTERNALS__.invoke = async (cmd, args, opts) => {
    if (cmd === 'write_file' && opts?.headers?.['x-history']) {
      const path = decodeURIComponent(opts.headers['x-path'])
      const list = store.get(path) ?? []
      const label = opts.headers['x-history-label'] && decodeURIComponent(opts.headers['x-history-label'])
      list.unshift({ version: { id: `${path}#${Date.now()}-${list.length}`, hash: '', time: Date.now(), size: args.length, source: 'save', label }, bytes: new Uint8Array(args) })
      store.set(path, list)
    }
    const list = store.get(args?.path) ?? []
    const hit = () => list.find((v) => v.version.id === args.id)
    switch (cmd) {
      case 'history_list':
        return list.map((v) => ({ ...v.version }))
      case 'history_read':
        if (hit().bytes) return hit().bytes.slice().buffer
        return (await fetch('/__fs' + encodeURI(args.path))).arrayBuffer()
      case 'history_label':
        window.__history.labels.push(args.label)
        hit().version.label = args.label || undefined
        return null
      case 'history_delete':
        store.set(args.path, list.filter((v) => v.version.id !== args.id))
        return null
    }
    return inner(cmd, args, opts)
  }
}

const { browser, page, logs } = await launch({ init: `(${fakeHistory})(${JSON.stringify(seed)})` })

const dialog = page.getByRole('dialog', { name: /^Version history of/ })
const slider = dialog.getByRole('slider')
const title = dialog.locator('.vh-head h2')
const pageText = () => dialog.locator('.vh-page').innerText()
const versionCount = (path) => page.evaluate((path) => window.__history.store.get(path)?.length ?? 0, path)

async function openHistory(name) {
  await page.locator('.tab', { hasText: name }).first().click({ button: 'right' })
  await page.getByRole('menuitem', { name: 'Version history…' }).click()
  await dialog.waitFor()
}

try {
  await start(page)
  await open(page, NOTES)
  await page.locator('.tab', { hasText: 'Notes' }).waitFor()
  await openHistory('Notes')

  // It opens on the latest version, showing what its save changed.
  await page.waitForFunction(() => document.querySelector('.vh-p.add'))
  assert.equal(await slider.getAttribute('aria-valuenow'), '3')
  assert.equal(await slider.getAttribute('aria-valuemax'), '3')
  assert.match(await dialog.locator('.vh-sub').innerText(), /Latest/)
  assert.equal(await dialog.locator('.vh-p.add').first().innerText(), 'Bring snacks.')
  assert.equal(await dialog.getByRole('button', { name: 'Restore' }).isDisabled(), true)

  // ← steps back one version: the named one, which changed May to June.
  await page.keyboard.press('ArrowLeft')
  await page.waitForFunction(() => document.querySelector('.vh-page del')?.textContent === 'May')
  assert.equal(await slider.getAttribute('aria-valuenow'), '2')
  assert.equal(await title.innerText(), 'Moved to June')
  assert.equal(await dialog.locator('.vh-page ins').innerText(), 'June')

  // Clicking the first dot jumps to the oldest version, shown as it was.
  await dialog.locator('.vh-tick').first().click()
  await page.waitForFunction(() => document.querySelector('[role=slider]')?.getAttribute('aria-valuenow') === '1')
  await page.waitForFunction(() => !document.querySelector('.vh-page ins, .vh-page del') && /May/.test(document.querySelector('.vh-page')?.textContent ?? ''))
  assert.match(await dialog.locator('.vh-sub').innerText(), /Before Writan first saved it · Oldest kept/)

  // Dragging along the track scrubs to the newest.
  const box = await slider.boundingBox()
  await page.mouse.move(box.x + 4, box.y + 10)
  await page.mouse.down()
  await page.mouse.move(box.x + box.width / 2, box.y + 10, { steps: 5 })
  assert.equal(await slider.getAttribute('aria-valuenow'), '2')
  await page.mouse.move(box.x + box.width - 2, box.y + 10, { steps: 5 })
  await page.mouse.up()
  assert.equal(await slider.getAttribute('aria-valuenow'), '3')

  // Compare with now, from the oldest: restoring it would undo June and the snacks.
  await page.keyboard.press('Home')
  await dialog.getByRole('radio', { name: 'What restoring this version would undo' }).click()
  await page.waitForFunction(() => document.querySelector('.vh-page ins')?.textContent === 'June')
  assert.match(await pageText(), /Bring snacks\./)
  assert.match(await dialog.locator('.vh-stats').innerText(), /1 edited.*added/s)

  // Naming: click the title, type, Enter.
  await title.click()
  const field = dialog.getByPlaceholder(/Name this version/)
  await field.fill('First draft')
  await field.press('Enter')
  await page.waitForFunction(() => document.querySelector('.vh-head h2')?.textContent === 'First draft')
  assert.deepEqual(await page.evaluate(() => window.__history.labels), ['First draft'])

  // Restore asks first. While it asks, the arrow keys don't move the timeline.
  await dialog.getByRole('button', { name: 'Restore' }).click()
  const ask = page.getByRole('dialog', { name: /Restore this version/ })
  await ask.waitFor()
  await page.keyboard.press('ArrowRight')
  assert.equal(await slider.getAttribute('aria-valuenow'), '1')
  await ask.getByRole('button', { name: 'Restore' }).click()
  // The restored copy is the newest version, and the timeline moves to it.
  await page.waitForFunction(() => document.querySelector('[role=slider]')?.getAttribute('aria-valuemax') === '4')
  await page.waitForFunction(() => document.querySelector('[role=slider]')?.getAttribute('aria-valuenow') === '4')
  assert.match(await dialog.locator('.vh-head h2').innerText(), /^Restored from .+ at .+$/u)
  const saved = await page.evaluate((p) => new TextDecoder().decode(window.__fake.files.get(p)), NOTES)
  assert.equal(saved, drafts[0])

  // Delete from the ⋯ menu.
  await dialog.getByRole('button', { name: 'More' }).click()
  await page.getByRole('menuitem', { name: 'Delete version' }).click()
  await page.getByRole('alertdialog', { name: 'Delete this version?' }).getByRole('button', { name: 'Delete' }).click()
  await page.waitForFunction(() => document.querySelector('[role=slider]')?.getAttribute('aria-valuemax') === '3')
  assert.equal(await versionCount(NOTES), 3)

  // Escape closes it.
  await page.keyboard.press('Escape')
  await dialog.waitFor({ state: 'detached' })

  // Unsaved changes: Save version saves them as a new version.
  await page.locator('.cm-content').first().click()
  await page.keyboard.press('Control+End')
  await page.keyboard.type('\nOne more line.')
  await page.waitForFunction(() => window.__workspace.useWorkspace.getState().tabs.find((t) => t.title.startsWith('Notes'))?.dirty)
  await openHistory('Notes')
  await dialog.getByRole('button', { name: 'Save version' }).click()
  await page.waitForFunction(() => document.querySelector('[role=slider]')?.getAttribute('aria-valuemax') === '4')
  await dialog.getByRole('button', { name: 'Save version' }).waitFor({ state: 'detached' })
  await page.waitForFunction(() => [...document.querySelectorAll('.vh-p.add')].some((p) => p.textContent.includes('One more line.')))
  await page.keyboard.press('Escape')

  // A file with no versions says so, and has no timeline.
  await open(page, FRESH)
  await page.locator('.tab', { hasText: 'Fresh' }).waitFor()
  await openHistory('Fresh')
  await dialog.getByText(/No versions yet/).waitFor()
  assert.equal(await slider.count(), 0)
  await page.keyboard.press('Escape')

  // A PDF can't be compared: a note in place of the page, and no comparison switch.
  await open(page, PDF)
  await page.locator('.tab', { hasText: 'Lease' }).waitFor()
  await openHistory('Lease')
  await dialog.getByText(/versions can’t be shown here/).waitFor()
  assert.equal(await dialog.getByRole('radiogroup').count(), 0)
  await page.keyboard.press('Escape')
} finally {
  await done(browser, logs)
}
