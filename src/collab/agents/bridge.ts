// The page's side of Writan's MCP server (src-tauri/src/collab/mcp.rs). Agents' `tools/list` and
// `tools/call` arrive here over a WebSocket to the relay's server, which only this computer can
// open, and the answers go back the same way. The connection goes through transport.ts like a
// shared document's, so in the browser it reaches the standalone relay the same way.

import { create } from 'zustand'
import { isTauri } from '../../lib/platform'
import { browserRelay, connect, resetNative, type Transport } from '../transport'
import { agentFrom } from './identity'
import { AgentSession, runTool, ToolError, TOOLS } from './tools'

/** Whether agents can connect, and where. */
export interface AgentsStatus {
  state: 'off' | 'starting' | 'on' | 'error'
  /** The MCP URL on this computer, and on the network, without the agent part. */
  local: string | null
  network: string[]
  message: string | null
}

export const useAgentsStatus = create<AgentsStatus>(() => ({ state: 'off', local: null, network: [], message: null }))

let bridge: Transport | null = null
const sessions = new Map<string, AgentSession>()

interface Call {
  t: 'call'
  id: number
  session: string
  agent: string | null
  client: string
  method: string
  params: { name?: string; arguments?: Record<string, unknown> } | null
}

async function answer(call: Call): Promise<unknown> {
  if (call.method === 'tools/list') return { tools: TOOLS }
  let session = sessions.get(call.session)
  if (!session) {
    session = new AgentSession(agentFrom(call.agent, call.client))
    sessions.set(call.session, session)
  }
  const name = call.params?.name ?? ''
  if (!TOOLS.some((t) => t.name === name)) return { content: [{ type: 'text', text: `Writan has no tool ${name}.` }], isError: true }
  try {
    const text = await runTool(session, name, call.params?.arguments ?? {})
    return { content: [{ type: 'text', text }] }
  } catch (err) {
    // A mistake the agent can fix comes back as the tool's answer, so it can try again.
    const text = err instanceof ToolError ? err.message : `Something went wrong: ${String((err as Error)?.message ?? err)}`
    return { content: [{ type: 'text', text }], isError: true }
  }
}

function onMessage(data: string) {
  let msg: Call | { t: 'end'; session: string }
  try {
    msg = JSON.parse(data)
  } catch {
    return
  }
  if (msg.t === 'end') {
    sessions.get(msg.session)?.end()
    sessions.delete(msg.session)
    return
  }
  if (msg.t !== 'call') return
  const call = msg
  void answer(call).then((result) => bridge?.send(JSON.stringify({ t: 'result', id: call.id, result })))
}

/**
 * Lets agents in: starts the MCP server (the relay's, which starts if nothing is shared yet) with
 * `token` in its URL, and connects this page to it.
 */
export async function startAgents(token: string): Promise<void> {
  stopBridge()
  useAgentsStatus.setState({ state: 'starting', message: null })
  try {
    let base: string
    let key: string
    let network: string[] = []
    if (isTauri) {
      await resetNative()
      const { invoke } = await import('@tauri-apps/api/core')
      const info = await invoke<{ port: number; bridgeKey: string; addresses: string[] }>('agents_enable', { token })
      base = `127.0.0.1:${info.port}`
      key = info.bridgeKey
      network = info.addresses.map((a) => `http://${a}:${info.port}/mcp/${token}`)
    } else {
      // The browser uses the standalone relay (cargo run --example collab_relay -- --mcp …).
      base = browserRelay().replace(/^wss?:\/\//, '')
      key = localStorage.getItem('workspace.agentsBridgeKey') ?? ''
    }
    bridge = await connect(`ws://${base}/bridge/${key}`, {
      message: onMessage,
      closed: (reason) => {
        bridge = null
        endSessions()
        if (useAgentsStatus.getState().state === 'on') useAgentsStatus.setState({ state: 'error', message: `Agents can’t reach Writan any more: ${reason}` })
      },
    })
    useAgentsStatus.setState({ state: 'on', local: `http://${base}/mcp/${token}`, network, message: null })
  } catch (err) {
    useAgentsStatus.setState({ state: 'error', message: String((err as Error)?.message ?? err) })
  }
}

function endSessions() {
  for (const s of sessions.values()) s.end()
  sessions.clear()
}

function stopBridge() {
  bridge?.close()
  bridge = null
  endSessions()
}

/** Stops letting agents in; the ones in documents leave them. */
export async function stopAgents(): Promise<void> {
  stopBridge()
  useAgentsStatus.setState({ state: 'off', local: null, network: [], message: null })
  if (isTauri) {
    const { invoke } = await import('@tauri-apps/api/core')
    await invoke('agents_disable').catch(() => {})
  }
}

/** Keeps agents' access as Settings › AI has it: on at launch if it was on, and following the
 * switch and the address from then on. Returns the undo. */
export function followAiSettings(settings: { getState(): { agents: boolean; token: string }; subscribe(f: (s: { agents: boolean; token: string }, prev: { agents: boolean; token: string }) => void): () => void }): () => void {
  const { agents, token } = settings.getState()
  if (agents) void startAgents(token)
  return settings.subscribe((s, prev) => {
    if (s.agents === prev.agents && s.token === prev.token) return
    if (s.agents) void startAgents(s.token)
    else if (prev.agents) void stopAgents()
  })
}
