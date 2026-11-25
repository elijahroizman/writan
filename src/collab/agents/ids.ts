// A Word paragraph's id, for agents to name it by (word.ts) and people to find it from the chat
// (AgentChat.tsx).
//
// A paragraph's id comes from its text, so every copy of the document gives it the same one with
// nothing to agree on, and it stays while others add or delete paragraphs around it. Paragraphs
// with the same text take the text of the paragraph before them into theirs too, so a new one
// never takes over an old one's id; only when that's the same as well does one get "-2". An empty
// paragraph has none. When a paragraph's text
// changes its id does too, and each agent's copy remembers what the old one became (`History` in
// word.ts), so an id an agent read or saw in the chat still finds its paragraph.

/** Six characters from a paragraph's text, starting with a letter (FNV-1a). */
function hashOf(text: string): string {
  let h = 0x811c9dc5
  for (let i = 0; i < text.length; i++) {
    h ^= text.charCodeAt(i)
    h = Math.imul(h, 0x01000193)
  }
  h >>>= 0
  return String.fromCharCode(97 + (h % 26)) + (Math.floor(h / 26) % 36 ** 5).toString(36).padStart(5, '0')
}

/** Each paragraph's id, from the texts of all of them in order; null for an empty one. */
export function idsOf(texts: string[]): (string | null)[] {
  const many = new Map<string, number>()
  for (const t of texts) if (t.trim()) many.set(t, (many.get(t) ?? 0) + 1)
  const seen = new Map<string, number>()
  let before = ''
  return texts.map((t) => {
    if (!t.trim()) return null
    // With the text of the last paragraph before it that isn't empty, when the text alone isn't
    // enough to tell it apart.
    const base = many.get(t)! > 1 ? hashOf(`${before}\n${t}`) : hashOf(t)
    before = t
    const n = (seen.get(base) ?? 0) + 1
    seen.set(base, n)
    return n === 1 ? `#${base}` : `#${base}-${n}`
  })
}

/** An id as an agent may write it: "#k3f9x2", "k3f9x2", "#k3f9x2-2". */
export const ID = /^#?([a-z][0-9a-z]{5}(?:-\d+)?)$/

/** Ids in a message. */
export const IDS_IN = /#[a-z][0-9a-z]{5}(?:-\d+)?(?![0-9a-z-])/g
