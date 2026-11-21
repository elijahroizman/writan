// Settings › AI: whether AI agents may join shared documents through Writan's MCP server, and
// the secret in its address. Kept apart from the other settings, as the place later AI features'
// settings go.

import { create } from 'zustand'

const KEY = 'workspace.ai'

export interface AiSettings {
  /** Agents can connect to Writan's MCP server. */
  agents: boolean
  /** The secret in the MCP address. Kept, so agents set up once keep working. */
  token: string
}

function newToken() {
  const b = crypto.getRandomValues(new Uint8Array(18))
  return btoa(String.fromCharCode(...b)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}

function load(): AiSettings {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) || '{}') as Partial<AiSettings>
    const token = typeof v.token === 'string' && /^[\w-]{16,}$/.test(v.token) ? v.token : newToken()
    return { agents: v.agents === true, token }
  } catch {
    return { agents: false, token: newToken() }
  }
}

interface AiState extends AiSettings {
  setAgents(on: boolean): void
  /** A new secret: agents set up with the old address can't connect any more. */
  newAddress(): void
}

function save(s: AiSettings) {
  try {
    localStorage.setItem(KEY, JSON.stringify({ agents: s.agents, token: s.token }))
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
  newAddress: () => {
    set({ token: newToken() })
    save(get())
  },
}))

// The secret is made once and kept from then on.
save(useAi.getState())
