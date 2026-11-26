import { create } from 'zustand'

/** Whether agents can connect, and where. */
export interface AgentsStatus {
  state: 'off' | 'starting' | 'on' | 'error'
  /** The MCP URL on this computer, and on the network, without the agent part. */
  local: string | null
  network: string[]
  message: string | null
}

export const useAgentsStatus = create<AgentsStatus>(() => ({ state: 'off', local: null, network: [], message: null }))
