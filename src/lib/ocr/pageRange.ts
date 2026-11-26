/** Parses 1-based page numbers and ranges (for example `"2, 5"` or `"1-3, 5"`) into sorted 0-based page indices. */
export function parsePageSelection(input: string, totalPages: number): { indices: number[]; error: string | null } {
  if (totalPages <= 0) {
    return { indices: [], error: 'This PDF has no pages.' }
  }
  const trimmed = input.trim()
  if (!trimmed) {
    return { indices: [], error: 'Enter page numbers such as 2, 5 or 1-3.' }
  }
  const parts = trimmed.split(',')
  const selected = new Set<number>()
  for (const raw of parts) {
    const part = raw.trim()
    if (!part) {
      return { indices: [], error: 'Invalid page selection. Separate pages or ranges with commas.' }
    }
    const single = /^(\d+)$/.exec(part)
    if (single) {
      const page = Number(single[1])
      if (!Number.isSafeInteger(page) || page < 1 || page > totalPages) {
        return { indices: [], error: `Page ${single[1]} is out of range (1–${totalPages}).` }
      }
      selected.add(page - 1)
      continue
    }
    const range = /^(\d+)\s*[-–]\s*(\d+)$/.exec(part)
    if (!range) {
      return { indices: [], error: `Invalid page range "${part}". Use numbers or ranges like 2, 5 or 1-3.` }
    }
    const start = Number(range[1])
    const end = Number(range[2])
    if (!Number.isSafeInteger(start) || start < 1 || start > totalPages) {
      return { indices: [], error: `Page ${range[1]} is out of range (1–${totalPages}).` }
    }
    if (!Number.isSafeInteger(end) || end < 1 || end > totalPages) {
      return { indices: [], error: `Page ${range[2]} is out of range (1–${totalPages}).` }
    }
    if (start > end) {
      return { indices: [], error: `Range ${start}-${end} is reversed. Use smaller-to-larger page numbers.` }
    }
    for (let page = start; page <= end; page++) {
      selected.add(page - 1)
    }
  }
  return { indices: [...selected].sort((a, b) => a - b), error: null }
}

/** Formats 0-based page indices into a compact 1-based range string (for example `[0, 1, 2, 4]` -> `"1-3, 5"`). */
export function formatPageSelection(indices: readonly number[]): string {
  const unique = [...new Set(indices)].filter((n) => Number.isInteger(n) && n >= 0).sort((a, b) => a - b)
  if (!unique.length) return ''
  const parts: string[] = []
  let start = unique[0]
  let prev = unique[0]
  for (let i = 1; i <= unique.length; i++) {
    const cur = unique[i]
    if (cur !== undefined && cur === prev + 1) {
      prev = cur
      continue
    }
    parts.push(start === prev ? `${start + 1}` : `${start + 1}-${prev + 1}`)
    if (cur !== undefined) {
      start = cur
      prev = cur
    }
  }
  return parts.join(', ')
}
