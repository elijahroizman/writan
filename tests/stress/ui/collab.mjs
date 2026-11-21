// Two copies of the app editing one document through the relay: a Word document, a
// presentation and a workbook, each shared from one browser and joined from another.
//
// Needs the standalone relay running on the default port, which the browser version shares
// through:
//   cd src-tauri && cargo run --example collab_relay -- --port 47812
// SHOTS=dir saves screenshots.
import { launch, start, check, done } from './harness.mjs'

const shots = process.env.SHOTS
const a = await launch({ tauri: false })
const b = await launch({ tauri: false })
const logs = []
a.page.on('pageerror', (e) => logs.push(`[pageerror a] ${e.message}`))
b.page.on('pageerror', (e) => logs.push(`[pageerror b] ${e.message}`))

const bodyText = (page) => page.locator('.docx-content-mount').innerText()

async function shareActive(page, name) {
  await page.getByRole('button', { name: 'Share on your network' }).click()
  const panel = page.getByRole('dialog', { name: 'Share on your network' })
  await panel.getByLabel('Your name').fill(name)
  await panel.getByRole('button', { name: 'Share', exact: true }).click()
  const invite = page.getByLabel('Invite link')
  await invite.waitFor()
  await page.waitForFunction(() => document.querySelector('.share-panel h2')?.textContent === 'Shared on your network')
  return invite.inputValue()
}

async function joinFrom(page, invite, name) {
  await page.getByRole('tab', { name: 'Home' }).click()
  await page.getByRole('button', { name: 'Join a shared document…' }).click()
  const dialog = page.getByRole('dialog', { name: 'Join a shared document' })
  await dialog.getByLabel('Link').fill(invite)
  await dialog.getByLabel('Your name').fill(name)
  await dialog.getByRole('button', { name: 'Join' }).click()
  await dialog.waitFor({ state: 'detached' })
}

const closePanel = (page) => page.keyboard.press('Escape')

try {
  await start(a.page)
  await start(b.page)

  // ── Word
  await a.page.getByRole('button', { name: /New document/ }).first().click()
  await a.page.waitForFunction(() => window.__docxEditor)
  await a.page.locator('.docx-content-mount').click({ position: { x: 200, y: 120 } })
  await a.page.keyboard.type('Hello from Ada.')
  await a.page.keyboard.press('Enter')
  await a.page.keyboard.type('Second line.')
  const invite = await shareActive(a.page, 'Ada')
  check('sharing gives a ws:// invite with a token', /^ws:\/\/127\.0\.0\.1:47812\/v1\/[\w-]+\?token=[\w-]+$/.test(invite), invite)
  await closePanel(a.page)

  await joinFrom(b.page, invite, 'Bo')
  await b.page.waitForFunction(() => document.querySelector('.docx-content-mount')?.textContent?.includes('Second line.'))
  check('the guest opens what the host had typed', (await bodyText(b.page)).includes('Hello from Ada.'))
  await a.page.waitForFunction(() => document.querySelector('.share-btn .share-avatar'))
  check('the host sees the guest arrive', (await a.page.locator('.share-btn .share-avatar').count()) === 1)

  // The guest types at the end of the second paragraph; the host at the end of the first.
  await b.page.locator('.docx-content-mount').click({ position: { x: 200, y: 120 } })
  await b.page.keyboard.press('Control+End')
  await b.page.keyboard.type(' Bo was here.')
  await a.page.keyboard.press('Control+Home')
  await a.page.keyboard.press('End')
  await a.page.keyboard.type(' Ada again.')
  await a.page.waitForFunction(() => document.querySelector('.docx-content-mount')?.textContent?.includes('Bo was here.'))
  await b.page.waitForFunction(() => document.querySelector('.docx-content-mount')?.textContent?.includes('Ada again.'))
  const [ta, tb] = [await bodyText(a.page), await bodyText(b.page)]
  check('both copies end with the same text', ta === tb, JSON.stringify([ta, tb]))
  check('each edit landed in its own paragraph', /Hello from Ada\. Ada again\.\s+Second line\. Bo was here\./.test(ta), JSON.stringify(ta))
  check('the host tab is unsaved, with everyone’s changes to save', await a.page.evaluate(() => window.__workspace.useWorkspace.getState().tabs.some((t) => t.kind === 'docx' && t.dirty)))
  if (shots) {
    await a.page.getByRole('button', { name: 'Shared on your network' }).click()
    await a.page.screenshot({ path: `${shots}/word-host.png` })
    await closePanel(a.page)
    await b.page.screenshot({ path: `${shots}/word-guest.png` })
  }

  // Undo on the guest takes back only the guest's typing, on both copies.
  await b.page.keyboard.press('Control+z')
  await a.page.waitForFunction(() => !document.querySelector('.docx-content-mount')?.textContent?.includes('Bo was here.'))
  check('undo takes back the guest’s typing everywhere', (await bodyText(a.page)).includes('Ada again.') && (await bodyText(a.page)) === (await bodyText(b.page)))

  // The host stops; the guest keeps a copy.
  await a.page.getByRole('button', { name: 'Shared on your network' }).click()
  await a.page.getByRole('button', { name: 'Stop sharing' }).click()
  await b.page.waitForFunction(() => !document.querySelector('.tab-shared'))
  await b.page.getByRole('button', { name: 'Share on your network' }).click()
  const ended = await b.page.locator('.share-panel').innerText()
  check('the guest is told sharing stopped', /Sharing has stopped/.test(ended), ended)
  await closePanel(b.page)

  // ── PowerPoint
  await a.page.getByRole('tab', { name: 'Home' }).click()
  await a.page.getByRole('button', { name: 'New presentation' }).first().click()
  await a.page.getByRole('dialog', { name: 'New presentation' }).waitFor()
  await a.page.keyboard.press('Enter')
  await a.page.waitForFunction(() => window.__slides?.slides?.length >= 1)
  const deckInvite = await shareActive(a.page, 'Ada')
  await closePanel(a.page)
  await joinFrom(b.page, deckInvite, 'Bo')
  await b.page.waitForFunction(() => window.__slides?.slides?.length >= 1 && document.querySelector('.tab.active .tab-shared'))
  const slidesOf = (page) => page.evaluate(() => window.__slides.slides.length)
  check('the guest gets the deck', (await slidesOf(b.page)) === (await slidesOf(a.page)), `${await slidesOf(a.page)} vs ${await slidesOf(b.page)}`)
  // The guest adds a slide; the host sees it.
  const before = await slidesOf(a.page)
  await b.page.evaluate(() => window.__slides.run({ cmd: 'addSlide', after: 0, layout: null }))
  await a.page.waitForFunction((n) => window.__slides.slides.length === n + 1, before)
  check('a slide the guest adds shows up for the host', (await slidesOf(a.page)) === before + 1)
  if (shots) await a.page.screenshot({ path: `${shots}/slides-host.png` })

  // ── Excel
  await a.page.getByRole('tab', { name: 'Home' }).click()
  await a.page.getByRole('button', { name: /New workbook/ }).first().click()
  await a.page.waitForFunction(() => window.__sheets)
  await a.page.evaluate(() => window.__sheets.run({ cmd: 'input', sheet: 0, row: 0, col: 0, text: '2' }))
  const bookInvite = await shareActive(a.page, 'Ada')
  await closePanel(a.page)
  await joinFrom(b.page, bookInvite, 'Bo')
  await b.page.waitForFunction(() => window.__sheets && document.querySelector('.tab.active .tab-shared'))
  await b.page.evaluate(() => window.__sheets.run({ cmd: 'input', sheet: 0, row: 1, col: 0, text: '=A1*21' }))
  await a.page.waitForFunction(() => JSON.parse(window.__sheets.doc.cell(0, 1, 0)).display === '42')
  check('a formula the guest types computes on the host', true)
  if (shots) await a.page.screenshot({ path: `${shots}/sheets-host.png` })
} catch (err) {
  check('the run finished', false, String(err?.stack ?? err))
}

a.logs.push(...logs)
await b.browser.close()
await done(a.browser, a.logs)
