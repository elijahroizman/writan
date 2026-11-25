// A Word paragraph's id, for agents to name it by (word.ts) and people to find it from the chat
// (AgentChat.tsx).
//
// A paragraph's id comes from its text, so every copy of the document gives it the same one with
// nothing to agree on, and it stays while others add or delete paragraphs around it. So an id
// means the paragraph with that text: an empty paragraph, or one whose text another paragraph has
// too, has none (the tools take its number), since which of them an id meant couldn't be told.
// When a paragraph's text changes its id does too, and each agent's copy remembers what the old
// one became (`History` in word.ts), so an id an agent read or saw in the chat still finds its
// paragraph.

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

/** Each paragraph's id, from the texts of all of them in order; null for an empty one, or one whose text another has too. */
export function idsOf(texts: string[]): (string | null)[] {
  const many = new Map<string, number>()
  for (const t of texts) many.set(t, (many.get(t) ?? 0) + 1)
  // Two texts with the same hash are as good as the same.
  const hashes = texts.map((t) => (t.trim() && many.get(t) === 1 ? `#${hashOf(t)}` : null))
  const seen = new Map<string, number>()
  for (const h of hashes) if (h) seen.set(h, (seen.get(h) ?? 0) + 1)
  return hashes.map((h) => (h && seen.get(h) === 1 ? h : null))
}

/** An id as an agent may write it: "#k3f9x2" or "k3f9x2". */
export const ID = /^#?([a-z][0-9a-z]{5})$/

/** Ids in a message. */
export const IDS_IN = /#[a-z][0-9a-z]{5}(?![0-9a-z-])/g
