// Two people's agents in one document messaging each other, each through its own person's Writan:
// Ada shares, Bo joins from another "computer" (a second relay, with its own MCP server), and
// Ada's Codex and Bo's Claude split up the work. Then Bo turns messages off. Also find_text, redo
// and open_document, which came with them.
//
// Needs two standalone relays, one per person, both with MCP on:
//   cd src-tauri && cargo run --example collab_relay -- --port 47812 --mcp agents-test-token --bridge agents-test-key
//   cd src-tauri && cargo run --example collab_relay -- --port 47813 --mcp agents-test-token-2 --bridge agents-test-key-2
import { launch, start, check, done } from './harness.mjs'

const MCP_A = process.env.MCP_URL ?? 'http://127.0.0.1:47812/mcp/agents-test-token'
const MCP_B = process.env.MCP_URL_2 ?? 'http://127.0.0.1:47813/mcp/agents-test-token-2'

const a = await launch({
  tauri: false,
  init: () => {
    localStorage.setItem('workspace.agentsBridgeKey', 'agents-test-key')
    localStorage.setItem('workspace.ai', JSON.stringify({ agents: true, token: 'agents-test-token' }))
  },
})
// Bo's app lets agents in through its own relay, as on another computer.
const b = await launch({
  tauri: false,
  init: () => {
    localStorage.setItem('workspace.collabRelay', 'ws://127.0.0.1:47813')
    localStorage.setItem('workspace.agentsBridgeKey', 'agents-test-key-2')
    localStorage.setItem('workspace.ai', JSON.stringify({ agents: true, token: 'agents-test-token-2' }))
  },
})
const logs = []
a.page.on('pageerror', (e) => logs.push(`[pageerror a] ${e.message}`))
b.page.on('pageerror', (e) => logs.push(`[pageerror b] ${e.message}`))

/** An MCP client: initialize, then calls with the session it got. */
async function mcpClient(url, agent, clientName) {
  let session = null
  let id = 0
  const post = async (body) => {
    const res = await fetch(`${url}?agent=${agent}`, {
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
    tools: async () => (await post({ jsonrpc: '2.0', id: ++id, method: 'tools/list' })).result.tools.map((t) => t.name),
    call: async (name, args = {}) => {
      const r = await post({ jsonrpc: '2.0', id: ++id, method: 'tools/call', params: { name, arguments: args } })
      if (r.error) throw new Error(`${name}: ${JSON.stringify(r.error)}`)
      return { text: r.result.content.map((c) => c.text).join('\n'), error: !!r.result.isError }
    },
  }
}

const docIn = async (agent) => /document: "([^"]+)"/.exec((await agent.call('list_documents')).text)?.[1]
const setMessages = (page, on) => page.evaluate((on) => window.__ai.useAi.getState().setMessages(on), on)

try {
  await start(a.page)
  await start(b.page)

  // Ada writes and shares; Bo joins.
  await a.page.getByRole('button', { name: /New document/ }).first().click()
  await a.page.waitForFunction(() => window.__docxEditor)
  await a.page.locator('.docx-content-mount').click({ position: { x: 200, y: 120 } })
  await a.page.keyboard.type('Intro')
  await a.page.keyboard.press('Enter')
  await a.page.keyboard.type('Budget for the budget review.')
  await a.page.getByRole('button', { name: 'Share on your network' }).click()
  const panel = a.page.getByRole('dialog', { name: 'Share on your network' })
  await panel.getByLabel('Your name').fill('Ada')
  await panel.getByRole('button', { name: /^(Save and )?Share$/ }).click()
  const invite = await a.page.getByLabel('Invite link').inputValue()
  await a.page.keyboard.press('Escape')
  await b.page.getByRole('tab', { name: 'Home' }).click()
  await b.page.getByRole('button', { name: 'Join a shared document…' }).click()
  const dialog = b.page.getByRole('dialog', { name: 'Join a shared document' })
  await dialog.getByLabel('Code or link').fill(invite)
  await dialog.getByLabel('Your name').fill('Bo')
  await dialog.getByRole('button', { name: 'Join' }).click()
  await dialog.waitFor({ state: 'detached' })
  await b.page.waitForFunction(() => document.querySelector('.docx-pages')?.textContent?.includes('budget review'))

  // Each person's agent, through each person's own Writan.
  const codex = await mcpClient(MCP_A, 'codex', 'codex-mcp-client')
  const claude = await mcpClient(MCP_B, 'claude', 'claude-code')
  const tools = await codex.tools()
  check('the new tools are listed', ['send_message', 'read_messages', 'find_text', 'redo', 'open_document', 'save_document'].every((t) => tools.includes(t)), tools.join())
  const docA = await docIn(codex)
  const docB = await docIn(claude)
  await codex.call('read_document', { document: docA })
  // Nobody else is in it yet: there's nobody to message.
  const alone = await codex.call('send_message', { document: docA, text: 'Anyone here?', to: 'Claude' })
  check('naming an agent that isn’t there is explained', alone.error && alone.text.includes('no other agent'), alone.text)

  // Said to every agent with nobody here yet, it waits for the next to join.
  const kept = await codex.call('send_message', { document: docA, text: 'Note for whoever comes: the intro is mine.' })
  check('a message to every agent with nobody here is kept', !kept.error && kept.text.includes('agents that join later will'), kept.text)
  await claude.call('read_document', { document: docB })
  await a.page.waitForFunction(() => window.__collab.useCollab.getState()[window.__workspace.useWorkspace.getState().activeId].peers.some((p) => p.agent === 'claude' && p.listens))
  const listed = (await codex.call('list_documents')).text
  check('Codex sees Bo’s Claude in the document', listed.includes('Bo’s Claude (AI)'), listed)

  const sent = await codex.call('send_message', { document: docA, text: 'I’ll take the intro; can you do the budget paragraph?' })
  check('send_message to every agent says who it reached', !sent.error && sent.text === 'Sent to Bo’s Claude.', sent.text)

  // Bo's Claude is told on its next call, once the message has reached Bo's app, and reads it.
  await new Promise((r) => setTimeout(r, 300))
  const next = await claude.call('read_document', { document: docB })
  check('another tool’s answer mentions the new message', next.text.includes('2 new messages from other agents'), next.text)
  const got = await claude.call('read_messages', { document: docB })
  check('read_messages shows who sent what', got.text.includes('Ada’s Codex') && got.text.includes('can you do the budget paragraph?') && got.text.includes('colleague'), got.text)
  check('including what was left before it joined', got.text.includes('Ada’s Codex, before you joined: Note for whoever comes'), got.text)
  check('reading empties them', (await claude.call('read_messages', { document: docB })).text === 'No new messages.')

  // Codex waits for changes; a message ends the wait.
  const waiting = codex.call('wait_for_changes', { document: docA, seconds: 20 })
  await new Promise((r) => setTimeout(r, 300))
  const reply = await claude.call('send_message', { document: docB, to: "Ada's Codex", text: 'Deal. Budget is mine.' })
  check('send_message to one agent by name', !reply.error && reply.text === 'Sent to Ada’s Codex.', reply.text)
  const woke = await waiting
  check('wait_for_changes ends with the message', woke.text.startsWith('Bo’s Claude sent you a message'), woke.text)
  const answer = await codex.call('read_messages', { document: docA })
  check('Codex reads the reply, to it alone', answer.text.includes('Bo’s Claude, to you') && answer.text.includes('Budget is mine.'), answer.text)

  // read_messages waits for one.
  const waitingRead = codex.call('read_messages', { document: docA, seconds: 20 })
  await new Promise((r) => setTimeout(r, 300))
  await claude.call('send_message', { document: docB, text: 'Done with the budget.' })
  check('read_messages with seconds waits for the next', (await waitingRead).text.includes('Done with the budget.'))

  // A third agent joining later gets what was said to every agent.
  const late = await mcpClient(MCP_A, 'grok', 'grok-build')
  await late.call('read_document', { document: docA })
  const earlier = (await late.call('read_messages', { document: docA })).text
  check('an agent joining later gets the earlier messages to everyone', earlier.includes('before you joined') && earlier.includes('Done with the budget.') && !earlier.includes('Budget is mine.'), earlier)
  await late.call('leave_document', { document: docA })

  // Bo turns messages off: his agent loses the tools, and the others can't reach it.
  await setMessages(b.page, false)
  await a.page.waitForFunction(() => window.__collab.useCollab.getState()[window.__workspace.useWorkspace.getState().activeId].peers.some((p) => p.agent === 'claude' && !p.listens))
  const offTools = await claude.tools()
  check('with messages off, the tools aren’t listed', !offTools.includes('send_message') && !offTools.includes('read_messages') && offTools.includes('find_text'), offTools.join())
  const offCall = await claude.call('send_message', { document: docB, text: 'hi' })
  check('with messages off, send_message is refused', offCall.error && offCall.text.includes('turned off messages'), offCall.text)
  const toDeaf = await codex.call('send_message', { document: docA, text: 'Still there?' })
  check('other agents are told it has messages off', !toDeaf.error && toDeaf.text.includes('Bo’s Claude has messages turned off'), toDeaf.text)
  const toDeafOne = await codex.call('send_message', { document: docA, to: 'Bo', text: 'Still there?' })
  check('and one to it alone is refused', toDeafOne.error && toDeafOne.text.includes('can’t get messages'), toDeafOne.text)
  check('list_documents says so', (await codex.call('list_documents')).text.includes('Bo’s Claude (AI, no messages)'))
  await setMessages(b.page, true)
  await a.page.waitForFunction(() => window.__collab.useCollab.getState()[window.__workspace.useWorkspace.getState().activeId].peers.some((p) => p.agent === 'claude' && p.listens))
  const back = await codex.call('send_message', { document: docA, text: 'Welcome back.' })
  check('turned on again, messages reach it', !back.error, back.text)
  check('and it reads them', (await claude.call('read_messages', { document: docB })).text.includes('Welcome back.'))

  // ── The other new tools
  const find = await codex.call('find_text', { document: docA, text: 'budget' })
  check('find_text gives paragraph numbers and counts', find.text.startsWith('2 matches') && find.text.includes('[2] (2 times)'), find.text)
  check('find_text says when nothing matches', (await codex.call('find_text', { document: docA, text: 'zebra' })).text.includes('isn’t in it'))
  await codex.call('replace_paragraphs', { document: docA, from: 1, text: 'Introduction' })
  await codex.call('undo', { document: docA })
  const redo = await codex.call('redo', { document: docA })
  check('redo puts the change back', !redo.error && redo.text.startsWith('Redone'), redo.text)
  await b.page.waitForFunction(() => document.querySelector('.docx-pages')?.textContent?.includes('Introduction'))
  check('everyone gets the redone change', true)
  // Someone else changes what the agent wrote: its undo leaves their change and says so.
  await codex.call('replace_paragraphs', { document: docA, from: 2, text: 'Budget line' })
  await b.page.waitForFunction(() => document.querySelector('.docx-pages')?.textContent?.includes('Budget line'))
  await b.page.evaluate(() => {
    const e = window.__docxEditor
    const p = JSON.parse(e.doc.paragraphs())[1]
    e.run({ cmd: 'select', anchor: [p.path, p.text.length], focus: [p.path, p.text.length] })
    e.run({ cmd: 'insertText', text: ' by Bo' })
  })
  await a.page.waitForFunction(() => document.querySelector('.docx-pages')?.textContent?.includes('Budget line by Bo'))
  await new Promise((r) => setTimeout(r, 300))
  const refused = await codex.call('undo', { document: docA })
  check('undo over someone else’s change says it couldn’t', refused.text.startsWith('Couldn’t undo'), refused.text)
  check('and Bo’s change stays', (await codex.call('read_document', { document: docA })).text.includes('[2] Budget line by Bo'))
  const save = await claude.call('save_document', { document: docB })
  check('a guest’s agent can’t save the host’s document', save.error && save.text.includes('their file'), save.text)

  const opened = await codex.call('open_document', { new: 'pptx' })
  const deck = /document: "([^"]+)"/.exec(opened.text)?.[1]
  check('open_document makes a new presentation', !opened.error && !!deck, opened.text)
  await codex.call('add_slide', { document: deck, title: 'Agenda', body: 'Intro\nBudget' })
  const inDeck = await codex.call('find_text', { document: deck, text: 'budget' })
  check('find_text finds a shape on a slide', /slide \d+, shape "\d+"/.test(inDeck.text), inDeck.text)
  const noPath = await codex.call('save_document', { document: deck })
  check('saving a new document asks for a path', noPath.error && noPath.text.includes('path'), noPath.text)
  const there = await codex.call('save_document', { document: deck, path: process.cwd() + '/package.json' })
  check('save_document won’t write over another file', there.error && /Save a presentation as \.pptx|already exists/.test(there.text), there.text)
} catch (err) {
  check('the run finished', false, String(err?.stack ?? err))
}

a.logs.push(...logs)
await b.browser.close()
await done(a.browser, a.logs)
