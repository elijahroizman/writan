import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { ChartSvg } from '../engine/sheets/Chart'
import type { ChartView } from '../engine/sheets/engine'

const DEFAULT_WIDTH = 600
const DEFAULT_HEIGHT = 360

/** Renders an Excel ChartView to a 2x PNG byte array for embedding in Word or Slides. */
export async function renderChartToPng(
  chart: ChartView,
  width = Math.round(chart.w) || DEFAULT_WIDTH,
  height = Math.round(chart.h) || DEFAULT_HEIGHT,
): Promise<{ bytes: Uint8Array; width: number; height: number }> {
  const w = Math.max(240, Math.min(1200, width))
  const h = Math.max(160, Math.min(800, height))
  const markup = renderToStaticMarkup(createElement(ChartSvg, { chart, width: w, height: h }))
  const start = markup.indexOf('<svg')
  const end = markup.lastIndexOf('</svg>')
  if (start === -1 || end === -1) throw new Error('Failed to render chart SVG.')
  let svg = markup.slice(start, end + '</svg>'.length)
  if (!svg.includes('xmlns=')) {
    svg = svg.replace('<svg', '<svg xmlns="http://www.w3.org/2000/svg"')
  }
  const scale = 2
  const blob = new Blob([svg], { type: 'image/svg+xml;charset=utf-8' })
  const url = URL.createObjectURL(blob)
  try {
    const img = await new Promise<HTMLImageElement>((resolve, reject) => {
      const el = new Image()
      el.onload = () => resolve(el)
      el.onerror = () => reject(new Error('Failed to render chart image.'))
      el.src = url
    })
    const canvas = document.createElement('canvas')
    canvas.width = w * scale
    canvas.height = h * scale
    const ctx = canvas.getContext('2d')
    if (!ctx) throw new Error('Canvas 2D context unavailable.')
    ctx.fillStyle = '#ffffff'
    ctx.fillRect(0, 0, canvas.width, canvas.height)
    ctx.scale(scale, scale)
    ctx.drawImage(img, 0, 0, w, h)
    const pngBlob = await new Promise<Blob>((resolve, reject) => {
      canvas.toBlob((b) => (b ? resolve(b) : reject(new Error('Failed to encode chart PNG.'))), 'image/png')
    })
    return { bytes: new Uint8Array(await pngBlob.arrayBuffer()), width: w, height: h }
  } finally {
    URL.revokeObjectURL(url)
  }
}
