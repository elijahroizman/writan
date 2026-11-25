// An AI agent doing what a person does in shared documents, through Writan's MCP server: seeing
// slides and pages as pictures, adding, moving, resizing, cropping, restacking and formatting
// pictures, shapes and text, slides, tables, Word formatting and pictures, and workbook
// formatting and structure. Each answer says what changed, and waiting says what others changed.
//
// Needs the standalone relay running with MCP on, as agents.mjs does:
//   cd src-tauri && cargo run --example collab_relay -- --port 47812 --mcp agents-test-token --bridge agents-test-key
// SHOTS=dir saves screenshots and the agent's view of a slide and a page.
import { writeFileSync } from 'node:fs'
import { launch, start, check, done } from './harness.mjs'

const MCP = process.env.MCP_URL ?? 'http://127.0.0.1:47812/mcp/agents-test-token'
const shots = process.env.SHOTS

const a = await launch({
  tauri: false,
  init: () => {
    localStorage.setItem('workspace.agentsBridgeKey', 'agents-test-key')
    localStorage.setItem('workspace.ai', JSON.stringify({ agents: true, token: 'agents-test-token' }))
  },
})
const logs = []
a.page.on('pageerror', (e) => logs.push(`[pageerror a] ${e.message}`))

/** An answer as the checks written before paragraph ids read it: without the ids, and without
 * the marks on the agent's own and its person's paragraphs. `raw` keeps them. */
const plain = (t) => t.replace(/\[(\d+) #[a-z][0-9a-z]{5}(?:-\d+)?\]/g, '[$1]').replace(/ \{by (?:you|[^{}]+, your person)\}/g, '')

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
    list: () => post({ jsonrpc: '2.0', id: ++id, method: 'tools/list' }),
    call: async (name, args = {}) => {
      const r = await post({ jsonrpc: '2.0', id: ++id, method: 'tools/call', params: { name, arguments: args } })
      if (r.error) throw new Error(`${name}: ${JSON.stringify(r.error)}`)
      const content = r.result.content
      const raw = content.filter((c) => c.type === 'text').map((c) => c.text).join('\n')
      return { text: plain(raw), raw, images: content.filter((c) => c.type === 'image'), error: !!r.result.isError }
    },
  }
}

async function shareActive(page, name) {
  await page.getByRole('button', { name: 'Share on your network' }).click()
  const panel = page.getByRole('dialog', { name: 'Share on your network' })
  await panel.getByLabel('Your name').fill(name)
  await panel.getByRole('button', { name: /^(Save and )?Share$/ }).click()
  await page.getByLabel('Invite link').waitFor()
  await page.keyboard.press('Escape')
}

/** A PNG made in the page: a block of color with a word on it, as base64. */
const png = (w, h, color, word) =>
  a.page.evaluate(
    ([w, h, color, word]) => {
      const c = document.createElement('canvas')
      c.width = w
      c.height = h
      const g = c.getContext('2d')
      g.fillStyle = color
      g.fillRect(0, 0, w, h)
      g.fillStyle = '#fff'
      g.font = `bold ${Math.round(h / 5)}px sans-serif`
      g.fillText(word, w * 0.1, h * 0.55)
      return c.toDataURL('image/png').split(',')[1]
    },
    [w, h, color, word],
  )

/** The host's own engine's view of a shape on a slide, from 1. */
const hostShape = (slide, id) =>
  a.page.evaluate(([slide, id]) => {
    const walk = (els) => {
      for (const el of els) {
        if (el.id === id) return el
        const hit = el.children && walk(el.children)
        if (hit) return hit
      }
      return null
    }
    return walk(JSON.parse(window.__slides.doc.elements(slide - 1)))
  }, [slide, id])

const waitFor = (f, arg) => a.page.waitForFunction(f, arg, { timeout: 10_000 })
const idIn = (text) => /"(\d+)"/.exec(text)?.[1]

const codex = await mcpClient('codex', 'codex-mcp-client')

try {
  await start(a.page)
  const tools = (await codex.list()).result.tools
  const names = tools.map((t) => t.name)
  check(
    'the new tools are listed',
    ['view_document', 'move_shape', 'crop_picture', 'format_text', 'format_shape', 'add_shape', 'add_picture', 'delete_shapes', 'copy_shapes', 'group_shapes', 'set_table_text', 'duplicate_slide', 'move_slide', 'set_slide', 'insert_table', 'change_table', 'format_picture', 'format_cells', 'size_cells', 'insert_cells', 'delete_cells', 'merge_cells', 'add_chart', 'edit_chart', 'edit_sheet'].every((t) => names.includes(t)),
    names.join(),
  )
  check('every tool has a unique name and an object schema', new Set(names).size === names.length && tools.every((t) => t.inputSchema?.type === 'object'))

  // ── PowerPoint
  await a.page.getByRole('button', { name: /New presentation/ }).first().click()
  await a.page.waitForFunction(() => window.__slides?.slides?.length >= 1)
  await shareActive(a.page, 'Ada')
  const deck = /\.pptx {2}document: "([^"]+)"/.exec((await codex.call('list_documents')).text)?.[1]
  const added = await codex.call('add_slide', { document: deck, title: 'Today’s menu' })
  const n = Number(/Added slide (\d+)/.exec(added.text)?.[1])
  check('add_slide answers with the new slide’s shapes', n > 0 && added.text.includes('Today’s menu') && /shape "\d+"/.test(added.text), added.text)
  const read = await codex.call('read_document', { document: deck, slide: n })
  check('read_document gives the slide size and each shape’s place and look', /1280 × 720 pixels/.test(read.text) && /at x \d+, y \d+, \d+ × \d+/.test(read.text) && /text .*\d+pt/.test(read.text), read.text)
  const title = /shape "(\d+)"[^\n]*Today’s menu/.exec(read.text)?.[1]

  // Pictures go in from data and from an address, and say where they are.
  const noodles = await codex.call('add_picture', { document: deck, slide: n, data: await png(800, 400, '#c0392b', 'Noodles'), name: 'Noodles', x: 40, y: 200, width: 400 })
  const p1 = idIn(noodles.text)
  check('add_picture puts in a picture from data, sized and placed, keeping its proportions', !noodles.error && /picture “Noodles”: at x 40, y 200, 400 × 200/.test(noodles.text), noodles.text)
  await waitFor(([n, id]) => JSON.parse(window.__slides.doc.elements(n - 1)).some((el) => el.id === id && el.type === 'image'), [n, p1])
  check('the host gets the picture', true)
  const logo = await codex.call('add_picture', { document: deck, slide: n, url: 'http://localhost:1420/favicon.svg', height: 120 })
  const p2 = idIn(logo.text)
  check('add_picture takes an address too', !logo.error && / × 120/.test(logo.text), logo.text)
  const notPicture = await codex.call('add_picture', { document: deck, slide: n, data: btoa('just some text, not a picture') })
  check('something that isn’t a picture is refused', notPicture.error && /isn’t a picture/.test(notPicture.text), notPicture.text)

  // Moving and resizing keeps proportions unless told not to, and says what it overlaps.
  const moved = await codex.call('move_shape', { document: deck, slide: n, shape: p1, x: 100, y: 180, width: 600 })
  check('move_shape moves and resizes, keeping proportions', /at x 100, y 180, 600 × 300/.test(moved.text), moved.text)
  const host = await hostShape(n, p1)
  check('the host sees it moved', Math.round(host.x) === 100 && Math.round(host.width) === 600, JSON.stringify(host))
  const onTop = await codex.call('move_shape', { document: deck, slide: n, shape: p2, x: 650, y: 200 })
  check('move_shape says what a shape overlaps', onTop.text.includes(`It overlaps "${p1}"`), onTop.text)
  const off = await codex.call('move_shape', { document: deck, slide: n, shape: p2, x: 1250 })
  check('move_shape says when a shape runs off the slide', /off the edge of the slide/.test(off.text), off.text)
  await codex.call('move_shape', { document: deck, slide: n, shape: p2, x: 650, y: 200 })

  // Restacking: shapes are listed back to front.
  await codex.call('move_shape', { document: deck, slide: n, shape: p2, layer: 'back' })
  let order = [...(await codex.call('read_document', { document: deck, slide: n })).text.matchAll(/shape "(\d+)"/g)].map((m) => m[1])
  check('layer back sends it behind the others', order[0] === p2, order.join())
  await codex.call('move_shape', { document: deck, slide: n, shape: p2, layer: 'front' })
  order = [...(await codex.call('read_document', { document: deck, slide: n })).text.matchAll(/shape "(\d+)"/g)].map((m) => m[1])
  check('layer front brings it in front', order[order.length - 1] === p2, order.join())
  const flip = await codex.call('move_shape', { document: deck, slide: n, shape: p1, rotation: 10, flip_horizontal: true })
  check('move_shape turns and flips', /turned 10°, flipped horizontally/.test(flip.text), flip.text)
  await codex.call('move_shape', { document: deck, slide: n, shape: p1, rotation: 0, flip_horizontal: false })

  // Cropping: fill a square frame, then fit the whole picture again, then cut a side.
  await codex.call('move_shape', { document: deck, slide: n, shape: p1, width: 300, height: 300, keep_proportions: false })
  const fill = await codex.call('crop_picture', { document: deck, slide: n, shape: p1, fit: 'fill' })
  check('crop_picture fill crops a wide picture to its square frame', /300 × 300; cropped 25% left, 25% right/.test(fill.text), fill.text)
  const keepLeft = await codex.call('crop_picture', { document: deck, slide: n, shape: p1, fit: 'fill', keep: 'left' })
  check('fill can keep one side', /cropped 50% right/.test(keepLeft.text) && !/% left/.test(keepLeft.text), keepLeft.text)
  const fit = await codex.call('crop_picture', { document: deck, slide: n, shape: p1, fit: 'fit' })
  check('crop_picture fit shows it whole, at its own proportions', /at x 100, y 255, 300 × 150/.test(fit.text) && !/cropped/.test(fit.text), fit.text)
  const side = await codex.call('crop_picture', { document: deck, slide: n, shape: p1, left: 0.5 })
  check('cropping a side keeps the rest where it was', /at x 250, y 255, 150 × 150; cropped 50% left/.test(side.text), side.text)
  check('the host sees the crop', (await hostShape(n, p1)).crop?.[0] === 0.5)
  const notPic = await codex.call('crop_picture', { document: deck, slide: n, shape: title, fit: 'fill' })
  check('cropping a text box is refused', notPic.error && /isn’t a picture/.test(notPic.text), notPic.text)

  // Text formatting: the whole shape, a paragraph, or a word.
  const big = await codex.call('format_text', { document: deck, slide: n, shape: title, size: 54, bold: true, color: '#1F4E79', align: 'center', font: 'Georgia' })
  check('format_text formats a whole shape’s text', /Georgia 54pt bold #1f4e79 center/i.test(big.text), big.text)
  const box = await codex.call('add_shape', { document: deck, slide: n, kind: 'text box', text: 'Pad thai\nGreen curry', x: 760, y: 200, width: 400, height: 120, size: 24 })
  const tb = idIn(box.text)
  check('add_shape adds a text box with text where asked', !box.error && /at x 760, y 200, 400 × \d+/.test(box.text) && box.text.includes('Pad thai / Green curry') && /24pt/.test(box.text), box.text)
  const word = await codex.call('format_text', { document: deck, slide: n, shape: tb, find: 'curry', italic: true })
  check('format_text can format just a word', !word.error && /Formatted 1 match/.test(word.text), word.text)
  const second = await codex.call('format_text', { document: deck, slide: n, shape: tb, from: 2, color: '#00A000' })
  check('format_text can format one paragraph', !second.error && /Formatted paragraph 2/.test(second.text), second.text)
  const hostTitle = await hostShape(n, title)
  check('the host gets the formatting', hostTitle.textStyle?.fontSize === 54 && hostTitle.textStyle?.bold && hostTitle.textStyle?.fontFamily === 'Georgia', JSON.stringify(hostTitle.textStyle))
  check('the host gets both lines', (await hostShape(n, tb)).text === 'Pad thai\nGreen curry')

  // Shapes: fill and outline, copies, groups, deleting.
  const rect = await codex.call('add_shape', { document: deck, slide: n, kind: 'rounded rectangle', x: 60, y: 520, width: 300, height: 120, fill: '#FFE699', outline: 'none', text: 'Specials' })
  const r1 = idIn(rect.text)
  check('add_shape adds a filled shape with text', /fill #ffe699/i.test(rect.text) && rect.text.includes('Specials'), rect.text)
  const outlined = await codex.call('format_shape', { document: deck, slide: n, shape: r1, outline: '#C00000', outline_width: 3 })
  check('format_shape sets the outline', /outline #c00000 3pt/i.test(outlined.text), outlined.text)
  const copied = await codex.call('copy_shapes', { document: deck, slide: n, shapes: [r1], to_slide: 1 })
  check('copy_shapes copies onto another slide', !copied.error && /Copied to slide 1 as "\d+"/.test(copied.text) && copied.text.includes('Specials'), copied.text)
  const grouped = await codex.call('group_shapes', { document: deck, slide: n, shapes: [r1, tb] })
  const g = /Grouped as shape "(\d+)"/.exec(grouped.text)?.[1]
  check('group_shapes groups', !!g && /group/.test(grouped.text), grouped.text)
  const inGroup = await codex.call('move_shape', { document: deck, slide: n, shape: tb, x: 0 })
  check('moving a shape inside a group says to move the group or ungroup', inGroup.error && inGroup.text.includes(`inside group "${g}"`), inGroup.text)
  const childText = await codex.call('format_text', { document: deck, slide: n, shape: tb, bold: true })
  check('formatting a shape inside a group says to ungroup', childText.error && childText.text.includes(`inside group "${g}"`), childText.text)
  const ungrouped = await codex.call('group_shapes', { document: deck, slide: n, shapes: [g], ungroup: true })
  check('group_shapes with ungroup takes it apart', !ungrouped.error && /Ungrouped into/.test(ungrouped.text), ungrouped.text)
  const shapesBefore = (await a.page.evaluate((n) => JSON.parse(window.__slides.doc.elements(n - 1)).length, n))
  const badColor = await codex.call('add_shape', { document: deck, slide: n, kind: 'oval', outline: 'not a color' })
  check('a bad color is refused before anything is added', badColor.error && (await a.page.evaluate((n) => JSON.parse(window.__slides.doc.elements(n - 1)).length, n)) === shapesBefore, badColor.text)
  const line = await codex.call('add_shape', { document: deck, slide: n, kind: 'line', x: 60, y: 660, width: 300 })
  const l1 = idIn(line.text)
  check('add_shape adds a line', !line.error && /line/.test(line.text), line.text)
  const lineBack = await codex.call('move_shape', { document: deck, slide: n, shape: l1, layer: 'back' })
  check('a line can be restacked, and the answer says where it is in the stack', !lineBack.error && /It’s 1 of \d+ from the back/.test(lineBack.text), lineBack.text)
  const lineMoved = await codex.call('move_shape', { document: deck, slide: n, shape: l1, x: 80, y: 650 })
  check('moving a line keeps it flat', /at x 80, y 650, 300 × [01];/.test(lineMoved.text), lineMoved.text)
  await codex.call('delete_shapes', { document: deck, slide: n, shapes: [l1] })
  const del = await codex.call('delete_shapes', { document: deck, slide: n, shapes: [p2] })
  check('delete_shapes deletes', !del.error && !(await hostShape(n, p2)), del.text)

  // Tables grow to fit what's written.
  const table = await codex.call('add_shape', { document: deck, slide: n, kind: 'table', rows: 2, columns: 2, x: 700, y: 480, width: 500, height: 160 })
  const t1 = idIn(table.text)
  const cells = await codex.call('set_table_text', { document: deck, slide: n, shape: t1, cells: [['Dish', 'Price', 'Spicy'], ['Pad thai', '$12', 'No'], ['Green curry', '$14', 'Yes']] })
  check('set_table_text fills cells and grows the table', !cells.error && /3 rows × 3 columns/.test(cells.text) && cells.text.includes('Green curry'), cells.text)

  // The agent sees the slide, with each shape's id on it.
  const view = await codex.call('view_document', { document: deck, slide: n })
  check('view_document gives a picture of the slide', view.images.length === 1 && view.images[0].mimeType === 'image/jpeg' && view.images[0].data.length > 1000 && /1280 × 720/.test(view.text), view.text.slice(0, 200))
  if (shots) writeFileSync(`${shots}/agent-view-slide.jpg`, Buffer.from(view.images[0].data, 'base64'))
  const close = await codex.call('view_document', { document: deck, slide: n, shape: p1, labels: false })
  check('view_document can show one shape closer up', close.images.length === 1 && /shape "\d+"/.test(close.text))
  if (shots) writeFileSync(`${shots}/agent-view-shape.jpg`, Buffer.from(close.images[0].data, 'base64'))

  // Slides: copy, move, hide, layout, delete.
  const before = await a.page.evaluate(() => window.__slides.slides.length)
  const dup = await codex.call('duplicate_slide', { document: deck, slide: n })
  check('duplicate_slide copies a slide after it', !dup.error && dup.text.includes(`Copied slide ${n} as slide ${n + 1}`), dup.text)
  await waitFor((k) => window.__slides.slides.length === k, before + 1)
  const mv = await codex.call('move_slide', { document: deck, slide: n + 1, to: 1 })
  check('move_slide moves it', !mv.error, mv.text)
  const hide = await codex.call('set_slide', { document: deck, slide: 1, hidden: true })
  check('set_slide hides a slide', /\(hidden\)/.test(hide.text), hide.text)
  await waitFor(() => window.__slides.slides[0].hidden === true)
  const gone = await codex.call('delete_slide', { document: deck, slide: 1 })
  check('delete_slide deletes it', !gone.error, gone.text)
  await waitFor((k) => window.__slides.slides.length === k, before)

  // Waiting says which slide and shape someone else changed.
  const waiting = codex.call('wait_for_changes', { document: deck, seconds: 15 })
  await new Promise((r) => setTimeout(r, 400))
  await a.page.evaluate(([n, id]) => {
    const e = window.__slides
    e.run({ cmd: 'setActive', index: n - 1 })
    const el = JSON.parse(e.doc.elements(n - 1)).find((x) => x.id === id)
    e.run({ cmd: 'setBox', id: Number(id), x: 20, y: el.y, w: el.width, h: el.height, rot: 0 })
  }, [n, p1])
  const heard = await waiting
  const waitingStack = codex.call('wait_for_changes', { document: deck, seconds: 15 })
  await new Promise((r) => setTimeout(r, 400))
  await a.page.evaluate(([n, id]) => {
    const e = window.__slides
    e.run({ cmd: 'setActive', index: n - 1 })
    e.run({ cmd: 'selectFrames', ids: [Number(id)] })
    e.run({ cmd: 'restack', how: 'front' })
  }, [n, p1])
  const heardStack = await waitingStack
  check('wait_for_changes says when shapes were restacked', /restacked, back to front now: .*"\d+"$/m.test(heardStack.text) && heardStack.text.trimEnd().includes(`"${p1}"`), heardStack.text)
  check('wait_for_changes says who changed which slide and shape', heard.text.includes('Ada changed it') && heard.text.includes(`Slide ${n}:`) && heard.text.includes(`changed shape "${p1}"`) && /at x 20,/.test(heard.text), heard.text)
  if (shots) await a.page.screenshot({ path: `${shots}/agent-slides-host.png` })

  // ── Word
  await a.page.getByRole('tab', { name: 'Home' }).click()
  await a.page.getByRole('button', { name: /New document/ }).first().click()
  await a.page.waitForFunction(() => window.__docxEditor)
  await a.page.locator('.docx-content-mount').click({ position: { x: 200, y: 120 } })
  await a.page.keyboard.type('Menu')
  await a.page.keyboard.press('Enter')
  await a.page.keyboard.type('Every dish is made to order.')
  await shareActive(a.page, 'Ada')
  const docs = await codex.call('list_documents')
  const docId = /\.docx {2}document: "([^"]+)"/.exec(docs.text)?.[1]
  const fmt = await codex.call('format_text', { document: docId, from: 1, size: 28, bold: true, color: '#1F4E79', align: 'center' })
  check('format_text formats Word paragraphs and says how they look', !fmt.error && /28pt/.test(fmt.text) && /bold/.test(fmt.text) && /aligned center/.test(fmt.text), fmt.text)
  const boldWord = await codex.call('format_text', { document: docId, from: 2, find: 'dish', bold: true })
  const unbold = await codex.call('format_text', { document: docId, from: 1, to: 2, bold: false })
  const dish = await codex.call('format_text', { document: docId, from: 2, find: 'dish', italic: false })
  check('bold false unbolds paragraphs that are partly bold', !boldWord.error && /bold/.test(boldWord.text.split('\n')[0]) && !unbold.error && !dish.error && !/bold/.test(dish.text.split('\n')[0]), `${boldWord.text}\n${dish.text}`)
  const heading = await codex.call('format_text', { document: docId, from: 1, italic: false })
  check('…and every paragraph in the range', !heading.error && !/bold/.test(heading.text.split('\n')[0]), heading.text)
  await codex.call('format_text', { document: docId, from: 1, bold: true })
  const part = await codex.call('format_text', { document: docId, from: 2, find: 'order', italic: true, highlight: 'yellow' })
  check('format_text formats just the words found', !part.error && /Formatted 1 match/.test(part.text) && /italic/.test(part.text), part.text)
  const list = await codex.call('insert_paragraphs', { document: docId, after: 2, text: 'Pad thai\nGreen curry' })
  check('insert_paragraphs answers with the new paragraphs', /\[3\] Pad thai/.test(list.text) && /\[4\] Green curry/.test(list.text), list.text)
  const bullets = await codex.call('format_text', { document: docId, from: 3, to: 4, list: 'bullet' })
  check('format_text makes a list', /bullet list/.test(bullets.text), bullets.text)
  const tbl = await codex.call('insert_table', { document: docId, after: 4, cells: [['Dish', 'Price'], ['Pad thai', '$12']] })
  check('insert_table puts in a filled table', !tbl.error && /row 2, column 2\) \$12/.test(tbl.text), tbl.text)
  const cellPara = Number(/\[(\d+)\] \(in a table, row 2, column 1\)/.exec(tbl.text)?.[1])
  const row = await codex.call('change_table', { document: docId, paragraph: cellPara, action: 'add_row_below' })
  check('change_table adds a row', !row.error, row.text)
  const afterRow = (await codex.call('read_document', { document: docId })).text
  check('the new row reads back', /row 3, column 2\)/.test(afterRow), afterRow)
  const pic = await codex.call('add_picture', { document: docId, after: 2, data: await png(600, 300, '#27ae60', 'Basil'), width: 300 })
  check('add_picture puts a picture in a Word document, sized', !pic.error && /300 × 150 pixels/.test(pic.text), pic.text)
  const cropW = await codex.call('format_picture', { document: docId, paragraph: 3, right: 0.5 })
  check('format_picture crops a Word picture', !cropW.error && /150 × 150 pixels, cropped 50% right/.test(cropW.text), cropW.text)
  const reset = await codex.call('format_picture', { document: docId, paragraph: 3, reset: true })
  check('format_picture reset uncrops', !reset.error && !/cropped \d/.test(reset.text), reset.text)
  await codex.call('format_picture', { document: docId, paragraph: 3, width: 300 })
  await codex.call('format_picture', { document: docId, paragraph: 3, right: 0.5 })
  const page = await codex.call('view_document', { document: docId, page: 1 })
  check('view_document gives a picture of a page', page.images.length === 1 && /Page 1 of 1/.test(page.text), page.text)
  if (shots) writeFileSync(`${shots}/agent-view-page.jpg`, Buffer.from(page.images[0].data, 'base64'))
  await a.page.waitForFunction(() => document.querySelector('.docx-pages')?.textContent?.includes('Green curry'))
  const waitW = codex.call('wait_for_changes', { document: docId, seconds: 15 })
  await new Promise((r) => setTimeout(r, 400))
  await a.page.locator('.docx-content-mount').click({ position: { x: 200, y: 120 } })
  await a.page.keyboard.press('Home')
  await a.page.keyboard.type('Our ')
  const heardW = await waitW
  check('wait_for_changes says which Word paragraphs changed', heardW.text.includes('Ada changed it') && /Paragraph 1 is now:\n\[1\] Our Menu/.test(heardW.text), heardW.text)
  if (shots) await a.page.screenshot({ path: `${shots}/agent-word-host.png` })

  // ── Excel
  await a.page.getByRole('tab', { name: 'Home' }).click()
  await a.page.getByRole('button', { name: /New workbook/ }).first().click()
  await a.page.waitForFunction(() => window.__sheets)
  await shareActive(a.page, 'Ada')
  const book = /\.xlsx {2}document: "([^"]+)"/.exec((await codex.call('list_documents')).text)?.[1]
  const wrote = await codex.call('write_cells', { document: book, cells: { A1: 'Dish', B1: 'Price', A2: 'Pad thai', B2: '12', A3: 'Curry', B3: '14', B4: '=SUM(B2:B3)' } })
  check('write_cells answers with what the cells show', /B4: =SUM\(B2:B3\) → 26/.test(wrote.text), wrote.text)
  const money = await codex.call('format_cells', { document: book, range: 'B2:B4', number_format: '$#,##0.00' })
  check('format_cells sets a number format, and the answer shows it', /B4: =SUM\(B2:B3\) → \$26\.00/.test(money.text), money.text)
  const head = await codex.call('format_cells', { document: book, range: 'A1:B1', bold: true, fill: '#DDEBF7', borders: 'bottom' })
  check('format_cells formats', !head.error, head.text)
  const bold = await a.page.evaluate(() => JSON.parse(window.__sheets.doc.cell(0, 0, 0)))
  check('the host gets the format', JSON.stringify(bold).includes('true') || bold.bold === true, JSON.stringify(bold))
  check('size_cells sizes a column', !(await codex.call('size_cells', { document: book, columns: 'A', width: 160 })).error)
  const ins = await codex.call('insert_cells', { document: book, row: 2 })
  const moved4 = (await codex.call('read_document', { document: book })).text
  check('insert_cells inserts a row, and formulas follow', !ins.error && /B5: =SUM\(B3:B4\)/.test(moved4), moved4)
  const delr = await codex.call('delete_cells', { document: book, rows: '2' })
  const back = (await codex.call('read_document', { document: book })).text
  check('delete_cells deletes it again', !delr.error && /B4: =SUM\(B2:B3\)/.test(back), back)
  check('merge_cells merges', !(await codex.call('merge_cells', { document: book, range: 'D1:E1' })).error)
  const chart = await codex.call('add_chart', { document: book, range: 'A1:B3', kind: 'column' })
  check('add_chart adds a chart', !chart.error && (await a.page.evaluate(() => window.__sheets.chartsOf(0).length)) === 1, chart.text)
  const titled = await codex.call('edit_chart', { document: book, chart: 1, title: 'Prices', x_title: 'Dish', y_title: 'Price', legend: 'bottom' })
  check('edit_chart sets the title, axis titles and legend', !titled.error && /“Prices”/.test(titled.text), titled.text)
  const colored = await codex.call('edit_chart', { document: book, chart: 1, colors: ['#E5007E'] })
  check('edit_chart sets a series color', !colored.error && /#E5007E/i.test(colored.text), colored.text)
  const hostChart = await a.page.evaluate(() => {
    const c = window.__sheets.chartsOf(0)[0]
    return { title: c.title, cat: c.catTitle, val: c.valTitle, legend: c.legend, color: c.series[0]?.color }
  })
  check('the host sees the chart change', hostChart.title === 'Prices' && hostChart.cat === 'Dish' && hostChart.val === 'Price' && hostChart.legend === 'bottom' && hostChart.color === '#E5007E', JSON.stringify(hostChart))
  const turned = await codex.call('edit_chart', { document: book, chart: 1, kind: 'bar', grouping: 'stacked' })
  check('edit_chart changes kind and stacking', !turned.error && /stacked bar/.test(turned.text), turned.text)
  const noChart = await codex.call('edit_chart', { document: book, chart: 9, title: 'x' })
  check('a chart number that isn’t there is refused', noChart.error && /has 1 chart/.test(noChart.text), noChart.text)
  const listed = (await codex.call('read_document', { document: book })).text
  check('read_document lists the chart', /Charts:\n- chart 1: stacked bar.*“Prices”/.test(listed), listed.split('Charts:')[1]?.trim())
  const sheetView = await codex.call('view_document', { document: book })
  check('view_document gives a picture of the sheet', sheetView.images.length === 1 && sheetView.images[0].data.length > 1000 && /Sheet “Sheet1”/.test(sheetView.text), sheetView.text.slice(0, 200))
  if (shots) writeFileSync(`${shots}/agent-view-sheet.jpg`, Buffer.from(sheetView.images[0].data, 'base64'))
  const chartPic = await codex.call('view_document', { document: book, chart: 1, labels: false })
  check('view_document can show one chart closer up', chartPic.images.length === 1 && /chart 1/.test(chartPic.text), chartPic.text.slice(0, 200))
  const sheet = await codex.call('edit_sheet', { document: book, action: 'add', name: 'Costs' })
  check('edit_sheet adds a named sheet', /Sheets now: Sheet1, Costs/.test(sheet.text), sheet.text)
  const ren = await codex.call('edit_sheet', { document: book, action: 'rename', sheet: 'Costs', name: 'Suppliers' })
  check('edit_sheet renames', /Sheet1, Suppliers/.test(ren.text), ren.text)
  await waitFor(() => window.__sheets.sheets.some((s) => s.name === 'Suppliers'))
  check('the host gets the new sheet', true)
  if (shots) await a.page.screenshot({ path: `${shots}/agent-sheets-host.png` })
} catch (err) {
  check('the run finished', false, String(err?.stack ?? err))
}

a.logs.push(...logs)
await done(a.browser, a.logs)
