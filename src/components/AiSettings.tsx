// Settings › AI. For now: letting AI agents join shared documents through Writan's MCP server,
// and setting that up in the agent someone uses. Later AI settings go on this page too.

import { Bot, Check, Copy, RefreshCw } from 'lucide-react'
import { useEffect, useState } from 'react'
import { AGENT_IDS, AGENTS, type AgentId } from '../collab/agents/identity'
import { useAgentsStatus } from '../collab/agents/bridge'
import SKILL from '../collab/agents/SKILL.md?raw'
import { confirmDialog, isTauri } from '../lib/platform'
import { useAi } from '../state/ai'
import { Button, Spinner, Switch } from './ui'

function CopyButton({ text, label = 'Copy' }: { text: string; label?: string }) {
  const [copied, setCopied] = useState(false)
  return (
    <Button
      onClick={() => {
        void navigator.clipboard.writeText(text).catch(() => {})
        setCopied(true)
        window.setTimeout(() => setCopied(false), 1500)
      }}
    >
      {copied ? <Check size={14} /> : <Copy size={14} />} {copied ? 'Copied' : label}
    </Button>
  )
}

export function AiPage() {
  const agents = useAi((s) => s.agents)
  const { setAgents, newAddress } = useAi.getState()
  const status = useAgentsStatus()
  const on = agents && status.state === 'on'
  // Pi and Codex share one skill file, so a change in one row can change another's.
  const [changes, setChanges] = useState(0)
  const changed = () => setChanges((n) => n + 1)
  return (
    <>
      <p className="settings-intro">
        AI agents such as Claude Code and Codex can join the documents you share and edit them with you. Everyone in the document sees each agent’s name and logo at its cursor, like a person’s.
      </p>
      {on && status.local && (
        <section className="plugin-group">
          <h4>Set up an agent on this computer</h4>
          <p className="settings-hint plugin-group-hint">
            Adds Writan’s MCP server to the agent, so it can join, and Writan’s skill, which tells it how to work alongside people. Start a new session in the agent afterwards.
          </p>
          {AGENT_IDS.map((id) => (
            <AgentRow key={id} id={id} url={`${status.local}?agent=${id}`} changes={changes} onChange={changed} />
          ))}
          <div className="plugin-row">
            <span className="agent-logo generic" aria-hidden>
              <Bot size={18} />
            </span>
            <span className="plugin-text">
              <span className="settings-label">Another AI</span>
              <span className="settings-hint">
                Give it Writan’s address as a remote MCP server (Streamable HTTP). If it reads skills, add Writan’s as <code>writan/SKILL.md</code>.
              </span>
            </span>
            <span className="settings-inline">
              <CopyButton text={status.local} label="Copy address" />
              <CopyButton text={SKILL} label="Copy skill" />
            </span>
          </div>
        </section>
      )}
      <section className="plugin-group">
        <h4>Agents</h4>
        <div className="plugin-row">
          <span className="plugin-text">
            <span className="settings-label">Let agents join shared documents</span>
            <span className="settings-hint">
              Agents connect to Writan through an address with a secret in it, from this computer or another on your network. They can only work on documents you’ve shared, while Writan is open.
            </span>
          </span>
          {agents && status.state === 'starting' ? <Spinner size={16} /> : <Switch label="Let agents join shared documents" checked={agents} onChange={setAgents} />}
        </div>
        {agents && status.state === 'error' && <p className="settings-hint error">{status.message}</p>}
        {on && (
          <div className="settings-inline">
            <Button
              onClick={() =>
                void confirmDialog('Agents set up with the current address won’t be able to connect until you set them up again.', { title: 'Make a new address?', okLabel: 'New Address' }).then(
                  (ok) => ok && newAddress(),
                )
              }
            >
              <RefreshCw size={14} /> New address
            </Button>
            <span className="settings-hint">Anyone with the address can edit what you share. Make a new one if it got out.</span>
          </div>
        )}
      </section>
    </>
  )
}

function AgentRow({ id, url, changes, onChange }: { id: AgentId; url: string; changes: number; onChange: () => void }) {
  const agent = AGENTS[id]
  const [installed, setInstalled] = useState<{ mcp: boolean; skill: boolean } | null>(null)
  const [busy, setBusy] = useState<'mcp' | 'skill' | 'remove' | null>(null)
  const [note, setNote] = useState<{ text: string; error: boolean } | null>(null)

  const check = async () => {
    if (!isTauri) return
    const { invoke } = await import('@tauri-apps/api/core')
    setInstalled(await invoke<{ mcp: boolean; skill: boolean }>('agents_installed', { agent: id }).catch(() => null))
  }
  useEffect(() => void check(), [id, changes])

  const add = async (what: 'mcp' | 'skill') => {
    setBusy(what)
    setNote(null)
    try {
      const { invoke } = await import('@tauri-apps/api/core')
      const text = what === 'mcp' ? await invoke<string>('agents_install_mcp', { agent: id, url }) : await invoke<string>('agents_install_skill', { agent: id, skill: SKILL })
      setNote({ text, error: false })
    } catch (err) {
      setNote({ text: String((err as Error)?.message ?? err), error: true })
    } finally {
      setBusy(null)
      onChange()
    }
  }

  const remove = async () => {
    if (!(await confirmDialog(`Takes Writan’s MCP server and skill back out of ${agent.product}. Everything else stays.`, { title: `Remove Writan from ${agent.product}?`, okLabel: 'Remove', danger: true }))) return
    setBusy('remove')
    setNote(null)
    try {
      const { invoke } = await import('@tauri-apps/api/core')
      setNote({ text: await invoke<string>('agents_remove', { agent: id }), error: false })
    } catch (err) {
      setNote({ text: String((err as Error)?.message ?? err), error: true })
    } finally {
      setBusy(null)
      onChange()
    }
  }

  return (
    <div className="plugin-row">
      <span className="agent-logo" aria-hidden>
        <img src={agent.logo} alt="" style={{ transform: `scale(${agent.logoScale ?? 1})` }} />
      </span>
      <span className="plugin-text">
        <span className="settings-label">{agent.product}</span>
        <span className="settings-hint">{installed ? [installed.mcp && 'MCP server added', installed.skill && 'skill added'].filter(Boolean).join(', ') || 'Not set up' : isTauri ? ' ' : 'Set up agents in the app.'}</span>
        {note && <span className={`settings-hint ${note.error ? 'error' : ''}`}>{note.text}</span>}
      </span>
      <span className="settings-inline">
        <Button disabled={!isTauri || busy !== null} onClick={() => void add('mcp')}>
          {busy === 'mcp' ? <Spinner size={14} /> : installed?.mcp ? <Check size={14} /> : null} {installed?.mcp ? 'Update MCP server' : 'Add MCP server'}
        </Button>
        <Button disabled={!isTauri || busy !== null} onClick={() => void add('skill')}>
          {busy === 'skill' ? <Spinner size={14} /> : installed?.skill ? <Check size={14} /> : null} {installed?.skill ? 'Update skill' : 'Add skill'}
        </Button>
        {(installed?.mcp || installed?.skill) && (
          <Button variant="danger" disabled={!isTauri || busy !== null} onClick={() => void remove()}>
            {busy === 'remove' ? <Spinner size={14} /> : null} Remove
          </Button>
        )}
      </span>
    </div>
  )
}
