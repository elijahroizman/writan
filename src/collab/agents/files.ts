// Opening, making and saving documents for an agent. Only an agent on this computer may name a
// path: one elsewhere on the network could otherwise read or write the person's files.

import { KINDS, kindFromPath } from '../../lib/kinds'
import { pathExists } from '../../lib/platform'
import { getEditor, useWorkspace, wakeTab } from '../../state/workspace'
import { type Args, str, type Tool, ToolError } from './common'

const KIND_NAMES = { docx: 'Word document', pptx: 'presentation', xlsx: 'workbook' } as const
type Kind = keyof typeof KIND_NAMES

export const FILE_TOOLS: Tool[] = [
  {
    name: 'open_document',
    description:
      'Opens a Word document, presentation or workbook in Writan, in a tab the person sees, and gives its document id: a file by its full `path` (agents on this computer only), or a new, empty one with `new` ("docx", "pptx" or "xlsx"). A file already open isn’t opened twice.',
    inputSchema: {
      type: 'object',
      properties: {
        path: { type: 'string', description: 'The file’s full path, like /Users/ada/Report.docx.' },
        new: { type: 'string', enum: ['docx', 'pptx', 'xlsx'], description: 'A new, empty document of this kind.' },
      },
    },
  },
  {
    name: 'save_document',
    description:
      'Saves a document open in Writan to its file, or to `path` (a full path that doesn’t exist yet, with the right extension), which becomes its file from then on. Agents on this computer only. A document someone else shared is theirs to save. Save when your person asks you to, or when you made a new document for them.',
    inputSchema: {
      type: 'object',
      properties: {
        document: { type: 'string', description: 'The document, as list_documents gives it.' },
        path: { type: 'string', description: 'Where to save a document that has no file yet, or a new file for it.' },
      },
      required: ['document'],
    },
  },
]

const isAbsolute = (p: string) => p.startsWith('/') || /^[A-Za-z]:[\\/]/.test(p) || p.startsWith('\\\\')

/** open_document. `documents` lists what agents can work on, to say whether the new tab is one. */
export async function openDocument(args: Args, local: boolean, documents: () => { document: string; title: string }[]): Promise<string> {
  const path = str(args, 'path')?.trim()
  const kind = str(args, 'new')
  const workspace = useWorkspace.getState()
  if (kind !== undefined) {
    if (!(kind in KIND_NAMES)) throw new ToolError('`new` is "docx", "pptx" or "xlsx".')
    const before = new Set(workspace.tabs.map((t) => t.id))
    workspace.newDocument(kind)
    const tab = useWorkspace.getState().tabs.find((t) => !before.has(t.id))
    if (!tab) throw new ToolError('Writan couldn’t make the document.')
    return `Opened a new ${KIND_NAMES[kind as Kind]}, “${tab.title}”, as document: "${tab.id}". It has no file yet; save_document with a path saves it.`
  }
  if (!path) throw new ToolError('Give a `path` to open, or `new` for an empty document.')
  if (!local) throw new ToolError('Only an agent on this computer can open files. Ask the person to open it.')
  if (!isAbsolute(path)) throw new ToolError('Give the file’s full path.')
  const k = kindFromPath(path)
  if (k !== 'docx' && k !== 'pptx' && k !== 'xlsx') throw new ToolError('Agents work on Word documents, presentations and workbooks (.docx, .pptx, .xlsx).')
  if (!(await pathExists(path))) throw new ToolError(`There’s no file at ${path}.`)
  await workspace.openPaths([path])
  const tab = useWorkspace.getState().tabs.find((t) => t.path === path)
  if (!tab) throw new ToolError(`Writan couldn’t open ${path}.`)
  const listed = documents().some((d) => d.document === tab.id)
  return listed ? `Opened “${tab.title}” as document: "${tab.id}".` : `Opened “${tab.title}”, but you can’t work on it while it follows someone else’s shared document.`
}

/** save_document, once the agent's changes have reached the tab. */
export async function saveDocument(tabId: string, role: 'sharing' | 'joined' | 'open', args: Args, local: boolean): Promise<string> {
  if (!local) throw new ToolError('Only an agent on this computer can save. Ask the person to save it.')
  if (role === 'joined') throw new ToolError('Someone else shared this document: it’s their file, and they save it. Your changes are already with them.')
  const tab = useWorkspace.getState().tabs.find((t) => t.id === tabId)
  if (!tab) throw new ToolError('The document isn’t open any more.')
  const path = str(args, 'path')?.trim()
  if (path) {
    if (!isAbsolute(path)) throw new ToolError('Give the full path to save to.')
    if (kindFromPath(path) !== tab.kind || path.toLowerCase().endsWith('.ppt')) throw new ToolError(`Save a ${KIND_NAMES[tab.kind as Kind] ?? 'document'} as .${KINDS[tab.kind].extensions[0]}.`)
    if (path !== tab.path && (await pathExists(path))) throw new ToolError(`${path} already exists. Pick a new name; Writan doesn’t let agents write over files.`)
  } else if (!tab.path) {
    throw new ToolError('It has no file yet. Give a `path` to save it to.')
  } else if (tab.path.toLowerCase().endsWith('.ppt')) {
    throw new ToolError('It came from an old .ppt file. Give a `path` ending in .pptx to save it as one.')
  }
  // A tab nobody has looked at yet has no editor to save from.
  if (!getEditor(tabId)) wakeTab(tabId)
  for (let i = 0; i < 600 && !getEditor(tabId); i++) await new Promise((r) => setTimeout(r, 25))
  if (!getEditor(tabId)) throw new ToolError('The document didn’t open in time to save it.')
  let ok: boolean
  try {
    ok = await useWorkspace.getState().save(tabId, { ...(path ? { to: path } : {}), quiet: true })
  } catch (err) {
    throw new ToolError(String((err as Error)?.message ?? err))
  }
  if (!ok) throw new ToolError('It wasn’t saved.')
  const now = useWorkspace.getState().tabs.find((t) => t.id === tabId)
  return `Saved to ${now?.path ?? path ?? tab.path}.`
}
