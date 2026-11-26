// Warms up the document engine while the app is idle: compiles the WebAssembly and loads the
// fonts nearly every Office document uses, so opening the first one doesn't wait for them.

import { host, preloadFonts } from './host'
import { loadEngine } from './load'

export const BASE_FONTS = ['"Carlito"', '"Caladea"', '"Liberation Sans"', '"Liberation Serif"']

const yieldStep = () => new Promise<void>((r) => globalThis.setTimeout(r, 0))

export async function preloadEngine() {
  const engine = await loadEngine()
  await yieldStep()
  await preloadFonts(BASE_FONTS)
  await yieldStep()
  // A small document through reading, layout, drawing, typing and saving: the browser compiles
  // WebAssembly functions the first time they run, so the first real document doesn't pay for it.
  const doc = new engine.WordDoc(new Uint8Array(), host)
  try {
    doc.layout()
    await yieldStep()
    doc.command(JSON.stringify({ cmd: 'insertText', text: 'Warm up the engine.' }))
    doc.command(JSON.stringify({ cmd: 'enter' }))
    doc.layout()
    await yieldStep()
    doc.paint(0, true)
    doc.overlay()
    doc.state()
    await yieldStep()
    doc.save()
  } finally {
    doc.free()
  }
}

/**
 * Starts what opening an Office document waits for (the engine's WebAssembly, the base fonts) at
 * the same moment as its editor's code, rather than after it. Both are shared and loaded once.
 */
export function startOfficeOpen() {
  void loadEngine().catch(() => {})
  void preloadFonts(BASE_FONTS)
}
