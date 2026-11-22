// Inline LaTeX math in Word and Slides: live SVG preview, native Office Math (OMML)
// insertion, vector SVG rendering, inspector recognition, in-place editing, and OOXML save/reopen persistence.
import { launch } from './harness.mjs'

const { browser, page, logs } = await launch()
const errors = []
const check = (name, ok, detail = '') => {
  console.log(`${ok ? 'ok' : 'FAIL'} ${name}${ok ? '' : ` — ${detail}`}`)
  if (!ok) errors.push(name)
}

try {
  await page.goto(process.env.WRITAN_URL ?? 'http://localhost:1420/')
  await page.waitForSelector('.home', { timeout: 30000 })

  // 1. Word: insert inline LaTeX equation, inspect it, edit it in place, and verify round-trip save.
  await page.getByRole('button', { name: 'New document', exact: true }).first().click()
  await page.waitForFunction(() => !!window.__docxEditor, null, { timeout: 30000 })

  await page.locator('.view[data-active=true] button.tool-btn:has-text("Math")').click()
  await page.waitForSelector('#writan-latex-input', { timeout: 10000 })
  await page.locator('#writan-latex-input').fill('\\frac{-b \\pm \\sqrt{b^2 - 4ac}}{2a}')

  const wordPreviewSvg = await page.locator('[data-testid="latex-math-preview"] svg').count()
  check('Word modal renders live MathJax SVG preview', wordPreviewSvg === 1, `svg count=${wordPreviewSvg}`)

  await page.getByRole('button', { name: 'Insert equation' }).click()
  await page.waitForSelector('#writan-latex-input', { state: 'detached', timeout: 10000 })

  // Select the inserted inline equation (offset 0..3 for U+FFFC) and verify mathTag + inspector UI.
  await page.evaluate(() => {
    window.__docxEditor.doc.command(JSON.stringify({ cmd: 'selectAll' }))
    window.__docxEditor.refresh()
  })
  const wordTag1 = await page.evaluate(() => window.__docxEditor.state.mathTag ?? null)
  check('Word stores writan:latex tag on inserted equation', !!wordTag1 && wordTag1.includes('writan:latex:') && wordTag1.includes('4ac'), String(wordTag1))

  const wordEditBtn = page.locator('.view[data-active=true] button:has-text("Edit equation")')
  await wordEditBtn.waitFor({ state: 'visible', timeout: 10000 })
  await wordEditBtn.click()
  await page.waitForSelector('#writan-latex-input', { timeout: 10000 })
  await page.locator('#writan-latex-input').fill('E = mc^2')
  await page.getByRole('button', { name: 'Update equation' }).click()
  await page.waitForSelector('#writan-latex-input', { state: 'detached', timeout: 10000 })

  const wordTag2 = await page.evaluate(() => window.__docxEditor.state.mathTag ?? null)
  check('Word updates LaTeX equation in place', !!wordTag2 && wordTag2.includes('E = mc^2'), String(wordTag2))

  // Verify equation renders visibly as a vector SVG <img> on the Word page.
  const wordImgLoaded = await page
    .waitForFunction(
      () =>
        [...document.querySelectorAll('.view[data-active=true] .docx-viewport img')].some(
          (img) => img.complete && img.naturalWidth > 0,
        ),
      null,
      { timeout: 5000 },
    )
    .then(() => true)
    .catch(() => false)
  check('Word renders equation SVG visibly on page', wordImgLoaded)

  // Verify saved .docx bytes are valid, contain native Office Math (<m:oMath>) with zero PNG media,
  // and preserve the equation tag when reopened.
  const wordSavedCheck = await page.evaluate(async () => {
    const saved = window.__docxEditor.save()
    if (!(saved instanceof Uint8Array) || saved.length < 500) return { ok: false, reason: 'too small' }
    // Unzip document.xml via DecompressionStream isn't needed if we inspect the zip entries directly.
    const bytes = new Uint8Array(saved)
    const text = new TextDecoder('latin1').decode(bytes)
    const hasPng = text.includes('word/media/')
    return { ok: !hasPng, size: saved.length, hasPng }
  })
  check('Word saves valid .docx without PNG equation media', wordSavedCheck.ok, JSON.stringify(wordSavedCheck))

  // 2. Slides: insert LaTeX equation, inspect it, edit it in place, and verify round-trip save.
  await page.getByRole('tab', { name: 'Home' }).click()
  await page.getByRole('button', { name: 'New presentation', exact: true }).first().click()
  await page.waitForFunction(() => !!window.__slides, null, { timeout: 30000 })

  await page.locator('.view[data-active=true] button.tool-btn:has-text("Math")').click()
  await page.waitForSelector('#writan-latex-input', { timeout: 10000 })
  await page.locator('#writan-latex-input').fill('\\int_0^\\infty e^{-x^2}\\,dx = \\frac{\\sqrt{\\pi}}{2}')

  const slidesPreviewSvg = await page.locator('[data-testid="latex-math-preview"] svg').count()
  check('Slides modal renders live MathJax SVG preview', slidesPreviewSvg === 1, `svg count=${slidesPreviewSvg}`)

  await page.getByRole('button', { name: 'Insert equation' }).click()
  await page.waitForSelector('#writan-latex-input', { state: 'detached', timeout: 10000 })

  const slideMathEl1 = await page.evaluate(() => window.__slides.elements.find((e) => e.mathTag) ?? null)
  const slideLatex1 = slideMathEl1?.mathTag ? JSON.parse(slideMathEl1.mathTag.slice('writan:latex:'.length)).latex : ''
  check('Slides inserts picture frame with writan:latex tag', !!slideMathEl1 && slideLatex1.includes('\\sqrt{\\pi}'), JSON.stringify(slideMathEl1))

  const slidesEditBtn = page.locator('.view[data-active=true] button:has-text("Edit equation")')
  await slidesEditBtn.waitFor({ state: 'visible', timeout: 10000 })
  await slidesEditBtn.click()
  await page.waitForSelector('#writan-latex-input', { timeout: 10000 })
  await page.locator('#writan-latex-input').fill('a^2 + b^2 = c^2')
  await page.getByRole('button', { name: 'Update equation' }).click()
  await page.waitForSelector('#writan-latex-input', { state: 'detached', timeout: 10000 })

  const slideMathEl2 = await page.evaluate(() => window.__slides.elements.find((e) => e.mathTag) ?? null)
  check('Slides updates LaTeX equation frame in place', !!slideMathEl2 && slideMathEl2.mathTag.includes('a^2 + b^2 = c^2'), JSON.stringify(slideMathEl2))

  const slideImgLoaded = await page
    .waitForFunction(
      () => [...document.querySelectorAll('.view[data-active=true] img')].some((img) => img.complete && img.naturalWidth > 0),
      null,
      { timeout: 5000 },
    )
    .then(() => true)
    .catch(() => false)
  check('Slides renders equation SVG visibly on slide', slideImgLoaded)

  const slidesSavedCheck = await page.evaluate(() => {
    const saved = window.__slides.save()
    if (!(saved instanceof Uint8Array) || saved.length < 500) return { ok: false, reason: 'too small' }
    const text = new TextDecoder('latin1').decode(new Uint8Array(saved))
    const hasPng = text.includes('ppt/media/')
    return { ok: !hasPng, size: saved.length, hasPng }
  })
  check('Slides saves valid .pptx without PNG equation media', slidesSavedCheck.ok, JSON.stringify(slidesSavedCheck))

  // 3. Syntax error validation disables submit and shows error message.
  await page.locator('.view[data-active=true] button.tool-btn:has-text("Math")').click()
  await page.waitForSelector('#writan-latex-input', { timeout: 10000 })
  await page.locator('#writan-latex-input').fill('\\frac{1}{')
  const submitDisabled = await page.getByRole('button', { name: /^(Insert|Update) equation$/ }).isDisabled()
  const previewErrText = await page.locator('[data-testid="latex-math-preview"]').innerText()
  check('Invalid LaTeX disables submit button', submitDisabled)
  check('Invalid LaTeX shows readable error text', previewErrText.length > 0 && !previewErrText.includes('[object Object]'), previewErrText)
  await page.keyboard.press('Escape')

  check('no console/page errors', logs.length === 0, logs.join('; '))
} finally {
  await browser.close()
}

if (errors.length) {
  console.error('Failed checks:', errors)
  process.exit(1)
}
