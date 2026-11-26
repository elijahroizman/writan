/** Clear `#print-root` after printing finishes so print-only bitmaps and clones don't linger in the DOM. */
export function schedulePrintCleanup(extra?: () => void) {
  const clean = () => {
    const root = document.getElementById('print-root')
    if (root) {
      for (const img of root.querySelectorAll('img')) {
        if (img.src.startsWith('blob:')) URL.revokeObjectURL(img.src)
      }
      root.innerHTML = ''
    }
    extra?.()
  }
  window.addEventListener('afterprint', clean, { once: true })
}
