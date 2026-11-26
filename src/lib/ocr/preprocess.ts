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
  skewAngle: number
  mapPoint: MapPoint
  mapBox: (bbox: OcrBBox) => OcrBBox
}

export function grayscale(data: Uint8Array | Uint8ClampedArray, width: number, height: number, channels?: 1 | 4): GrayImage {
  const pixelCount = width * height
  if (!Number.isSafeInteger(width) || !Number.isSafeInteger(height) || width < 1 || height < 1) {
    throw new Error('OCR requires a valid RGBA image.')
  }
  if ((channels === 1 || data.length === pixelCount) && channels !== 4) {
    if (data.length !== pixelCount) throw new Error('OCR requires a valid grayscale image.')
    const pixels = data instanceof Uint8Array && data.constructor === Uint8Array ? data : new Uint8Array(data)
    return { pixels, width, height }
  }
  if (data.length !== pixelCount * 4) {
    throw new Error('OCR requires a valid RGBA image.')
  }
  const pixels = new Uint8Array(pixelCount)
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

export function upscale2x(image: GrayImage): GrayImage {
  const srcW = image.width
  const srcH = image.height
  const src = image.pixels
  const width = srcW * 2
  const height = srcH * 2
  const pixels = new Uint8Array(width * height)
  const maxX = srcW - 1
  const maxY = srcH - 1
  for (let y = 0; y < height; y++) {
    let sy = (y + 0.5) / 2 - 0.5
    if (sy < 0) sy = 0
    else if (sy > maxY) sy = maxY
    const top = Math.floor(sy)
    const bottom = top + 1 <= maxY ? top + 1 : maxY
    const dy = sy - top
    const invDy = 1 - dy
    const topRow = top * srcW
    const botRow = bottom * srcW
    const dstRow = y * width
    for (let x = 0; x < width; x++) {
      let sx = (x + 0.5) / 2 - 0.5
      if (sx < 0) sx = 0
      else if (sx > maxX) sx = maxX
      const left = Math.floor(sx)
      const right = left + 1 <= maxX ? left + 1 : maxX
      const dx = sx - left
      const upper = src[topRow + left] * (1 - dx) + src[topRow + right] * dx
      const lower = src[botRow + left] * (1 - dx) + src[botRow + right] * dx
      pixels[dstRow + x] = Math.round(upper * invDy + lower * dy)
    }
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
  const src = image.pixels
  const pixels = new Uint8Array(outWidth * outHeight)
  if (rotation === 90) {
    for (let y = 0; y < outHeight; y++) {
      const sy = Math.floor(height - (y + 0.5)) * width
      const dstRow = y * outWidth
      for (let x = 0; x < outWidth; x++) pixels[dstRow + x] = src[sy + x]
    }
  } else if (rotation === 180) {
    for (let y = 0; y < outHeight; y++) {
      const sy = Math.floor(height - (y + 0.5)) * width
      const dstRow = y * outWidth
      for (let x = 0; x < outWidth; x++) pixels[dstRow + x] = src[sy + Math.floor(width - (x + 0.5))]
    }
  } else {
    for (let y = 0; y < outHeight; y++) {
      const dstRow = y * outWidth
      for (let x = 0; x < outWidth; x++) pixels[dstRow + x] = src[x * width + Math.floor(width - (y + 0.5))]
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
  const srcW = image.width
  const srcH = image.height
  const srcPixels = image.pixels
  const width = Math.ceil(srcW * Math.abs(cos) + srcH * Math.abs(sin))
  const height = Math.ceil(srcW * Math.abs(sin) + srcH * Math.abs(cos))
  const halfW = width / 2
  const halfH = height / 2
  const srcHalfW = srcW / 2
  const srcHalfH = srcH / 2
  const maxX = srcW - 1
  const maxY = srcH - 1
  const inverse: MapPoint = (x, y) => ({
    x: (x - halfW) * cos + (y - halfH) * sin + srcHalfW,
    y: (y - halfH) * cos - (x - halfW) * sin + srcHalfH,
  })
  const pixels = new Uint8Array(width * height)
  for (let y = 0; y < height; y++) {
    const dy = y + 0.5 - halfH
    const baseSx = dy * sin + srcHalfW
    const baseSy = dy * cos + srcHalfH
    const dstRow = y * width
    for (let x = 0; x < width; x++) {
      const dx = x + 0.5 - halfW
      const sxRaw = dx * cos + baseSx
      const syRaw = baseSy - dx * sin
      if (sxRaw < 0 || sxRaw >= srcW || syRaw < 0 || syRaw >= srcH) {
        pixels[dstRow + x] = 255
      } else {
        let sx = sxRaw - 0.5
        let sy = syRaw - 0.5
        if (sx < 0) sx = 0
        else if (sx > maxX) sx = maxX
        if (sy < 0) sy = 0
        else if (sy > maxY) sy = maxY
        const left = Math.floor(sx)
        const top = Math.floor(sy)
        const right = left + 1 <= maxX ? left + 1 : maxX
        const bottom = top + 1 <= maxY ? top + 1 : maxY
        const fx = sx - left
        const fy = sy - top
        const upper = srcPixels[top * srcW + left] * (1 - fx) + srcPixels[top * srcW + right] * fx
        const lower = srcPixels[bottom * srcW + left] * (1 - fx) + srcPixels[bottom * srcW + right] * fx
        pixels[dstRow + x] = Math.round(upper * (1 - fy) + lower * fy)
      }
    }
  }
  return { pixels, width, height, inverse }
}

export function prepareOrientation(image: GrayImage, rotation: OcrRotation, knownSkew?: number): PreparedImage {
  const oriented = rotateQuarter(image, rotation)
  const scale = oriented.height < 900 ? 2 : 1
  const scaled = scale === 2 ? upscale2x(oriented) : oriented
  const skewAngle = knownSkew ?? detectDeskew(scaled)
  const skew = rotateDeskew(scaled, skewAngle)
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
  return { pixels: skew.pixels, width: skew.width, height: skew.height, scale, skewAngle, mapPoint, mapBox }
}

export function preprocessImage(
  data: Uint8Array | Uint8ClampedArray, width: number, height: number, rotation: OcrRotation = 0,
): PreparedImage {
  return prepareOrientation(enhanceContrast(grayscale(data, width, height)), rotation)
}
