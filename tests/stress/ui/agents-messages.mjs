// Two people's agents in one document's chat, each through its own person's Writan: Ada shares,
// Bo joins from another "computer" (a second relay, with its own MCP server), and Ada's Codex and
// Bo's Claude split up the work, while Ada and Bo can read along. Then Bo turns messages off.
// Also paragraph styles, putting paragraphs at the end, and asking before changing someone
// else's, and find_text, redo and open_document.
//
// Needs two standalone relays, one per person, both with MCP on:
//   cd src-tauri && cargo run --example collab_relay -- --port 47812 --mcp agents-test-token --bridge agents-test-key
//   cd src-tauri && cargo run --example collab_relay -- --port 47813 --mcp agents-test-token-2 --bridge agents-test-key-2
import { launch, start, check, done, shot } from './harness.mjs'

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

/** An answer as the checks written before paragraph ids read it: without the ids, and without
 * the marks on the agent's own and its person's paragraphs. `raw` keeps them. */
const plain = (t) => t.replace(/\[(\d+) #[a-z][0-9a-z]{5}(?:-\d+)?\]/g, '[$1]').replace(/ \{by (?:you|[^{}]+, your person)\}/g, '')

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
      const raw = r.result.content.map((c) => c.text).join('\n')
      return { text: plain(raw), raw, error: !!r.result.isError }
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
  const ownRaw = (await codex.call('read_document', { document: docA })).raw
  check('but it says it’s its person’s', /\{by Ada, your person\} Ada typed this\./.test(ownRaw), ownRaw)
  const ownDel = await codex.call('delete_paragraphs', { document: docA, from: adaN })
  check('and deleting it needs no confirm', !ownDel.error, ownDel.text)
  // Tidy up for what follows: back to the two paragraphs it started with.
  const now = (await codex.call('read_document', { document: docA })).text
  const introN = Number(/\[(\d+)\] Intro\n/.exec(now)?.[1])
  if (introN > 1) await codex.call('delete_paragraphs', { document: docA, from: 1, to: introN - 1, confirm: true })

  // A paragraph with the same text as the one after it: the answer gives the new one's place.
  const same = await codex.call('insert_paragraphs', { document: docA, after: 0, text: 'Intro' })
  check('the answer gives where a paragraph went, even beside one with the same text', same.text.includes('It’s paragraph 1.'), same.text)
  // Beside one with the same text, which of the two is whose can't be told, so it takes confirm.
  const sameDel = await codex.call('delete_paragraphs', { document: docA, from: 1 })
  check('deleting one of two same paragraphs side by side needs confirm', sameDel.error && sameDel.raw.includes('can’t tell whose'), sameDel.raw)
  await codex.call('delete_paragraphs', { document: docA, from: 1, confirm: true })

  // ── The other new tools
  const find = await codex.call('find_text', { document: docA, text: 'budget' })
  check('find_text gives paragraph numbers and counts', find.text.startsWith('2 matches') && find.text.includes('[2] (2 times)'), find.text)
  check('find_text says when nothing matches', (await codex.call('find_text', { document: docA, text: 'zebra' })).text.includes('isn’t in it'))
  await codex.call('replace_paragraphs', { document: docA, from: 1, text: 'Introduction', confirm: true })
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
  // ── Paragraph ids, who wrote what before an agent came, catching up, and who comes and goes
  const idOf = (text, line) => new RegExp(`\\[(\\d+) (#[a-z0-9-]+)\\][^\\n]*${line}`).exec(text)
  const readA = (await codex.call('read_document', { document: docA })).raw
  const readB = (await claude.call('read_document', { document: docB })).raw
  const budget = idOf(readA, 'Budget line by Bo')
  check('read_document gives each paragraph an id', !!budget, readA)
  check('every agent’s copy gives a paragraph the same id', idOf(readB, 'Budget line by Bo')?.[2] === budget?.[2], readB)
  const version = Number(/version (\d+)/.exec(readA)?.[1])
  check('read_document says which version it read', version > 0, readA)
  // Bo's Claude adds paragraphs above it; Codex's id still finds it.
  await claude.call('insert_paragraphs', { document: docB, after: 0, text: 'Agenda\nWho comes' })
  await new Promise((r) => setTimeout(r, 300))
  const byId = await codex.call('insert_paragraphs', { document: docA, after: budget[2], text: 'Totals follow.' })
  const moved = Number(budget[1]) + 2
  check('an id finds its paragraph after others put paragraphs above it', !byId.error && byId.text.includes(`It’s paragraph ${moved + 1}.`), byId.raw)
  // What changed since: only Claude's two and Codex's own.
  const since = (await codex.call('read_document', { document: docA, since: version })).raw
  check('read_document since a version gives only what changed after it', since.includes('{by Bo’s Claude} Agenda') && since.includes('{by you} Totals follow.') && !since.includes('Budget line by Bo') && !since.includes('Introduction'), since)
  // Bo changes the paragraph: the old id is followed to it, but replacing it by the old id is refused.
  await b.page.evaluate(() => {
    const e = window.__docxEditor
    const p = JSON.parse(e.doc.paragraphs()).find((x) => x.text.startsWith('Budget line by Bo'))
    e.run({ cmd: 'select', anchor: [p.path, p.text.length], focus: [p.path, p.text.length] })
    e.run({ cmd: 'insertText', text: ', checked' })
  })
  await a.page.waitForFunction(() => document.querySelector('.docx-pages')?.textContent?.includes('by Bo, checked'))
  await new Promise((r) => setTimeout(r, 300))
  const stale = await codex.call('replace_paragraphs', { document: docA, from: budget[2], text: 'Budget, final', confirm: true })
  check('replacing by an id someone else has changed since is refused', stale.error && stale.raw.includes(`${budget[2]} has changed since (Bo changed it)`) && stale.raw.includes('Budget line by Bo, checked'), stale.raw)
  const followed = await codex.call('format_text', { document: docA, from: budget[2], bold: true })
  check('formatting by that id follows it to the paragraph, and says so', !followed.error && followed.raw.includes(`${budget[2]} has changed since`) && followed.raw.includes('Budget line by Bo, checked'), followed.raw)
  // A message that names a paragraph's id shows under it.
  const totals = idOf((await codex.call('read_document', { document: docA })).raw, 'Totals follow\\.')[2]
  await claude.call('send_message', { document: docB, text: `${totals} needs the numbers from the budget sheet.` })
  await new Promise((r) => setTimeout(r, 300))
  const noted = (await codex.call('read_document', { document: docA })).raw
  check('a message naming a paragraph’s id shows under it', new RegExp(`Totals follow\\.\\n    ↳ Bo’s Claude: ${totals} needs the numbers`).test(noted), noted)
  // Bo's window links the id to its paragraph.
  await b.page.getByRole('button', { name: /Agents’ chat/ }).click()
  const idButton = b.page.locator('.chat-para', { hasText: totals })
  await idButton.waitFor()
  if (process.env.SHOTS) await shot(b.page, `${process.env.SHOTS}/chat-ids.png`, '.chat-panel')
  check('people’s chat links a paragraph id to the paragraph', (await idButton.getAttribute('title')) === 'Totals follow.')
  await idButton.click()
  const atTotals = await b.page.evaluate(() => {
    const e = window.__docxEditor
    const sel = JSON.parse(e.doc.selection())
    return JSON.parse(e.doc.paragraphs()).find((p) => JSON.stringify(p.path) === JSON.stringify(sel.focus[0]))?.text
  })
  check('and clicking it goes there', atTotals === 'Totals follow.', atTotals)
  await b.page.keyboard.press('Escape')
  // Someone coming in ends a wait, and every agent hears of it with its next answer.
  const waitJoin = codex.call('wait_for_changes', { document: docA, seconds: 20 })
  await new Promise((r) => setTimeout(r, 300))
  const grok = await mcpClient(MCP_A, 'grok', 'grok-build')
  const grokRead = (await grok.call('read_document', { document: docA })).raw
  const joinedWait = await waitJoin
  check('someone coming in ends wait_for_changes, and says who', joinedWait.raw.startsWith('Someone came into the document') && /Grok \(AI\) came into the document (just now|\d+ seconds ago)/.test(joinedWait.raw), joinedWait.raw)
  // A new agent knows who wrote what before it came.
  check('an agent that joins later sees who wrote what before it came', grokRead.includes('{by Bo’s Claude} Agenda') && grokRead.includes('{by Bo} Budget line by Bo, checked') && grokRead.includes('{by Codex} Totals follow.'), grokRead)
  check('and a summary of who wrote how much', /Who last changed each paragraph: [^\n]*Bo’s Claude \d/.test(grokRead), grokRead)
  check('and the notes on paragraphs from before it came', grokRead.includes(`↳ Bo’s Claude: ${totals} needs the numbers`), grokRead)
  await grok.call('insert_paragraphs', { document: docA, at_end: true, text: 'Grok was here.' })
  await grok.call('leave_document', { document: docA })
  await new Promise((r) => setTimeout(r, 300))
  const left = await claude.call('find_text', { document: docB, text: 'zebra' })
  check('every answer says who came and went', /Ada’s Grok \(AI\) came into the document/.test(left.raw) && /Ada’s Grok \(AI\) left the document/.test(left.raw), left.raw)
  // Joining after it left, another agent still knows its name.
  const claude2 = await mcpClient(MCP_B, 'claude', 'claude-code')
  const later = (await claude2.call('read_document', { document: docB })).raw
  check('an agent that joins later still names who wrote what and left', later.includes('{by Ada’s Grok} Grok was here.'), later)
  await claude2.call('leave_document', { document: docB })
  // A paragraph whose text another has too has no id: which one an id meant couldn't be told.
  const mineSame = await codex.call('insert_paragraphs', { document: docA, at_end: true, text: 'Same line' })
  const sameId = idOf(mineSame.raw, 'Same line')
  await claude.call('insert_paragraphs', { document: docB, after: 1, text: 'Same line' })
  await new Promise((r) => setTimeout(r, 300))
  const twinRead = (await codex.call('read_document', { document: docA })).raw
  check('paragraphs with the same text have no id', /\[2\] \{by Bo’s Claude\} Same line/.test(twinRead) && /\[\d+\] \{by you\} Same line/.test(twinRead), twinRead)
  const ownSame = await codex.call('replace_paragraphs', { document: docA, from: sameId[2], text: 'Same line, mine' })
  check('and an id from before is refused, not guessed', ownSame.error && ownSame.raw.includes(`No paragraph ${sameId[2]} now`), ownSame.raw)
  // Side by side, who wrote which can't be told, so changing them needs confirm.
  await codex.call('delete_paragraphs', { document: docA, from: 2, confirm: true })
  const nextTo = Number(/\[(\d+)\] \{by you\} Same line/.exec((await codex.call('read_document', { document: docA })).raw)?.[1] ?? sameId[1])
  await claude.call('insert_paragraphs', { document: docB, after: nextTo, text: 'Same line' })
  await new Promise((r) => setTimeout(r, 300))
  const unclear = (await codex.call('read_document', { document: docA })).raw
  check('who wrote two same paragraphs side by side is unclear', (unclear.match(/\{by unclear: one next to it has the same text\} Same line/g) ?? []).length === 2, unclear)
  await codex.call('delete_paragraphs', { document: docA, from: nextTo, to: nextTo + 1, confirm: true })
  // An old id follows its paragraph's changes, but never to a new paragraph that has text it once had.
  const alpha = idOf((await codex.call('insert_paragraphs', { document: docA, at_end: true, text: 'Alpha one' })).raw, 'Alpha one')
  const beta = idOf((await codex.call('replace_paragraphs', { document: docA, from: alpha[2], text: 'Beta one' })).raw, 'Beta one')
  await new Promise((r) => setTimeout(r, 300))
  await claude.call('replace_paragraphs', { document: docB, from: beta[2], text: 'Gamma one', confirm: true })
  await new Promise((r) => setTimeout(r, 300))
  await codex.call('insert_paragraphs', { document: docA, after: 0, text: 'Beta one' })
  const oldAlpha = await codex.call('delete_paragraphs', { document: docA, from: alpha[2] })
  check('an old id isn’t followed to a new paragraph with text it once had', oldAlpha.error && oldAlpha.raw.includes(`No paragraph ${alpha[2]} now`), oldAlpha.raw)
  await codex.call('delete_paragraphs', { document: docA, from: 1 })
  await codex.call('delete_paragraphs', { document: docA, from: beta[1], confirm: true })
  // Nor once its paragraph is deleted and the text written again.
  const delta = idOf((await codex.call('insert_paragraphs', { document: docA, at_end: true, text: 'Delta one' })).raw, 'Delta one')
  const epsilon = idOf((await codex.call('replace_paragraphs', { document: docA, from: delta[2], text: 'Epsilon one' })).raw, 'Epsilon one')
  await new Promise((r) => setTimeout(r, 300))
  await claude.call('delete_paragraphs', { document: docB, from: epsilon[2], confirm: true })
  await new Promise((r) => setTimeout(r, 300))
  await codex.call('insert_paragraphs', { document: docA, after: 0, text: 'Epsilon one' })
  const oldDelta = await codex.call('delete_paragraphs', { document: docA, from: delta[2] })
  check('an old id isn’t followed to text written again after its paragraph was deleted', oldDelta.error && oldDelta.raw.includes(`No paragraph ${delta[2]} now`), oldDelta.raw)
  await codex.call('delete_paragraphs', { document: docA, from: 1 })
  // full: true gives a long changed paragraph whole.
  const long = `Long ${'word '.repeat(90)}end.`
  // (Hearing first of who came and went meanwhile, which would end the wait at once.)
  await codex.call('find_text', { document: docA, text: 'zebra' })
  const waitLong = codex.call('wait_for_changes', { document: docA, seconds: 20, full: true })
  await new Promise((r) => setTimeout(r, 300))
  await claude.call('insert_paragraphs', { document: docB, at_end: true, text: long })
  const whole2 = await waitLong
  check('wait_for_changes with full gives changed paragraphs whole', whole2.raw.includes(long), whole2.raw)

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
