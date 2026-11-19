// The slideshow plays a slide's object animations a click at a time before moving on, and going
// back undoes them. The deck is a sample with an animation timeline added here.
// node tests/stress/ui/slides-animations.mjs   (ENGINE=webkit for WebKit)
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import JSZip from 'jszip'
import { launch, start, open, check, done } from './harness.mjs'

const S = resolve(import.meta.dirname, '../samples')
const ctn = (id, attrs, body) => `<p:cTn id="${id}" ${attrs}>${body}</p:cTn>`
const at = (delay) => `<p:stCondLst><p:cond delay="${delay}"/></p:stCondLst>`
let n = 2
const effect = (spid, cls, preset, subtype, nodeType, dur) =>
  `<p:par>${ctn(++n, `presetID="${preset}" presetClass="${cls}" presetSubtype="${subtype}" fill="hold" nodeType="${nodeType}"`, `${at(0)}<p:childTnLst>` +
    `<p:set><p:cBhvr>${ctn(++n, 'dur="1" fill="hold"', at(cls === 'exit' ? dur - 1 : 0))}<p:tgtEl><p:spTgt spid="${spid}"/></p:tgtEl><p:attrNameLst><p:attrName>style.visibility</p:attrName></p:attrNameLst></p:cBhvr><p:to><p:strVal val="${cls === 'exit' ? 'hidden' : 'visible'}"/></p:to></p:set>` +
    `<p:animEffect transition="${cls === 'exit' ? 'out' : 'in'}" filter="fade"><p:cBhvr>${ctn(++n, `dur="${dur}"`, '')}<p:tgtEl><p:spTgt spid="${spid}"/></p:tgtEl></p:cBhvr></p:animEffect>` +
    `</p:childTnLst>`)}</p:par>`
const group = (delay, ...effects) => `<p:par>${ctn(++n, 'fill="hold"', `${at(delay)}<p:childTnLst>${effects.join('')}</p:childTnLst>`)}</p:par>`
const click = (...groups) => `<p:par>${ctn(++n, 'fill="hold"', `${at('indefinite')}<p:childTnLst>${groups.join('')}</p:childTnLst>`)}</p:par>`
// Click 1 fades in box 2, then box 3 flies up after it; click 2 fades box 4 out.
const timing =
  `<p:timing><p:tnLst><p:par>${ctn(1, 'dur="indefinite" restart="never" nodeType="tmRoot"', `<p:childTnLst><p:seq concurrent="1" nextAc="seek">${ctn(2, 'dur="indefinite" nodeType="mainSeq"', `<p:childTnLst>` +
    click(group(0, effect(2, 'entr', 10, 0, 'clickEffect', 400)), group(400, effect(3, 'entr', 2, 4, 'afterEffect', 300))) +
    click(group(0, effect(4, 'exit', 10, 0, 'clickEffect', 300))) +
    `</p:childTnLst>`)}</p:seq></p:childTnLst>`)}</p:par></p:tnLst></p:timing>`

const zip = await JSZip.loadAsync(readFileSync(`${S}/S1-pptx-evil-links.pptx`))
const slide = await zip.file('ppt/slides/slide1.xml').async('string')
zip.file('ppt/slides/slide1.xml', slide.replace('</p:sld>', `${timing}</p:sld>`))
mkdirSync('/tmp/writan-stress', { recursive: true })
const deck = '/tmp/writan-stress/slides-animations.pptx'
writeFileSync(deck, await zip.generateAsync({ type: 'nodebuffer' }))

const { browser, page, logs } = await launch()
try {
  await start(page)
  await open(page, deck)
  await page.waitForFunction(() => window.__slides, null, { timeout: 20000 })
  await page.waitForTimeout(1500)
  await page.evaluate(() => { const w = window.__workspace; w.getEditor(w.useWorkspace.getState().activeId).actions['pptx.present-start']() })
  await page.locator('.present .slide-surface').first().waitFor()
  await page.waitForTimeout(300)
  // Which of boxes 2-5 show, and whether any is still animating.
  const shown = () =>
    page.evaluate(() => {
      const surface = [...document.querySelectorAll('.present .slide-surface')].at(-1)
      const box = (id) => surface.querySelector(`:scope > [data-id="${id}"]`)
      return {
        shown: [2, 3, 4, 5].filter((id) => box(id) && getComputedStyle(box(id)).visibility !== 'hidden'),
        animating: [2, 3, 4, 5].some((id) => box(id)?.getAnimations().length),
      }
    })
  const same = (a, b) => JSON.stringify(a) === JSON.stringify(b)

  let s = await shown()
  check('frames that enter later are hidden when the slide comes up', same(s.shown, [4, 5]), JSON.stringify(s))
  // Box 2 has a link, but while it's hidden a click on it plays the next step instead.
  const hiddenBox = await page.evaluate(() => {
    const r = [...document.querySelectorAll('.present .slide-surface')].at(-1).querySelector(':scope > [data-id="2"]').getBoundingClientRect()
    return { x: r.x + r.width / 2, y: r.y + r.height / 2 }
  })
  await page.mouse.click(hiddenBox.x, hiddenBox.y)
  await page.waitForTimeout(150)
  s = await shown()
  check('a click starts the first step and plays it', s.animating && s.shown.includes(2), JSON.stringify(s))
  check('a hidden frame\'s link does not take the click', (await page.evaluate(() => window.__fake.opened)).length === 0)
  await page.waitForTimeout(1200)
  s = await shown()
  check('the effect set to follow the clicked one plays after it', same(s.shown, [2, 3, 4, 5]) && !s.animating, JSON.stringify(s))
  await page.keyboard.press('ArrowRight')
  await page.waitForTimeout(900)
  s = await shown()
  check('the next click plays the exit', same(s.shown, [2, 3, 5]) && !s.animating, JSON.stringify(s))
  await page.keyboard.press('ArrowLeft')
  await page.waitForTimeout(200)
  s = await shown()
  check('going back undoes the last step', same(s.shown, [2, 3, 4, 5]), JSON.stringify(s))
  await page.keyboard.press('ArrowLeft')
  await page.waitForTimeout(200)
  s = await shown()
  check('going back again returns to the slide as it came up', same(s.shown, [4, 5]), JSON.stringify(s))
  check('the slideshow is still running', (await page.locator('.present').count()) === 1)
} finally {
  await done(browser, logs)
}
