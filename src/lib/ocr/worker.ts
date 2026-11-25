import simdGlueUrl from 'tesseract.js-core/tesseract-core-simd-lstm.wasm.js?url'
import simdWasmUrl from 'tesseract.js-core/tesseract-core-simd-lstm.wasm?url'
import scalarGlueUrl from 'tesseract.js-core/tesseract-core-lstm.wasm.js?url'
import scalarWasmUrl from 'tesseract.js-core/tesseract-core-lstm.wasm?url'
import { getPackMeta } from './catalog'
import { enhanceContrast, grayscale, prepareOrientation } from './preprocess'
import type { PreparedImage } from './preprocess'
import type { OcrBlock, OcrLine, OcrPageInput, OcrPageResult, OcrParagraph, OcrRotation } from './types'

export type OcrWorkerCall =
  | { kind: 'initialize'; packs: { code: string; bytes: Uint8Array }[] }
  | { kind: 'recognize'; page: OcrPageInput }

type OcrWorkerRequest = { id: number } & OcrWorkerCall
export type OcrWorkerReply =
  | { id: number; kind: 'ready' }
  | { id: number; kind: 'result'; page: OcrPageResult }
  | { id: number; kind: 'error'; message: string }

interface IteratorBox {
  x0: number
  y0: number
  x1: number
  y1: number
}

interface TessIterator {
  Empty(level: number): boolean
  IsAtBeginningOf(level: number): boolean
  Next(level: number): boolean
  Orientation(orientation: number, writingDirection: number, lineOrder: number, deskew: number): void
  Confidence(level: number): number
  getBoundingBox(level: number): IteratorBox
  getBaseline(level: number): IteratorBox & { has_baseline: boolean }
  getRowAttributes(): { row_height: number }
  getWordFontAttributes(): { pointsize: number }
}

interface TessApi {
  Init(path: string, langs: string, oem: number): number
  SetPageSegMode(mode: number): void
  SetImage(address: number, width: number, height: number, depth: number, stride: number): void
  SetSourceResolution(ppi: number): void
  Recognize(monitor: number): number
  GetIterator(): TessIterator
  GetUTF8Text(): string | null
  MeanTextConf(): number
  Clear(): void
}

interface TessCore {
  FS: {
    mkdir(path: string): void
    writeFile(path: string, bytes: Uint8Array): void
  }
  HEAPU8: Uint8Array
  _malloc(size: number): number
  _free(address: number): void
  TessBaseAPI: new () => TessApi
  getPointer(iterator: TessIterator): number
  _emscripten_bind_ResultIterator_GetUTF8Text_1(iterator: number, level: number): number
  destroy(object: TessApi | TessIterator): void
  OEM_LSTM_ONLY: number
  PSM_AUTO: number
  RIL_BLOCK: number
  RIL_PARA: number
  RIL_TEXTLINE: number
  RIL_WORD: number
  WRITING_DIRECTION_TOP_TO_BOTTOM: number
  pointerHelper: new () => {
    i32(name: string): number
    f32(name: string): number
    get(): { orientation: number; writingDirection: number }
  }
}

type CreateCore = (options: { locateFile: (name: string) => string; wasmBinary: ArrayBuffer }) => Promise<TessCore>

const worker = self as unknown as {
  onmessage: ((event: MessageEvent<OcrWorkerRequest>) => void) | null
  postMessage(reply: OcrWorkerReply): void
  exports?: { TesseractCore?: CreateCore }
}

// A minimal WASM module with a v128 instruction tests SIMD without loading the engine.
const simdProbe = new Uint8Array([
  0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 123, 3, 2, 1, 0,
  10, 10, 1, 8, 0, 65, 0, 253, 15, 253, 98, 11,
])

async function loadCore(glueUrl: string, wasmUrl: string): Promise<TessCore> {
  const response = await fetch(wasmUrl)
  if (!response.ok) throw new Error('Could not load Writan’s bundled OCR engine.')
  const wasmBinary = await response.arrayBuffer()
  // Emscripten's classic glue supports an exports object. Capture its factory
  // via that branch when evaluating the same bundled script in our module worker.
  worker.exports = {}
  try {
    await import(/* @vite-ignore */ glueUrl)
    const createCore = worker.exports.TesseractCore
    if (!createCore) throw new Error('Could not start Writan’s bundled OCR engine.')
    return await createCore({ locateFile: () => wasmUrl, wasmBinary })
  } finally {
    delete worker.exports
  }
}

async function createEngine(packs: { code: string; bytes: Uint8Array }[]) {
  let core: TessCore | undefined
  if (WebAssembly.validate(simdProbe)) {
    try {
      core = await loadCore(simdGlueUrl, simdWasmUrl)
    } catch {
      // Some webviews advertise SIMD but fail when instantiating the full module.
    }
  }
  core ??= await loadCore(scalarGlueUrl, scalarWasmUrl)
  core.FS.mkdir('/tessdata')
  for (const { code, bytes } of packs) {
    if (!getPackMeta(code)) throw new Error('Unknown OCR language pack.')
    core.FS.writeFile('/tessdata/' + code + '.traineddata', bytes)
  }
  const api = new core.TessBaseAPI()
  if (api.Init('/tessdata', packs.map(({ code }) => code).join('+'), core.OEM_LSTM_ONLY) !== 0) {
    core.destroy(api)
    throw new Error('Could not load the selected OCR languages. Try downloading their packs again.')
  }
  api.SetPageSegMode(core.PSM_AUTO)
  return { core, api }
}

function hasRtl(text: string): boolean {
  return /[\p{Script=Hebrew}\p{Script=Arabic}]/u.test(text)
}

function confidence(value: number): number {
  return Number.isFinite(value) ? Math.max(0, Math.min(100, value)) : 0
}

const decoder = new TextDecoder()

function extract(core: TessCore, iterator: TessIterator, image: PreparedImage, resolution: number) {
  const blocks: OcrBlock[] = []
  const orientationCounts = [0, 0, 0, 0]
  const blockLevel = core.RIL_BLOCK
  const paragraphLevel = core.RIL_PARA
  const lineLevel = core.RIL_TEXTLINE
  const wordLevel = core.RIL_WORD
  const box = ({ x0, y0, x1, y1 }: IteratorBox) => image.mapBox({ x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
  let block: OcrBlock | undefined
  let paragraph: OcrParagraph | undefined
  let line: OcrLine | undefined
  let blockOrientation = 0
  let sidewaysOrientation = 0
  const iteratorAddress = core.getPointer(iterator)

  function readText(level: number): string {
    // The generated iterator wrapper decodes the allocated string without
    // releasing it. Read the raw pointer and free it after decoding.
    const pointer = core._emscripten_bind_ResultIterator_GetUTF8Text_1(iteratorAddress, level)
    if (!pointer) return ''
    try {
      const end = core.HEAPU8.indexOf(0, pointer)
      return decoder.decode(core.HEAPU8.subarray(pointer, end)).trim()
    } finally {
      core._free(pointer)
    }
  }

  function baselineRaw(level: number, bounds: IteratorBox): number | undefined {
    const { y0, y1, has_baseline } = iterator.getBaseline(level)
    if (!has_baseline) return
    return Math.max(bounds.y0, Math.min(bounds.y1, (y0 + y1) / 2))
  }

  function baseline(level: number, bounds: IteratorBox): number | undefined {
    const raw = baselineRaw(level, bounds)
    if (raw === undefined) return
    return image.mapPoint((bounds.x0 + bounds.x1) / 2, raw).y
  }

  do {
    if (iterator.IsAtBeginningOf(blockLevel)) {
      block = undefined
      paragraph = undefined
      line = undefined
    }
    if (iterator.IsAtBeginningOf(paragraphLevel)) {
      paragraph = undefined
      line = undefined
    }
    if (iterator.IsAtBeginningOf(lineLevel)) line = undefined
    if (iterator.Empty(wordLevel)) continue
    const text = readText(wordLevel)
    if (!text) continue
    if (!block) {
      block = { bbox: box(iterator.getBoundingBox(blockLevel)), confidence: confidence(iterator.Confidence(blockLevel)), paragraphs: [] }
      blocks.push(block)
      const output = new core.pointerHelper()
      iterator.Orientation(output.i32('orientation'), output.i32('writingDirection'), output.i32('lineOrder'), output.f32('deskew'))
      const { orientation, writingDirection } = output.get()
      blockOrientation = orientation
      sidewaysOrientation = 0
      if (writingDirection === core.WRITING_DIRECTION_TOP_TO_BOTTOM) {
        const { x0, y0, x1, y1, has_baseline } = iterator.getBaseline(lineLevel)
        if (has_baseline && Math.abs(y1 - y0) > Math.abs(x1 - x0)) sidewaysOrientation = y1 > y0 ? 1 : 3
      }
    }
    if (!paragraph) {
      const text = readText(paragraphLevel)
      paragraph = { text, bbox: box(iterator.getBoundingBox(paragraphLevel)), confidence: confidence(iterator.Confidence(paragraphLevel)), rtl: hasRtl(text), lines: [] }
      block.paragraphs.push(paragraph)
    }
    if (!line) {
      const text = readText(lineLevel)
      const lineBounds = iterator.getBoundingBox(lineLevel)
      line = { text, bbox: box(lineBounds), baseline: baseline(lineLevel, lineBounds), confidence: confidence(iterator.Confidence(lineLevel)), rtl: hasRtl(text), words: [] }
      paragraph.lines.push(line)
    }
    const bounds = iterator.getBoundingBox(wordLevel)
    const { pointsize } = iterator.getWordFontAttributes()
    const fontSize = pointsize > 0 ? pointsize * resolution / 72 : iterator.getRowAttributes().row_height
    // Automatic segmentation can label sideways Latin or RTL text as vertical.
    // For those scripts the direction of the baseline reveals the quarter-turn.
    const horizontalScript = /[\p{Script=Latin}\p{Script=Cyrillic}\p{Script=Hebrew}\p{Script=Arabic}]/u.test(text)
    orientationCounts[(horizontalScript && sidewaysOrientation) || blockOrientation]++
    const wordBlRaw = baselineRaw(wordLevel, bounds) ?? baselineRaw(lineLevel, bounds) ?? (bounds.y0 + (bounds.y1 - bounds.y0) * 0.8)
    const blStart = image.mapPoint(bounds.x0, wordBlRaw)
    const blEnd = image.mapPoint(bounds.x1, wordBlRaw)
    const boxTop = image.mapPoint(bounds.x0, bounds.y0)
    const boxBot = image.mapPoint(bounds.x0, bounds.y1)
    line.words.push({
      text,
      bbox: box(bounds),
      baseline: (blStart.y + blEnd.y) / 2,
      baselineLine: { x0: blStart.x, y0: blStart.y, x1: blEnd.x, y1: blEnd.y },
      orientedHeightPx: Math.max(1, Math.hypot(boxBot.x - boxTop.x, boxBot.y - boxTop.y)),
      confidence: confidence(iterator.Confidence(wordLevel)),
      rtl: hasRtl(text),
      fontSizePx: Math.max(1, (fontSize || bounds.y1 - bounds.y0) / image.scale),
    })
  } while (iterator.Next(wordLevel))

  const orientation = orientationCounts.indexOf(Math.max(...orientationCounts))
  return { blocks, orientation }
}

interface Candidate {
  text: string
  blocks: OcrBlock[]
  meanConfidence: number
  rotation: OcrRotation
}

type Engine = Awaited<ReturnType<typeof createEngine>>

function recognizePage({ core, api }: Engine, page: OcrPageInput): OcrPageResult {
  const start = performance.now()
  const base = enhanceContrast(grayscale(page.data, page.width, page.height))
  let best: Candidate | undefined
  for (const rotation of [0, 90, 180, 270] as const) {
    const image = prepareOrientation(base, rotation)
    const resolution = Math.round((page.heightPt && page.heightPt > 0 ? page.height * 72 / page.heightPt : 96) * image.scale)
    const address = core._malloc(image.pixels.length)
    if (!address) throw new Error('There is not enough memory to recognize this page.')
    try {
      core.HEAPU8.set(image.pixels, address)
      api.SetImage(address, image.width, image.height, 1, image.width)
      api.SetSourceResolution(resolution)
      if (api.Recognize(0) !== 0) throw new Error('The OCR engine could not recognize this page.')
      const meanConfidence = confidence(api.MeanTextConf())
      if (best && meanConfidence <= best.meanConfidence) continue
      const iterator = api.GetIterator()
      if (!core.getPointer(iterator)) continue
      try {
        const { blocks, orientation } = extract(core, iterator, image, resolution)
        if (blocks.length) {
          const correction = ((rotation + 360 - orientation * 90) % 360) as OcrRotation
          best = { blocks, meanConfidence, rotation: correction, text: api.GetUTF8Text()?.trim() ?? '' }
        }
      } finally {
        core.destroy(iterator)
      }
    } finally {
      api.Clear()
      core._free(address)
    }
    if (rotation === 0 && best && best.meanConfidence >= 45) break
  }
  return {
    pageIndex: page.pageIndex ?? 0,
    widthPx: page.width,
    heightPx: page.height,
    widthPt: page.widthPt,
    heightPt: page.heightPt,
    text: best?.text ?? '',
    meanConfidence: best?.meanConfidence ?? 0,
    detectedRotation: best?.rotation ?? 0,
    elapsedMs: performance.now() - start,
    blocks: best?.blocks ?? [],
  }
}

let engine: Engine | undefined
worker.onmessage = async ({ data: request }) => {
  try {
    if (request.kind === 'initialize') {
      if (engine) throw new Error('OCR worker is already initialized.')
      engine = await createEngine(request.packs)
      worker.postMessage({ id: request.id, kind: 'ready' })
    } else {
      if (!engine) throw new Error('OCR worker is not initialized.')
      worker.postMessage({ id: request.id, kind: 'result', page: recognizePage(engine, request.page) })
    }
  } catch (error) {
    worker.postMessage({ id: request.id, kind: 'error', message: error instanceof Error ? error.message : String(error) })
  }
}
