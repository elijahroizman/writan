// Who an AI agent is, as people see it in a shared document: its name, its color and its
// product's logo. An agent installed from Settings › AI says which it is in its URL
// (`?agent=codex`); any other is known by the name its MCP client gives (`clientInfo.name`).

import antigravity from './logos/antigravity.svg'
import claude from './logos/claude.svg'
import codex from './logos/codex.svg'
import grok from './logos/grok.svg'
import hermes from './logos/hermes.svg'

export type AgentId = 'claude' | 'codex' | 'antigravity' | 'grok' | 'hermes'

export interface Agent {
  id: AgentId
  /** As people see it in the document. */
  name: string
  /** The product, in Settings. */
  product: string
  color: string
  logo: string
  /** How much bigger than the others its logo is drawn, for a mark that's narrow in its box. */
  logoScale?: number
  /** The name its MCP client gives in `initialize`, or the start of it. */
  client: (name: string) => boolean
}

export const AGENTS: Record<AgentId, Agent> = {
  claude: { id: 'claude', name: 'Claude', product: 'Claude Code', color: '#D97757', logo: claude, client: (n) => n === 'claude-code' || n.startsWith('claude') },
  codex: { id: 'codex', name: 'Codex', product: 'Codex', color: '#0D0D0D', logo: codex, client: (n) => n === 'codex-mcp-client' || n.startsWith('codex') },
  antigravity: { id: 'antigravity', name: 'Antigravity', product: 'Antigravity', color: '#3186FF', logo: antigravity, client: (n) => n.startsWith('antigravity') },
  grok: { id: 'grok', name: 'Grok', product: 'Grok Build', color: '#0D0D0D', logo: grok, client: (n) => n.startsWith('grok') },
  hermes: { id: 'hermes', name: 'Hermes', product: 'Hermes Agent', color: '#0000F2', logo: hermes, logoScale: 1.35, client: (n) => n.startsWith('hermes') },
}

export const AGENT_IDS = Object.keys(AGENTS) as AgentId[]

/** What an agent that isn't one of those shows as. */
export interface OtherAgent {
  id: null
  name: string
}

/** The agent behind an MCP session, from its URL's `agent` and its client's name. */
export function agentFrom(agent: string | null | undefined, client: string | null | undefined): Agent | OtherAgent {
  if (agent && agent in AGENTS) return AGENTS[agent as AgentId]
  const name = (client ?? '').toLowerCase()
  const known = AGENT_IDS.find((id) => AGENTS[id].client(name))
  if (known) return AGENTS[known]
  // An MCP client's name, made readable: "my-agent-client" is "My agent".
  const readable = (client ?? '').replace(/[-_](mcp[-_])?client$/i, '').replace(/[-_]+/g, ' ').trim()
  return { id: null, name: readable ? readable.charAt(0).toUpperCase() + readable.slice(1) : 'AI agent' }
}

/** The logo for a peer that's an agent, if it's one Writan knows, and how much to enlarge it. */
export function logoOf(agent: string | null | undefined): { src: string; scale: number } | null {
  const a = agent && agent in AGENTS ? AGENTS[agent as AgentId] : null
  return a ? { src: a.logo, scale: a.logoScale ?? 1 } : null
}
