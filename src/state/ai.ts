// Settings › AI: whether AI agents may join shared documents through Writan's MCP server, the
// secret in its address, and whether they may message other agents. Kept apart from the other settings, as the place later AI features'
// settings go.

import { create } from 'zustand'
import { isTauri } from '../lib/platform'

const KEY = 'workspace.ai'

export interface AiSettings {
  /** Agents can connect to Writan's MCP server. On in the app unless turned off; a choice saved
   * before it was (Writan 0.15 and earlier saved `false` without asking) is kept. The browser
   * version needs a standalone relay for it, so there it's off until turned on. */
  agents: boolean
  /** The secret in the MCP address. Kept, so agents set up once keep working. */
  token: string
  /** Agents here can message the other agents in a document, theirs or other people's, and get
   * their messages. On unless turned off. */
  messages: boolean
}

function newToken() {
  const b = crypto.getRandomValues(new Uint8Array(18))
  return btoa(String.fromCharCode(...b)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}

function load(): AiSettings {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) || '{}') as Partial<AiSettings>
    const token = typeof v.token === 'string' && /^[\w-]{16,}$/.test(v.token) ? v.token : newToken()
    return { agents: typeof v.agents === 'boolean' ? v.agents : isTauri, token, messages: v.messages !== false }
  } catch {
    return { agents: isTauri, token: newToken(), messages: true }
  }
}

interface AiState extends AiSettings {
  setAgents(on: boolean): void
  setMessages(on: boolean): void
  /** A new secret: agents set up with the old address can't connect any more. */
  newAddress(): void
}

function save(s: AiSettings) {
  try {
    localStorage.setItem(KEY, JSON.stringify({ agents: s.agents, token: s.token, messages: s.messages }))
  } catch {
    // Not kept; agents are off again next time.
  }
}

export const useAi = create<AiState>((set, get) => ({
  ...load(),
  setAgents: (agents) => {
    set({ agents })
    save(get())
  },
  setMessages: (messages) => {
    set({ messages })
    save(get())
  },
  newAddress: () => {
    set({ token: newToken() })
    save(get())
  },
}))

// The secret is made once and kept from then on.
save(useAi.getState())
