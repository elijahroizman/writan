// Two people's agents in one document's chat, each through its own person's Writan: Ada shares,
// Bo joins from another "computer" (a second relay, with its own MCP server), and Ada's Codex and
// Bo's Claude split up the work, while Ada and Bo can read along. Then Bo turns messages off.
// Also paragraph styles, putting paragraphs at the end, and asking before changing someone
// else's, and find_text, redo and open_document.
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

  // Said with nobody here yet, it waits for the next to join.
  const kept = await codex.call('send_message', { document: docA, text: 'Note for whoever comes: the intro is mine.' })
  check('a message with nobody here is kept', !kept.error && kept.text.includes('agents that join later will'), kept.text)
  // Joining, it gets what was left before, with its first answer.
  const joined = await claude.call('read_document', { document: docB })
  check('an agent gets what was said before it joined in its first answer', joined.text.includes('Ada’s Codex, before you joined: Note for whoever comes'), joined.text)
  await a.page.waitForFunction(() => window.__collab.useCollab.getState()[window.__workspace.useWorkspace.getState().activeId].peers.some((p) => p.agent === 'claude' && p.listens))
  const listed = (await codex.call('list_documents')).text
  check('Codex sees Bo’s Claude in the document', listed.includes('Bo’s Claude (AI)'), listed)

  const sent = await codex.call('send_message', { document: docA, text: 'I’ll take the intro; can you do the budget paragraph?' })
  check('send_message says who gets it, and that people can read it', !sent.error && sent.text.startsWith('Sent to the chat: Bo’s Claude gets it.') && sent.text.includes('people in the document can read it'), sent.text)

  // Bo's Claude gets the messages in the answer to its next call, whichever tool, once they've
  // reached Bo's app.
  await new Promise((r) => setTimeout(r, 300))
  const next = await claude.call('find_text', { document: docB, text: 'Intro' })
  check('another tool’s answer brings the new message', next.text.includes('1 new message in the document’s chat') && next.text.includes('Ada’s Codex, just now: I’ll take the intro; can you do the budget paragraph?') && next.text.includes('colleague'), next.text)
  check('once seen, they aren’t shown again', (await claude.call('read_messages', { document: docB })).text === 'No new messages.')

  // The people see the chat, without a note popping up for each message.
  await b.page.waitForFunction(() => window.__collab.useChat.getState()[window.__workspace.useWorkspace.getState().activeId]?.said.length === 2)
  const chatB = await b.page.evaluate(() => window.__collab.useChat.getState()[window.__workspace.useWorkspace.getState().activeId].said.map((m) => `${m.name}: ${m.text}`))
  check('Bo’s window has the chat, the earlier message too', chatB[0] === 'Ada’s Codex: Note for whoever comes: the intro is mine.' && chatB[1].includes('budget paragraph'), chatB.join(' | '))
  const chatA = await a.page.evaluate(() => window.__collab.useChat.getState()[window.__workspace.useWorkspace.getState().activeId].said.map((m) => `${m.name}: ${m.text}`))
  check('Ada’s window has it too, with her agent by its own name', chatA[1]?.startsWith('Codex: I’ll take the intro'), chatA.join(' | '))
  check('no note pops up for a message', (await b.page.locator('.arrival, [role="status"]').filter({ hasText: 'budget paragraph' }).count()) === 0)
  const chatButton = b.page.getByRole('button', { name: /Agents’ chat, 2 new/ })
  check('the title bar has a chat button saying there’s something new', await chatButton.isVisible())
  await chatButton.click()
  const chatPanel = b.page.getByRole('dialog', { name: 'Agents’ chat' })
  await chatPanel.waitFor()
  check('the chat panel shows the messages', (await chatPanel.textContent()).includes('can you do the budget paragraph?'))
  if (process.env.SHOTS) await b.page.screenshot({ path: `${process.env.SHOTS}/chat.png` })
  await b.page.keyboard.press('Escape')
  check('once looked at, nothing is new', await b.page.getByRole('button', { name: 'Agents’ chat' }).isVisible())

  // Codex waits for changes; a message ends the wait, and comes with it.
  const waiting = codex.call('wait_for_changes', { document: docA, seconds: 20 })
  await new Promise((r) => setTimeout(r, 300))
  const reply = await claude.call('send_message', { document: docB, text: 'Deal. Budget is mine.' })
  check('a reply goes to the chat', !reply.error && reply.text.startsWith('Sent to the chat: Ada’s Codex gets it.'), reply.text)
  const woke = await waiting
  check('wait_for_changes ends with the message in its answer', woke.text.startsWith('A message came in the document’s chat') && woke.text.includes('Bo’s Claude, just now: Deal. Budget is mine.'), woke.text)

  // A message that came while the agent was busy ends the next wait at once.
  await claude.call('send_message', { document: docB, text: 'Starting on it now.' })
  await new Promise((r) => setTimeout(r, 300))
  const t0 = Date.now()
  const pending = await codex.call('wait_for_changes', { document: docA, seconds: 20 })
  check('a message not seen yet ends wait_for_changes at once', Date.now() - t0 < 2000 && pending.text.includes('Starting on it now.'), `${Date.now() - t0} ms: ${pending.text}`)

  // read_messages waits for one, and no longer than a client waits for an answer.
  const waitingRead = codex.call('read_messages', { document: docA, seconds: 20 })
  await new Promise((r) => setTimeout(r, 300))
  await claude.call('send_message', { document: docB, text: 'Done with the budget.' })
  check('read_messages with seconds waits for the next', (await waitingRead).text.includes('Done with the budget.'))

  // Messages that crossed: the sender sees what it hadn't before sending.
  await claude.call('send_message', { document: docB, text: 'Want me to do the summary too?' })
  await new Promise((r) => setTimeout(r, 300))
  const crossed = await codex.call('send_message', { document: docA, text: 'I’ll write the summary.' })
  check('send_message shows a message that crossed with it', crossed.text.includes('Want me to do the summary too?'), crossed.text)

  // A third agent joining later gets the chat so far.
  const late = await mcpClient(MCP_A, 'grok', 'grok-build')
  const lateRead = (await late.call('read_document', { document: docA })).text
  check('an agent joining later gets the chat so far', lateRead.includes('before you joined') && lateRead.includes('Done with the budget.') && lateRead.includes('Budget is mine.'), lateRead)
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
  check('list_documents says so', (await codex.call('list_documents')).text.includes('Bo’s Claude (AI, no messages)'))
  check('Bo still sees the chat', await b.page.waitForFunction(() => window.__collab.useChat.getState()[window.__workspace.useWorkspace.getState().activeId]?.said.some((m) => m.text === 'Still there?')).then(() => true, () => false))
  await setMessages(b.page, true)
  await a.page.waitForFunction(() => window.__collab.useCollab.getState()[window.__workspace.useWorkspace.getState().activeId].peers.some((p) => p.agent === 'claude' && p.listens))
  const back = await codex.call('send_message', { document: docA, text: 'Welcome back.' })
  check('turned on again, messages reach it', !back.error, back.text)
  await new Promise((r) => setTimeout(r, 300))
  check('and it reads them', (await claude.call('read_messages', { document: docB })).text.includes('Welcome back.'))

  // A person typing with pauses between words: the wait reports what they wrote once they've
  // stopped, not half of it.
  const typed = (text) =>
    a.page.evaluate((text) => {
      const e = window.__docxEditor
      const ps = JSON.parse(e.doc.paragraphs())
      const p = ps[ps.length - 1]
      e.run({ cmd: 'select', anchor: [p.path, p.text.length], focus: [p.path, p.text.length] })
      e.run({ cmd: 'insertText', text })
    }, text)
  const typing = claude.call('wait_for_changes', { document: docB, seconds: 20 })
  await new Promise((r) => setTimeout(r, 300))
  await typed(' Bye')
  await new Promise((r) => setTimeout(r, 1400))
  await typed(' everyone')
  const whole = await typing
  check('wait_for_changes waits out a pause in someone’s typing', whole.text.includes('Ada changed it.') && whole.text.includes('Bye everyone'), whole.text)
  await codex.call('replace_paragraphs', { document: docA, from: 2, text: 'Budget for the budget review.' })

  // ── Paragraphs: styles, the end, and other people's
  const titled = await codex.call('insert_paragraphs', {
    document: docA,
    after: 0,
    paragraphs: [
      { text: 'The Plan', style: 'Title' },
      { text: 'Written together', style: 'Subtitle' },
      { text: 'Why we meet' },
    ],
  })
  check('insert_paragraphs takes a style for each paragraph, and read shows Title and Subtitle', /\[1\] \(Title\) The Plan/.test(titled.text) && /\[2\] \(Subtitle\) Written together/.test(titled.text) && /\[3\] Why we meet/.test(titled.text), titled.text)
  const afterTitle = await codex.call('insert_paragraphs', { document: docA, after: 1, text: 'By Ada and Bo\nDraft' })
  check('paragraphs put in after a title are body text, not more title', /\[2\] By Ada and Bo/.test(afterTitle.text) && /\[3\] Draft/.test(afterTitle.text), afterTitle.text)
  const replacedTitle = await codex.call('replace_paragraphs', { document: docA, from: 1, text: 'Our Plan\nFirst line of the body' })
  check('replacing a title with more lines keeps the title and makes the rest body text', /\[1\] \(Title\) Our Plan/.test(replacedTitle.text) && /\[2\] First line of the body/.test(replacedTitle.text), replacedTitle.text)
  const atEnd = await claude.call('insert_paragraphs', { document: docB, at_end: true, text: 'Bo’s closing line' })
  const total = Number(/(\d+) paragraphs/.exec((await claude.call('read_document', { document: docB })).text)?.[1])
  check('at_end puts them after the last paragraph', atEnd.text.includes(`[${total}] Bo’s closing line`), atEnd.text)
  await new Promise((r) => setTimeout(r, 300))
  const read = (await codex.call('read_document', { document: docA })).text
  check('read_document says who else wrote a paragraph', read.includes(`[${total}] {by Bo’s Claude} Bo’s closing line`), read)
  // Both agents adding at the end at the same moment: both keep theirs.
  const [endA, endB] = await Promise.all([
    codex.call('insert_paragraphs', { document: docA, at_end: true, text: 'Codex at the end' }),
    claude.call('insert_paragraphs', { document: docB, at_end: true, text: 'Claude at the end' }),
  ])
  await new Promise((r) => setTimeout(r, 500))
  const both = (await codex.call('read_document', { document: docA })).text
  check('two agents adding at the end at once both keep theirs', both.includes('] Codex at the end') && both.includes('Claude at the end') && !endA.error && !endB.error, both)
  const nA = Number(/\[(\d+)\] Codex at the end/.exec(both)?.[1])
  check('and each answer says where its paragraph went', endA.text.includes(`paragraph ${nA}.`), endA.text)
  await codex.call('delete_paragraphs', { document: docA, from: total + 1, to: total + 2, confirm: true })
  const guarded = await codex.call('delete_paragraphs', { document: docA, from: total })
  check('deleting another agent’s paragraph needs confirm', guarded.error && guarded.text.includes('Bo’s Claude’s') && guarded.text.includes('confirm: true'), guarded.text)
  const guardedReplace = await codex.call('replace_paragraphs', { document: docA, from: total, text: 'Mine now' })
  check('so does replacing it', guardedReplace.error && guardedReplace.text.includes('confirm'), guardedReplace.text)
  const confirmed = await codex.call('delete_paragraphs', { document: docA, from: total, confirm: true })
  check('with confirm it goes', !confirmed.error, confirmed.text)
  // Ada's own typing is her agent's person's: no confirm needed.
  await a.page.evaluate(() => {
    const e = window.__docxEditor
    const ps = JSON.parse(e.doc.paragraphs())
    const p = ps[ps.length - 1]
    e.run({ cmd: 'select', anchor: [p.path, p.text.length], focus: [p.path, p.text.length] })
    e.run({ cmd: 'enter' })
    e.run({ cmd: 'insertText', text: 'Ada typed this.' })
  })
  await new Promise((r) => setTimeout(r, 300))
  const own = (await codex.call('read_document', { document: docA })).text
  const adaN = Number(/\[(\d+)\][^\n]*Ada typed this\./.exec(own)?.[1])
  check('its own person’s paragraph isn’t marked as someone else’s', own.includes(`[${adaN}] Ada typed this.`), own)
  const ownDel = await codex.call('delete_paragraphs', { document: docA, from: adaN })
  check('and deleting it needs no confirm', !ownDel.error, ownDel.text)
  // Tidy up for what follows: back to the two paragraphs it started with.
  const now = (await codex.call('read_document', { document: docA })).text
  const introN = Number(/\[(\d+)\] Intro\n/.exec(now)?.[1])
  if (introN > 1) await codex.call('delete_paragraphs', { document: docA, from: 1, to: introN - 1, confirm: true })

  // A paragraph with the same text as the one after it: the answer gives the new one's place.
  const same = await codex.call('insert_paragraphs', { document: docA, after: 0, text: 'Intro' })
  check('the answer gives where a paragraph went, even beside one with the same text', same.text.includes('It’s paragraph 1.'), same.text)
  await codex.call('delete_paragraphs', { document: docA, from: 1 })

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
  check('and Bo’s change stays, as his', (await codex.call('read_document', { document: docA })).text.includes('[2] {by Bo} Budget line by Bo'))
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
