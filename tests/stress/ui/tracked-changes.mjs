// Tracked changes can be accepted and rejected from the Word inspector, and undone.
// node tests/stress/ui/tracked-changes.mjs
import { join, resolve } from 'node:path'
import JSZip from 'jszip'
import { launch, start, open, tabs, check, done, TMP } from './harness.mjs'

const file = resolve(import.meta.dirname, '../samples/tracked-changes.docx')
const { browser, page, logs } = await launch()
await start(page)
await open(page, file)

const section = page.locator('.section', { has: page.locator('.revision-list') })
const accept = page.getByRole('button', { name: 'Accept all' })
const reject = page.getByRole('button', { name: 'Reject all' })
const body = async () => {
  const bytes = await page.evaluate(async () => {
    const s = window.__workspace.useWorkspace.getState()
    return [...(await window.__workspace.getEditor(s.activeId).getBytes())]
  })
  return (await JSZip.loadAsync(Uint8Array.from(bytes))).file('word/document.xml').async('string')
}
const items = () => page.locator('.revision').allTextContents()
const marks = (xml) => /<w:(ins|del|rPrChange)\b/.test(xml)

await section.waitFor()
check('the inspector lists every tracked change', (await items()).length === 3, JSON.stringify(await items()))
check('each change names its kind and author', (await items()).join('|') === 'InsertedAnnquick|DeletedBobslow|FormattedAnn', JSON.stringify(await items()))
check('the section title counts them', /Tracked changes · 3/.test(await section.textContent()))

await accept.click()
await section.waitFor({ state: 'detached' })
let xml = await body()
check('accept all keeps the inserted text', xml.includes('quick') && !xml.includes('slow'))
check('accept all keeps the new formatting', xml.includes('<w:b/>'))
check('accept all leaves no revision marks', !marks(xml), xml.slice(0, 400))
check('accepting marks the document unsaved', (await tabs(page)).some((t) => t.dirty))

await page.keyboard.press(process.platform === 'darwin' ? 'Meta+z' : 'Control+z')
await section.waitFor()
check('undo brings every change back', (await items()).length === 3)
check('undo restores the marks on save', marks(await body()))

await reject.click()
await section.waitFor({ state: 'detached' })
xml = await body()
check('reject all puts the deleted text back', xml.includes('slow') && !xml.includes('quick'))
check('reject all drops the new formatting', !xml.includes('<w:b/>'))
check('reject all leaves no revision marks', !marks(xml))
await page.screenshot({ path: join(TMP, 'tracked-changes.png') })

await done(browser, logs)
