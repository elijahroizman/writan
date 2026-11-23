// An AI agent in a document nobody shared, then shared: the agent opens it to itself, the person
// sharing it later keeps the agent in, and everyone else sees the agent as that person's
// ("Ada’s Codex") while Ada sees "Codex". Also the popup that asks to let someone in.
//
// Needs the standalone relay running with MCP on, which the browser version shares through:
//   cd src-tauri && cargo run --example collab_relay -- --port 47812 --mcp agents-test-token --bridge agents-test-key
// RELAY=ws://127.0.0.1:port and MCP_URL point elsewhere. SHOTS=dir saves screenshots.
import { launch, start, check, done } from './harness.mjs'

const RELAY = process.env.RELAY ?? 'ws://127.0.0.1:47812'
const MCP = process.env.MCP_URL ?? 'http://127.0.0.1:47812/mcp/agents-test-token'
const shots = process.env.SHOTS

const relayInit = (relay) => localStorage.setItem('workspace.collabRelay', relay)
const a = await launch({
  tauri: false,
  init: () => {
    localStorage.setItem('workspace.agentsBridgeKey', 'agents-test-key')
    localStorage.setItem('workspace.ai', JSON.stringify({ agents: true, token: 'agents-test-token' }))
  },
})
await a.page.addInitScript(relayInit, RELAY)
const b = await launch({ tauri: false })
await b.page.addInitScript(relayInit, RELAY)
const logs = []
a.page.on('pageerror', (e) => logs.push(`[pageerror a] ${e.message}`))
b.page.on('pageerror', (e) => logs.push(`[pageerror b] ${e.message}`))

async function mcpClient(agent, clientName) {
  let session = null
  let id = 0
  const post = async (body) => {
    const res = await fetch(`${MCP}${agent ? `?agent=${agent}` : ''}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Accept: 'application/json, text/event-stream', ...(session ? { 'Mcp-Session-Id': session } : {}) },
      body: JSON.stringify(body),
    })
    if (res.headers.get('mcp-session-id')) session = res.headers.get('mcp-session-id')
    return res.status === 202 ? null : res.json()
  }
  await post({ jsonrpc: '2.0', id: ++id, method: 'initialize', params: { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: clientName, version: '1' } } })
  await post({ jsonrpc: '2.0', method: 'notifications/initialized' })
  return {
    call: async (name, args = {}) => {
      const r = await post({ jsonrpc: '2.0', id: ++id, method: 'tools/call', params: { name, arguments: args } })
      if (r.error) throw new Error(`${name}: ${JSON.stringify(r.error)}`)
      return { text: r.result.content.map((c) => c.text).join('\n'), error: !!r.result.isError }
    },
  }
}

const share = (page) => page.evaluate(() => window.__collab.useCollab.getState()[window.__workspace.useWorkspace.getState().activeId] ?? null)
const bodyText = (page) => page.locator('.docx-pages').innerText()

try {
  await start(a.page)
  await start(b.page)
  const codex = await mcpClient('codex', 'codex-mcp-client')
  check('with nothing open, list_documents says so', (await codex.call('list_documents')).text.includes('is open in Writan'))

  // A document nobody shared, with unsaved typing.
  await a.page.getByRole('button', { name: /New document/ }).first().click()
  await a.page.waitForFunction(() => window.__docxEditor)
  await a.page.locator('.docx-content-mount').click({ position: { x: 200, y: 120 } })
  await a.page.keyboard.type('Draft by Ada.')
  await a.page.waitForFunction(() => window.__workspace.useWorkspace.getState().tabs.find((t) => t.id === window.__workspace.useWorkspace.getState().activeId)?.dirty)

  const list = await codex.call('list_documents')
  const docId = /document: "([^"]+)"/.exec(list.text)?.[1]
  check('a document that isn’t shared is listed as such', !!docId && list.text.includes('not shared'), list.text)
  const read = await codex.call('read_document', { document: docId })
  check('the agent reads it, unsaved typing included', read.text.includes('Draft by Ada.'), read.text)

  const s1 = await share(a.page)
  check('it’s open to this computer’s agents only', s1?.role === 'host' && s1.private === true && s1.status === 'live' && !s1.code && !s1.invite, JSON.stringify(s1))
  check('the tab isn’t marked shared', (await a.page.locator('.tab-shared').count()) === 0)
  await a.page.waitForFunction(() => document.querySelector('.share-btn .share-avatar.agent img'))
  check('the title bar shows the agent’s logo', true)
  check('the share button still says Share', (await a.page.locator('.share-btn').getAttribute('aria-label')) === 'Share on your network')
  const dirty = await a.page.evaluate(() => window.__workspace.useWorkspace.getState().tabs.find((t) => t.id === window.__workspace.useWorkspace.getState().activeId))
  check('the document stays unsaved', dirty.dirty && dirty.recovered, JSON.stringify({ dirty: dirty.dirty, recovered: dirty.recovered }))

  const ins = await codex.call('insert_paragraphs', { document: docId, after: 1, text: 'Codex added this.' })
  check('the agent edits it', !ins.error, ins.text)
  await a.page.waitForFunction(() => document.querySelector('.docx-pages')?.textContent?.includes('Codex added this.'))
  check('its edit shows in the tab', true)
  const peersA = (await share(a.page)).peers
  check('here the agent is just “Codex”', peersA.some((p) => p.agent === 'codex' && p.name === 'Codex'), JSON.stringify(peersA))

  // The panel offers to share, and says the agent is in it.
  await a.page.locator('.share-btn').click()
  const panel = a.page.getByRole('dialog', { name: 'Share on your network' })
  await panel.waitFor()
  check('the panel says Codex is working in it', (await panel.innerText()).includes('Codex is working in it from this computer.'), await panel.innerText())
  if (shots) await a.page.screenshot({ path: `${shots}/unshared-panel.png` })
  await panel.getByLabel('Your name').fill('Ada')
  await panel.getByRole('button', { name: /^(Save and )?Share$/ }).click()
  const invite = a.page.getByLabel('Invite link')
  await invite.waitFor()
  const link = await invite.inputValue()
  await a.page.keyboard.press('Escape')
  const s2 = await share(a.page)
  check('sharing it gives it a link', s2.private === false && !!s2.invite, JSON.stringify(s2))

  // Joined from another browser: the agent shows as Ada's.
  await b.page.getByRole('tab', { name: 'Home' }).click()
  await b.page.getByRole('button', { name: 'Join a shared document…' }).click()
  const dialog = b.page.getByRole('dialog', { name: 'Join a shared document' })
  await dialog.getByLabel('Code or link').fill(link)
  await dialog.getByLabel('Your name').fill('Bo')
  await dialog.getByRole('button', { name: 'Join' }).click()
  await dialog.waitFor({ state: 'detached' })
  await b.page.waitForFunction(() => document.querySelector('.docx-pages')?.textContent?.includes('Codex added this.'))
  const peersB = (await share(b.page)).peers
  const agentB = peersB.find((p) => p.agent === 'codex')
  check('someone else sees it as Ada’s Codex', agentB?.name === 'Ada’s Codex' && agentB.owner === 'Ada', JSON.stringify(peersB))
  check('the agent stayed in when Ada shared', (await share(a.page)).peers.some((p) => p.agent === 'codex' && p.name === 'Codex'))

  // Bo's own agent, on Bo's computer, is Bo's to everyone else and just “Claude” to Bo.
  // (Both pages answer the same MCP bridge key here, so this one goes through page A again; what
  // matters is that whoever's app it joined through owns it.)
  const ed = await codex.call('insert_paragraphs', { document: docId, after: 2, text: 'And this, once shared.' })
  check('the agent goes on editing once shared', !ed.error, ed.text)
  await b.page.waitForFunction(() => document.querySelector('.docx-pages')?.textContent?.includes('And this, once shared.'))
  await b.page.waitForFunction(() => [...document.querySelectorAll('.docx-overlay .peer-tag')].some((t) => t.textContent.includes('Ada’s Codex')))
  check('Bo sees its caret named Ada’s Codex', true)
  if (shots) await b.page.screenshot({ path: `${shots}/guest-sees-owner.png` })
  const [ta, tb] = [await bodyText(a.page), await bodyText(b.page)]
  check('both copies agree', ta === tb, JSON.stringify([ta, tb]))

  // Leaving a shared document doesn't stop the share.
  await codex.call('leave_document', { document: docId })
  await new Promise((r) => setTimeout(r, 300))
  check('the share goes on after the agent leaves', (await share(a.page))?.status === 'live')

  // A second, unshared document: once the agent leaves, it's back to an ordinary tab.
  await a.page.getByRole('tab', { name: 'Home' }).click()
  await a.page.getByRole('button', { name: /New workbook/ }).first().click()
  await a.page.waitForFunction(() => window.__sheets)
  const books = await codex.call('list_documents')
  const book = /\.xlsx {2}document: "([^"]+)"/.exec(books.text)?.[1]
  const wrote = await codex.call('write_cells', { document: book, cells: { A1: '20', A2: '22', A3: '=A1+A2' } })
  check('the agent writes cells in an unshared workbook', !wrote.error, wrote.text)
  await a.page.waitForFunction(() => JSON.parse(window.__sheets.doc.cell(0, 2, 0)).display === '42')
  await codex.call('leave_document', { document: book })
  await a.page.waitForFunction(() => !window.__collab.useCollab.getState()[window.__workspace.useWorkspace.getState().activeId])
  check('when the agent leaves, the workbook isn’t open to anyone any more', true)
  check('what the agent wrote stays', await a.page.evaluate(() => JSON.parse(window.__sheets.doc.cell(0, 2, 0)).display === '42'))

  // A guest's own agent: page C joins Ada's document as Cy, and from then on answers the agent (the
  // newest page to connect takes the bridge). Everyone else sees it as Cy's; Cy sees “Codex”.
  const c = await launch({ tauri: false, init: () => {
    localStorage.setItem('workspace.agentsBridgeKey', 'agents-test-key')
    localStorage.setItem('workspace.ai', JSON.stringify({ agents: true, token: 'agents-test-token' }))
  } })
  await c.page.addInitScript(relayInit, RELAY)
  c.page.on('pageerror', (e) => logs.push(`[pageerror c] ${e.message}`))
  try {
    await start(c.page)
    await c.page.getByRole('button', { name: 'Join a shared document…' }).click()
    const joinC = c.page.getByRole('dialog', { name: 'Join a shared document' })
    await joinC.getByLabel('Code or link').fill(link)
    await joinC.getByLabel('Your name').fill('Cy')
    await joinC.getByRole('button', { name: 'Join' }).click()
    await joinC.waitFor({ state: 'detached' })
    await c.page.waitForFunction(() => document.querySelector('.docx-pages')?.textContent?.includes('Codex added this.'))
    // Let the bridge settle on C.
    await new Promise((r) => setTimeout(r, 500))
    const cDocs = await codex.call('list_documents')
    const cDoc = /document: "([^"]+)"/.exec(cDocs.text)?.[1]
    check('on Cy’s computer the document is listed as joined', cDocs.text.includes('joined from here'), cDocs.text)
    const cRead = await codex.call('read_document', { document: cDoc })
    check('Cy’s agent reads it', cRead.text.includes('Codex added this.'), cRead.text)
    await a.page.waitForFunction(() => window.__collab.useCollab.getState()[window.__workspace.useWorkspace.getState().tabs.find((t) => t.kind === 'docx').id]?.peers.some((p) => p.name === 'Cy’s Codex'))
    check('Ada sees Cy’s agent as Cy’s Codex', true)
    const peersC = (await share(c.page)).peers
    check('Cy sees their own agent as just Codex', peersC.some((p) => p.agent === 'codex' && p.name === 'Codex'), JSON.stringify(peersC))
    // Cy leaves, and their agent goes with them.
    await c.page.locator('.share-btn').click()
    await c.page.getByRole('button', { name: 'Leave' }).click()
    await a.page.waitForFunction(() => !window.__collab.useCollab.getState()[window.__workspace.useWorkspace.getState().tabs.find((t) => t.kind === 'docx').id]?.peers.some((p) => p.agent === 'codex'))
    check('when Cy leaves, Cy’s agent leaves too', true)
  } finally {
    a.logs.push(...c.logs)
    await c.browser.close()
  }

  // Someone asking to join gets a popup with their name and the document, until it's answered.
  await a.page.evaluate(() => {
    const ws = window.__workspace.useWorkspace.getState()
    const doc = ws.tabs.find((t) => t.kind === 'docx')
    const c = window.__collab.useCollab
    const now = c.getState()[doc.id]
    c.setState({
      [doc.id]: {
        ...now,
        waiting: [
          { request: 'join-1', session: 's', name: 'Yoav', waiting: true },
          { request: 'join-2', session: 's', name: 'Yoav’s Claude', agent: 'claude', waiting: true },
        ],
      },
    })
  })
  const popups = a.page.locator('.join-request')
  await popups.first().waitFor()
  check('each request gets a popup', (await popups.count()) === 2)
  check('a person’s says who wants to join', (await popups.nth(0).innerText()).includes('Yoav wants to join'))
  check('an agent’s says whose it is, with its logo', (await popups.nth(1).innerText()).includes('Yoav’s Claude wants to join') && (await popups.nth(1).locator('.share-avatar.agent img').count()) === 1)
  if (shots) await a.page.screenshot({ path: `${shots}/join-requests.png` })
  await popups.nth(1).getByRole('button', { name: 'Let in' }).click()
  await popups.nth(0).getByRole('button', { name: 'Deny' }).click()
  await a.page.waitForFunction(() => !document.querySelector('.join-request'))
  check('answering takes the popup away', true)
} catch (err) {
  check('the run finished', false, String(err?.stack ?? err))
}

a.logs.push(...logs)
await b.browser.close()
await done(a.browser, a.logs)
