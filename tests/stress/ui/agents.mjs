// An AI agent working in shared documents through Writan's MCP server, as Codex would: this
// script speaks MCP over HTTP to the standalone relay, and two browsers see what it does.
//
// Needs the standalone relay running with MCP on, which the browser version shares through:
//   cd src-tauri && cargo run --example collab_relay -- --port 47812 --mcp agents-test-token --bridge agents-test-key
// SHOTS=dir saves screenshots.
import { launch, start, check, done } from './harness.mjs'

const MCP = process.env.MCP_URL ?? 'http://127.0.0.1:47812/mcp/agents-test-token'
const shots = process.env.SHOTS

// Page A lets agents in, as Settings › AI would with the switch on.
const a = await launch({
  tauri: false,
  init: () => {
    localStorage.setItem('workspace.agentsBridgeKey', 'agents-test-key')
    localStorage.setItem('workspace.ai', JSON.stringify({ agents: true, token: 'agents-test-token' }))
  },
})
const b = await launch({ tauri: false })
const logs = []
a.page.on('pageerror', (e) => logs.push(`[pageerror a] ${e.message}`))
b.page.on('pageerror', (e) => logs.push(`[pageerror b] ${e.message}`))

/** An MCP client: initialize, then calls with the session it got. */
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
  const init = await post({ jsonrpc: '2.0', id: ++id, method: 'initialize', params: { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: clientName, version: '1' } } })
  await post({ jsonrpc: '2.0', method: 'notifications/initialized' })
  return {
    init,
    list: () => post({ jsonrpc: '2.0', id: ++id, method: 'tools/list' }),
    call: async (name, args = {}) => {
      const r = await post({ jsonrpc: '2.0', id: ++id, method: 'tools/call', params: { name, arguments: args } })
      if (r.error) throw new Error(`${name}: ${JSON.stringify(r.error)}`)
      return { text: r.result.content.map((c) => c.text).join('\n'), error: !!r.result.isError }
    },
  }
}

const bodyText = (page) => page.locator('.docx-pages').innerText()

async function shareActive(page, name) {
  await page.getByRole('button', { name: 'Share on your network' }).click()
  const panel = page.getByRole('dialog', { name: 'Share on your network' })
  await panel.getByLabel('Your name').fill(name)
  await panel.getByRole('button', { name: /^(Save and )?Share$/ }).click()
  const invite = page.getByLabel('Invite link')
  await invite.waitFor()
  const link = await invite.inputValue()
  await page.keyboard.press('Escape')
  return link
}

async function joinFrom(page, invite, name) {
  await page.getByRole('tab', { name: 'Home' }).click()
  await page.getByRole('button', { name: 'Join a shared document…' }).click()
  const dialog = page.getByRole('dialog', { name: 'Join a shared document' })
  await dialog.getByLabel('Code').fill(invite)
  await dialog.getByLabel('Your name').fill(name)
  await dialog.getByRole('button', { name: 'Join' }).click()
  await dialog.waitFor({ state: 'detached' })
}

try {
  await start(a.page)
  await start(b.page)

  // Nothing shared yet: the agent is told how to get going.
  const codex = await mcpClient('codex', 'codex-mcp-client')
  check('initialize names the server and the session', codex.init?.result?.serverInfo?.name === 'writan', JSON.stringify(codex.init))
  const tools = (await codex.list()).result.tools.map((t) => t.name)
  check('the tools are listed', ['list_documents', 'read_document', 'insert_paragraphs', 'write_cells', 'where_is_everyone'].every((t) => tools.includes(t)), tools.join())
  check('with nothing shared, list_documents says what to do', (await codex.call('list_documents')).text.includes('Nothing is shared'))

  // ── Word
  await a.page.getByRole('button', { name: /New document/ }).first().click()
  await a.page.waitForFunction(() => window.__docxEditor)
  await a.page.locator('.docx-content-mount').click({ position: { x: 200, y: 120 } })
  await a.page.keyboard.type('Meeting notes')
  await a.page.keyboard.press('Enter')
  await a.page.keyboard.type('Ada wrote this line.')
  const invite = await shareActive(a.page, 'Ada')
  await joinFrom(b.page, invite, 'Bo')
  await b.page.waitForFunction(() => document.querySelector('.docx-pages')?.textContent?.includes('Ada wrote this line.'))

  const list = await codex.call('list_documents')
  const docId = /document: "([^"]+)"/.exec(list.text)?.[1]
  check('the shared document is listed with who is in it', !!docId && list.text.includes('Ada') && list.text.includes('Bo'), list.text)
  const read = await codex.call('read_document', { document: docId })
  check('the agent reads numbered paragraphs', read.text.includes('[1] Meeting notes') && read.text.includes('[2] Ada wrote this line.'), read.text)

  // Joining shows the agent to everyone, with its name, logo and color.
  await a.page.waitForFunction(() => [...document.querySelectorAll('.arrival')].some((el) => el.textContent.includes('Codex joined')))
  check('people see a note that Codex joined', true)
  const avatar = await a.page.locator('.share-btn .share-avatar.agent img').count()
  check('the title bar shows Codex’s logo', avatar === 1)

  const ins = await codex.call('insert_paragraphs', { document: docId, after: 2, text: 'Codex added this.\nAnd this.' })
  check('insert_paragraphs works', !ins.error, ins.text)
  await a.page.waitForFunction(() => document.querySelector('.docx-pages')?.textContent?.includes('And this.'))
  await b.page.waitForFunction(() => document.querySelector('.docx-pages')?.textContent?.includes('And this.'))
  const [ta, tb] = [await bodyText(a.page), await bodyText(b.page)]
  check('both people get the agent’s paragraphs, in place', ta === tb && /Ada wrote this line\.\s+Codex added this\.\s+And this\./.test(ta), JSON.stringify([ta, tb]))

  // Its caret shows where it wrote, named, with its logo, in its own color.
  await a.page.waitForFunction(() => [...document.querySelectorAll('.docx-overlay .peer-tag')].some((t) => t.textContent.includes('Codex')))
  const tag = a.page.locator('.docx-overlay .peer-tag', { hasText: 'Codex' })
  check('the agent’s caret is named like a person’s, without its logo', (await tag.innerText()) === 'Codex' && (await tag.locator('img').count()) === 0)
  const color = await a.page.evaluate(() => {
    const s = window.__collab.useCollab.getState()[window.__workspace.useWorkspace.getState().activeId]
    return s.peers.find((p) => p.agent === 'codex')?.color
  })
  check('the agent has its own color', color === '#0D0D0D', color)

  const style = await codex.call('set_style', { document: docId, from: 1, style: 'Heading 1' })
  check('set_style works', !style.error, style.text)
  const again = await codex.call('read_document', { document: docId })
  check('the heading reads back as one', again.text.includes('[1] (Heading 1) Meeting notes'), again.text)

  const repl = await codex.call('replace_text', { document: docId, find: 'Ada wrote', replace: 'Ada typed' })
  check('replace_text works', repl.text.startsWith('Replaced 1'), repl.text)
  await b.page.waitForFunction(() => document.querySelector('.docx-pages')?.textContent?.includes('Ada typed this line.'))

  // The agent sees where people are.
  await b.page.locator('.docx-content-mount').click({ position: { x: 200, y: 120 } })
  await b.page.keyboard.press('Control+End')
  await new Promise((r) => setTimeout(r, 300))
  const where = await codex.call('where_is_everyone', { document: docId })
  check('where_is_everyone says where Bo is', /Bo: paragraph 4/.test(where.text), where.text)

  // Waiting hears someone else's change.
  const waiting = codex.call('wait_for_changes', { document: docId, seconds: 10 })
  await new Promise((r) => setTimeout(r, 300))
  await b.page.keyboard.type(' Bo again.')
  const heard = await waiting
  check('wait_for_changes hears Bo', heard.text.includes('Bo changed it'), heard.text)
  // It hears the first of Bo's keys; the rest follow.
  await a.page.waitForFunction(() => document.querySelector('.docx-pages')?.textContent?.includes('Bo again.'))

  // Undo takes back only the agent's own change.
  const undo = await codex.call('undo', { document: docId })
  check('undo takes back the agent’s last change', !undo.error, undo.text)
  await a.page.waitForFunction(() => document.querySelector('.docx-pages')?.textContent?.includes('Ada wrote this line.'))
  check('Bo’s typing stays', (await bodyText(a.page)).includes('Bo again.'))
  if (shots) await a.page.screenshot({ path: `${shots}/agent-word.png` })

  // Deleting the paragraph just before a table leaves the table.
  await a.page.evaluate(() => {
    const e = window.__docxEditor
    const paras = JSON.parse(e.doc.paragraphs())
    const last = paras[paras.length - 1]
    e.run({ cmd: 'select', anchor: [last.path, last.text.length], focus: [last.path, last.text.length] })
    e.run({ cmd: 'enter' })
    e.run({ cmd: 'insertText', text: 'Before the table.' })
    e.run({ cmd: 'insertTable', rows: 2, cols: 2 })
  })
  await new Promise((r) => setTimeout(r, 500))
  const withTable = await codex.call('read_document', { document: docId })
  const beforeTable = /\[(\d+)\] Before the table\./.exec(withTable.text)?.[1]
  const del = await codex.call('delete_paragraphs', { document: docId, from: Number(beforeTable) })
  await a.page.waitForFunction(() => !document.querySelector('.docx-pages')?.textContent?.includes('Before the table.'))
  const tableLeft = (await codex.call('read_document', { document: docId })).text
  check('deleting the paragraph before a table keeps the table', !del.error && /\(in a table/.test(tableLeft), `${del.text}\n${tableLeft}`)
  const across = await codex.call('delete_paragraphs', { document: docId, from: 1, to: 6 })
  check('deleting across a table is refused', across.error && across.text.includes('Delete them in parts'), across.text)

  // Deleting the first paragraph (a heading) leaves the next one with its own style.
  const first = await codex.call('delete_paragraphs', { document: docId, from: 1 })
  const afterFirst = (await codex.call('read_document', { document: docId, from: 1, to: 1 })).text
  check('the paragraph after a deleted heading keeps its own style', !first.error && /\[1\] Ada wrote this line\./.test(afterFirst), afterFirst)

  // A mistake comes back as an answer the agent can act on.
  const bad = await codex.call('replace_paragraphs', { document: docId, from: 99, text: 'x' })
  check('a wrong paragraph number is explained', bad.error && /has \d+ paragraphs/.test(bad.text), bad.text)

  // A second agent, known by its client's name.
  const claude = await mcpClient(null, 'claude-code')
  await claude.call('read_document', { document: docId })
  await a.page.waitForFunction(() => window.__collab.useCollab.getState()[window.__workspace.useWorkspace.getState().activeId].peers.some((p) => p.agent === 'claude' && p.name === 'Claude' && p.color === '#D97757'))
  check('an agent is known by its MCP client’s name too', true)

  await codex.call('leave_document', { document: docId })
  await a.page.waitForFunction(() => [...document.querySelectorAll('.arrival')].some((el) => el.textContent.includes('Codex left')))
  check('people see a note that Codex left', true)

  // ── Excel
  await a.page.getByRole('tab', { name: 'Home' }).click()
  await a.page.getByRole('button', { name: /New workbook/ }).first().click()
  await a.page.waitForFunction(() => window.__sheets)
  await shareActive(a.page, 'Ada')
  const docs = await codex.call('list_documents')
  const book = /Untitled Workbook\.xlsx {2}document: "([^"]+)"/.exec(docs.text)?.[1]
  const wrote = await codex.call('write_cells', { document: book, cells: { A1: '20', A2: '22', A3: '=A1+A2' } })
  check('write_cells works', !wrote.error, wrote.text)
  await a.page.waitForFunction(() => JSON.parse(window.__sheets.doc.cell(0, 2, 0)).display === '42')
  check('the formula the agent wrote computes for the host', true)
  const cells = await codex.call('read_document', { document: book })
  check('the agent reads formulas and results', cells.text.includes('A3: =A1+A2 → 42'), cells.text)
  await a.page.waitForFunction(() => [...document.querySelectorAll('.sg-body .peer-tag')].some((t) => t.textContent.includes('Codex')))
  check('the cells the agent wrote are outlined with its name', true)
  if (shots) await a.page.screenshot({ path: `${shots}/agent-sheets.png` })

  // ── PowerPoint
  await a.page.getByRole('tab', { name: 'Home' }).click()
  await a.page.getByRole('button', { name: /New presentation/ }).first().click()
  await a.page.waitForFunction(() => window.__slides?.slides?.length >= 1)
  await shareActive(a.page, 'Ada')
  const decks = await codex.call('list_documents')
  const deck = /\.pptx {2}document: "([^"]+)"/.exec(decks.text)?.[1]
  const before = await a.page.evaluate(() => window.__slides.slides.length)
  const added = await codex.call('add_slide', { document: deck, title: 'From Codex', body: 'One\nTwo' })
  check('add_slide works', !added.error, added.text)
  await a.page.waitForFunction((n) => window.__slides.slides.length === n + 1, before)
  const deckText = await codex.call('read_document', { document: deck, slide: before + 1 })
  check('the new slide has its title and body', deckText.text.includes('From Codex') && deckText.text.includes('One / Two'), deckText.text)
  const slideRepl = await codex.call('replace_text', { document: deck, find: 'from codex', replace: 'By Codex' })
  check('replace_text in a presentation says what it changed', slideRepl.text.startsWith('Replaced in 1 shape'), slideRepl.text)
  if (shots) await a.page.screenshot({ path: `${shots}/agent-slides.png` })
} catch (err) {
  check('the run finished', false, String(err?.stack ?? err))
}

a.logs.push(...logs)
await b.browser.close()
await done(a.browser, a.logs)
