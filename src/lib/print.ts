/** Clear `#print-root` after printing finishes so print-only clones don't linger in the DOM. Its
 * pictures' blob URLs belong to the editors' media caches, which revoke them when they close. */
export function schedulePrintCleanup(extra?: () => void) {
  const clean = () => {
    const root = document.getElementById('print-root')
    if (root) root.innerHTML = ''
    extra?.()
  }
  window.addEventListener('afterprint', clean, { once: true })
}
