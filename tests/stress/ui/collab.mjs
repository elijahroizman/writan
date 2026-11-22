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

// The pages' text, without the name tags over them.
const bodyText = (page) => page.locator('.docx-pages').innerText()

async function shareActive(page, name) {
  await page.getByRole('button', { name: 'Share on your network' }).click()
  const panel = page.getByRole('dialog', { name: 'Share on your network' })
  await panel.getByLabel('Your name').fill(name)
  // A document that isn't saved is saved first: the button says so.
  await panel.getByRole('button', { name: /^(Save and )?Share$/ }).click()
  const invite = page.getByLabel('Invite link')
  await invite.waitFor()
  await page.waitForFunction(() => document.querySelector('.share-panel h2')?.textContent === 'Shared on your network')
  return invite.inputValue()
}

async function joinFrom(page, invite, name) {
  await page.getByRole('tab', { name: 'Home' }).click()
  await page.getByRole('button', { name: 'Join a shared document…' }).click()
  const dialog = page.getByRole('dialog', { name: 'Join a shared document' })
  await dialog.getByLabel('Code or link').fill(invite)
  await dialog.getByLabel('Your name').fill(name)
  await dialog.getByRole('button', { name: 'Join' }).click()
  await dialog.waitFor({ state: 'detached' })
}

const closePanel = (page) => page.keyboard.press('Escape')

/** The name tags of everyone else's carets and boxes a page shows. */
const peerTags = (page, scope) => page.locator(`${scope} .peer-tag`).allInnerTexts()
const share = (page) => page.evaluate(() => {
  const s = window.__collab.useCollab.getState()
  return s[window.__workspace.useWorkspace.getState().activeId]
})

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
  await b.page.waitForFunction(() => document.querySelector('.docx-pages')?.textContent?.includes('Second line.'))
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
  await a.page.waitForFunction(() => document.querySelector('.docx-pages')?.textContent?.includes('Bo was here.'))
  await b.page.waitForFunction(() => document.querySelector('.docx-pages')?.textContent?.includes('Ada again.'))
  const [ta, tb] = [await bodyText(a.page), await bodyText(b.page)]
  check('both copies end with the same text', ta === tb, JSON.stringify([ta, tb]))
  check('each edit landed in its own paragraph', /Hello from Ada\. Ada again\.\s+Second line\. Bo was here\./.test(ta), JSON.stringify(ta))
  check('the host tab is unsaved, with everyone’s changes to save', await a.page.evaluate(() => window.__workspace.useWorkspace.getState().tabs.some((t) => t.kind === 'docx' && t.dirty)))
  check('sharing saved the document first', await a.page.evaluate(() => window.__workspace.useWorkspace.getState().tabs.some((t) => t.kind === 'docx' && t.path)), JSON.stringify(await a.page.evaluate(() => window.__workspace.useWorkspace.getState().tabs.map((t) => [t.kind, t.path, t.title]))))

  // Everyone sees the others' carets, in their color, with their name.
  await a.page.waitForFunction(() => document.querySelector('.docx-overlay .peer-caret[data-peer]'))
  await b.page.waitForFunction(() => document.querySelector('.docx-overlay .peer-caret[data-peer]'))
  check('the host sees the guest’s caret named', (await peerTags(a.page, '.docx-overlay')).join() === 'Bo', (await peerTags(a.page, '.docx-overlay')).join())
  check('the guest sees the host’s caret named', (await peerTags(b.page, '.docx-overlay')).join() === 'Ada', (await peerTags(b.page, '.docx-overlay')).join())
  const [sa, sb] = [await share(a.page), await share(b.page)]
  check('each person has a color of their own', sa.you.color !== sb.you.color, `${sa.you.color} ${sb.you.color}`)
  check('a person’s color is the same on every copy', sa.peers[0].color === sb.you.color && sb.peers[0].color === sa.you.color, JSON.stringify([sa, sb]))
  const caretColor = await a.page.locator('.docx-overlay .peer-caret').evaluate((el) => getComputedStyle(el).backgroundColor)
  check('the caret is drawn in that color', caretColor.replace(/\s/g, '') === `rgb(${[1, 3, 5].map((i) => parseInt(sb.you.color.slice(i, i + 2), 16)).join(',')})`, caretColor)
  check('the guest knows who keeps the file', sb.peers[0].host === true && sa.peers[0].host === false)
  // A guest's caret on the host's second line sits below the host's own on the first.
  const hostCaret = await a.page.locator('.docx-overlay .wp-caret').boundingBox()
  const guestCaret = await a.page.locator('.docx-overlay .peer-caret').boundingBox()
  check('the guest’s caret is where the guest is', !!hostCaret && !!guestCaret && guestCaret.y > hostCaret.y + 5, JSON.stringify([hostCaret, guestCaret]))
  // Selecting shows on the other copy too.
  await b.page.keyboard.press('Shift+Home')
  await a.page.waitForFunction(() => document.querySelector('.docx-overlay .peer-sel'))
  check('the guest’s selection shows on the host', (await a.page.locator('.docx-overlay .peer-sel').count()) > 0)
  await b.page.keyboard.press('End')
  await a.page.waitForFunction(() => !document.querySelector('.docx-overlay .peer-sel'))

  // The guest's tab isn't a file here: no unsaved dot, and the title bar says whose it is.
  check('the guest’s title bar names whose computer the file is on', (await b.page.locator('.autosave-chip').innerText()).includes('On Ada’s computer'), await b.page.locator('.autosave-chip').innerText())
  check('the guest’s tab has no unsaved dot', (await b.page.locator('.tab.active.dirty').count()) === 0)
  if (shots) {
    await a.page.getByRole('button', { name: 'Shared on your network' }).click()
    await a.page.screenshot({ path: `${shots}/word-host.png` })
    await closePanel(a.page)
    await b.page.screenshot({ path: `${shots}/word-guest.png` })
  }

  // Undo on the guest takes back only the guest's typing, on both copies.
  await b.page.keyboard.press('Control+z')
  await a.page.waitForFunction(() => !document.querySelector('.docx-pages')?.textContent?.includes('Bo was here.'))
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
  await a.page.getByRole('button', { name: 'New presentation', exact: true }).first().click()
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
  await a.page.waitForFunction(() => window.__workspace.useWorkspace.getState().tabs.some((t) => t.kind === 'pptx' && t.dirty))
  check('the guest’s change leaves the host with unsaved changes', true)
  // The guest is on the slide it added; the host's list puts the guest's dot there.
  await a.page.waitForFunction(() => document.querySelector('.slide-item .peer-dot'))
  const dotted = await a.page.locator('.slide-item').evaluateAll((els) => els.map((el) => !!el.querySelector('.peer-dot')))
  check('the host’s slide list shows which slide the guest is on', dotted.filter(Boolean).length === 1 && dotted[1], JSON.stringify(dotted))
  // On the same slide, the frame the guest picked is outlined on the host's, with the name.
  await b.page.locator('.slide-item').first().click()
  await b.page.evaluate(() => {
    window.__slides.run({ cmd: 'selectFrames', ids: [Number(window.__slides.elements[0].id)] })
  })
  await a.page.waitForFunction(() => document.querySelector('.sc-overlay .peer-box'))
  check('the frame the guest picked is outlined for the host', (await peerTags(a.page, '.sc-overlay')).join() === 'Bo', (await peerTags(a.page, '.sc-overlay')).join())
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
  await b.page.evaluate(() => window.__sheets.run({ cmd: 'select', sheet: 0, anchor: [2, 1], focus: [3, 2] }))
  // Its outline was on the cell the guest typed in; it moves to the new range.
  await a.page.waitForFunction(() => document.querySelector('.sg-body .peer-box')?.getBoundingClientRect().height > 30)
  check('the cells the guest selected are outlined for the host', (await peerTags(a.page, '.sg-body')).join() === 'Bo')
  const box = await a.page.locator('.sg-body .peer-box').boundingBox()
  const b3 = await a.page.evaluate(() => {
    const g = window.__sheets.geometryOf(0)
    return { w: g.x(3) - g.x(1), h: g.y(4) - g.y(2) }
  })
  check('the outline covers the guest’s range', !!box && Math.abs(box.width - b3.w) < 2 && Math.abs(box.height - b3.h) < 2, JSON.stringify([box, b3]))
  check('the sheet list shows the guest’s sheet', (await a.page.locator('nav[aria-label="Sheets"] .peer-dot').count()) === 1)

  // A guest that closes its tab right after an edit isn't asked to save, and the edit still
  // reaches the host.
  const closed = await b.page.evaluate(async () => {
    window.__sheets.run({ cmd: 'input', sheet: 0, row: 5, col: 0, text: 'last word' })
    const ws = window.__workspace.useWorkspace.getState()
    return ws.closeTab(ws.activeId)
  })
  check('the guest’s tab closes without asking', closed === true && (await b.page.locator('.dialog').count()) === 0)
  await a.page.waitForFunction(() => JSON.parse(window.__sheets.doc.cell(0, 5, 0)).display === 'last word')
  check('what the guest did just before closing reached the host', true)
  if (shots) await a.page.screenshot({ path: `${shots}/sheets-host.png` })
} catch (err) {
  check('the run finished', false, String(err?.stack ?? err))
}

a.logs.push(...logs)
await b.browser.close()
await done(a.browser, a.logs)
