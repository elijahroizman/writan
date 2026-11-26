import type { OcrBBox, OcrRotation } from './types'

export interface GrayImage {
  pixels: Uint8Array
  width: number
  height: number
}

interface Point {
  x: number
  y: number
}

type MapPoint = (x: number, y: number) => Point

export interface PreparedImage extends GrayImage {
  scale: number
  mapPoint: MapPoint
  mapBox: (bbox: OcrBBox) => OcrBBox
}

export function grayscale(data: Uint8Array | Uint8ClampedArray, width: number, height: number): GrayImage {
  if (!Number.isSafeInteger(width) || !Number.isSafeInteger(height) || width < 1 || height < 1 ||
      data.length !== width * height * 4) {
    throw new Error('OCR requires a valid RGBA image.')
  }
  const pixels = new Uint8Array(width * height)
  for (let i = 0, j = 0; i < pixels.length; i++, j += 4) {
    const luminance = (77 * data[j] + 150 * data[j + 1] + 29 * data[j + 2]) / 256
    pixels[i] = Math.round(255 + (luminance - 255) * data[j + 3] / 255)
  }
  return { pixels, width, height }
}

function byte(value: number): number {
  return Math.min(255, Math.max(0, Math.round(value)))
}

export function enhanceContrast({ pixels, width, height }: GrayImage): GrayImage {
  const radius = Math.max(4, Math.min(32, Math.round(height / 60)))
  const rowMeans = new Uint8Array(pixels.length)
  for (let y = 0; y < height; y++) {
    const start = y * width
    let sum = 0
    for (let x = 0; x < Math.min(width, radius + 1); x++) sum += pixels[start + x]
    for (let x = 0; x < width; x++) {
      const count = Math.min(x + radius + 1, width) - Math.max(x - radius, 0)
      rowMeans[start + x] = Math.round(sum / count)
      if (x >= radius) sum -= pixels[start + x - radius]
      if (x + radius + 1 < width) sum += pixels[start + x + radius + 1]
    }
  }

  const output = new Uint8Array(pixels.length)
  const sums = new Uint32Array(width)
  for (let y = 0; y < Math.min(height, radius + 1); y++) {
    for (let x = 0; x < width; x++) sums[x] += rowMeans[y * width + x]
  }
  for (let y = 0; y < height; y++) {
    const row = y * width
    const count = Math.min(y + radius + 1, height) - Math.max(y - radius, 0)
    const prev = Math.max(0, y - 1) * width
    const next = Math.min(height - 1, y + 1) * width
    for (let x = 0; x < width; x++) {
      const index = row + x
      const localMean = sums[x] / count
      const pixel = pixels[index]
      const neighbors = (pixels[row + Math.max(0, x - 1)] + pixels[row + Math.min(width - 1, x + 1)] +
        pixels[prev + x] + pixels[next + x]) / 4
      const backgroundLift = 0.6 * (255 - localMean) * pixel / 255
      output[index] = byte(pixel + backgroundLift + 0.45 * (pixel - localMean) + 0.22 * (pixel - neighbors))
      if (y >= radius) sums[x] -= rowMeans[(y - radius) * width + x]
      if (y + radius + 1 < height) sums[x] += rowMeans[(y + radius + 1) * width + x]
    }
  }
  return { pixels: output, width, height }
}

function bilinear(image: GrayImage, x: number, y: number): number {
  x = Math.max(0, Math.min(image.width - 1, x))
  y = Math.max(0, Math.min(image.height - 1, y))
  const left = Math.floor(x)
  const top = Math.floor(y)
  const right = Math.min(left + 1, image.width - 1)
  const bottom = Math.min(top + 1, image.height - 1)
  const dx = x - left
  const dy = y - top
  const { pixels, width } = image
  const upper = pixels[top * width + left] * (1 - dx) + pixels[top * width + right] * dx
  const lower = pixels[bottom * width + left] * (1 - dx) + pixels[bottom * width + right] * dx
  return Math.round(upper * (1 - dy) + lower * dy)
}

export function upscale2x(image: GrayImage): GrayImage {
  const width = image.width * 2
  const height = image.height * 2
  const pixels = new Uint8Array(width * height)
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) pixels[y * width + x] = bilinear(image, (x + 0.5) / 2 - 0.5, (y + 0.5) / 2 - 0.5)
  }
  return { pixels, width, height }
}

/** Clockwise rotation. Inverse maps pixel edges, including the far image edges. */
export function rotateQuarter(image: GrayImage, rotation: OcrRotation): GrayImage & { inverse: MapPoint } {
  const { width, height } = image
  if (rotation === 0) return { ...image, inverse: (x, y) => ({ x, y }) }
  const outWidth = rotation === 180 ? width : height
  const outHeight = rotation === 180 ? height : width
  const inverse: MapPoint = rotation === 90 ? (x, y) => ({ x: y, y: height - x })
    : rotation === 180 ? (x, y) => ({ x: width - x, y: height - y })
    : (x, y) => ({ x: width - y, y: x })
  const pixels = new Uint8Array(outWidth * outHeight)
  for (let y = 0; y < outHeight; y++) {
    for (let x = 0; x < outWidth; x++) {
      const src = inverse(x + 0.5, y + 0.5)
      pixels[y * outWidth + x] = image.pixels[Math.floor(src.y) * width + Math.floor(src.x)]
    }
  }
  return { pixels, width: outWidth, height: outHeight, inverse }
}

/** Return the clockwise correction that makes rows of ink most distinct. */
export function detectDeskew({ pixels, width, height }: GrayImage): number {
  const step = Math.max(1, Math.ceil(Math.max(width, height) / 500))
  const marginX = Math.floor(width * 0.04)
  const marginY = Math.floor(height * 0.04)
  const xs: number[] = []
  const ys: number[] = []
  const weights: number[] = []
  for (let y = marginY; y < height - marginY; y += step) {
    for (let x = marginX; x < width - marginX; x += step) {
      const ink = 230 - pixels[y * width + x]
      if (ink < 12) continue
      xs.push((x - width / 2) / step)
      ys.push((y - height / 2) / step)
      weights.push(ink)
    }
  }
  if (weights.length < 40) return 0

  const extent = Math.ceil((width + height) / (2 * step)) + 2
  const rows = new Float64Array(extent * 2)
  function score(degrees: number) {
    rows.fill(0)
    const radians = degrees * Math.PI / 180
    const sin = Math.sin(radians)
    const cos = Math.cos(radians)
    for (let i = 0; i < weights.length; i++) {
      const y = extent + xs[i] * sin + ys[i] * cos
      const row = Math.floor(y)
      const fraction = y - row
      rows[row] += weights[i] * (1 - fraction)
      rows[row + 1] += weights[i] * fraction
    }
    let result = 0
    for (const row of rows) result += row * row
    return result
  }

  const upright = score(0)
  let best = upright
  let angle = 0
  function consider(candidate: number) {
    const value = score(candidate)
    if (value > best) {
      angle = candidate
      best = value
    }
  }
  for (let candidate = -15; candidate <= 15; candidate += 2.5) consider(candidate)
  const coarse = angle
  for (let candidate = Math.max(-15, coarse - 2); candidate <= Math.min(15, coarse + 2); candidate += 0.5) {
    consider(candidate)
  }
  return best > upright * 1.04 ? angle : 0
}

export function rotateDeskew(image: GrayImage, degrees: number): GrayImage & { inverse: MapPoint } {
  if (!degrees) return { ...image, inverse: (x, y) => ({ x, y }) }
  const radians = degrees * Math.PI / 180
  const cos = Math.cos(radians)
  const sin = Math.sin(radians)
  const width = Math.ceil(image.width * Math.abs(cos) + image.height * Math.abs(sin))
  const height = Math.ceil(image.width * Math.abs(sin) + image.height * Math.abs(cos))
  const inverse: MapPoint = (x, y) => ({
    x: (x - width / 2) * cos + (y - height / 2) * sin + image.width / 2,
    y: (y - height / 2) * cos - (x - width / 2) * sin + image.height / 2,
  })
  const pixels = new Uint8Array(width * height)
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const src = inverse(x + 0.5, y + 0.5)
      pixels[y * width + x] = src.x < 0 || src.x >= image.width || src.y < 0 || src.y >= image.height
        ? 255 : bilinear(image, src.x - 0.5, src.y - 0.5)
    }
  }
  return { pixels, width, height, inverse }
}

export function prepareOrientation(image: GrayImage, rotation: OcrRotation): PreparedImage {
  const oriented = rotateQuarter(image, rotation)
  const scale = oriented.height < 900 ? 2 : 1
  const scaled = scale === 2 ? upscale2x(oriented) : oriented
  const skew = rotateDeskew(scaled, detectDeskew(scaled))
  const mapPoint: MapPoint = (x, y) => {
    const pt = skew.inverse(x, y)
    return oriented.inverse(pt.x / scale, pt.y / scale)
  }
  function mapBox({ x, y, w, h }: OcrBBox): OcrBBox {
    const corners = [mapPoint(x, y), mapPoint(x + w, y), mapPoint(x, y + h), mapPoint(x + w, y + h)]
    const clipX = (n: number) => Math.max(0, Math.min(image.width, n))
    const clipY = (n: number) => Math.max(0, Math.min(image.height, n))
    const left = clipX(Math.min(...corners.map((pt) => pt.x)))
    const right = clipX(Math.max(...corners.map((pt) => pt.x)))
    const top = clipY(Math.min(...corners.map((pt) => pt.y)))
    const bottom = clipY(Math.max(...corners.map((pt) => pt.y)))
    return { x: left, y: top, w: right - left, h: bottom - top }
  }
  return { pixels: skew.pixels, width: skew.width, height: skew.height, scale, mapPoint, mapBox }
}

export function preprocessImage(
  data: Uint8Array | Uint8ClampedArray, width: number, height: number, rotation: OcrRotation = 0,
): PreparedImage {
  return prepareOrientation(enhanceContrast(grayscale(data, width, height)), rotation)
}
