// Restyling Writan itself: an agent on this computer reads the themes and publishes its own into
// the themes folder, and switches the app to it. Modeled on T3 Code's t3_theme_read and
// t3_theme_set (MIT, © 2026 T3 Tools Inc.).

import { cssLoadsFiles } from '../../lib/themeCss'
import type { ThemeDef } from '../../lib/themes'
import { DEFAULT_THEME_ID, isThemeId, parseThemeFile, PRESET_THEMES, THEME_GUIDE, ThemeFileError, themeFile } from '../../lib/themes'
import { isTauri } from '../../lib/platform'
import { allThemes, publishTheme, refreshPublished, useAppearance } from '../../state/appearance'
import { useWorkspace } from '../../state/workspace'
import { type Answer, type Args, str, type Tool, ToolError } from './common'

export const THEME_TOOLS: Tool[] = [
  {
    name: 'read_theme',
    description:
      'Reads Writan’s own look: the themes in use for light and dark, every theme there is, where the themes folder is, and the guide to restyling Writan (color roles, CSS rules and selectors). Call it before set_theme. Give theme_id to also get that theme’s file.',
    inputSchema: {
      type: 'object',
      properties: { theme_id: { type: 'string', description: 'Also return this theme’s file.' } },
    },
  },
  {
    name: 'set_theme',
    description:
      'Restyles Writan itself, only when the person asks you to. With `theme`, publishes it to the themes folder as `theme_id` (the same id updates it) and switches Writan to it for light and dark. Without `theme`, switches to an existing theme. `appearance` sets light, dark or system mode. Only for agents on this computer.',
    inputSchema: {
      type: 'object',
      properties: {
        theme_id: { type: 'string', description: 'Lowercase letters, digits and hyphens, like "nightfall". Not a built-in theme’s id when publishing.' },
        theme: {
          type: 'object',
          description:
            'The theme: { "name", "accent", "light", "dark" (hex colors), optional "variants": { "light": { role: color }, "dark": { role: color } }, optional "css" }. read_theme’s guide lists the roles.',
        },
        appearance: { type: 'string', enum: ['light', 'dark', 'system'] },
      },
    },
  },
]

const describe = (t: ThemeDef) => `- ${t.id}: ${t.name}${t.published ? ' (themes folder' : t.custom ? ' (made in Writan' : ' (built in'}${t.css ? ', has CSS' : ''}${t.background ? ', has a background image' : ''})`

export async function runThemeTool(name: string, args: Args, local: boolean): Promise<Answer> {
  if (!local) throw new ToolError('Only an agent on the computer Writan runs on can change how it looks.')
  await refreshPublished()
  if (name === 'read_theme') return { text: readTheme(str(args, 'theme_id')) }
  return { text: await setTheme(args) }
}

function readTheme(themeId: string | undefined): string {
  const { light, dark, themesDir } = useAppearance.getState()
  const { themeMode, theme } = useWorkspace.getState()
  const themes = allThemes()
  const lines = [
    `In use: "${light}" in light mode, "${dark}" in dark mode. Appearance: ${themeMode}${themeMode === 'system' ? ` (${theme} now)` : ''}.`,
    '',
    'Themes:',
    ...themes.map(describe),
    '',
    isTauri && themesDir ? `Themes folder: ${themesDir}` : 'There’s no themes folder in the browser version; set_theme saves the theme in the browser instead.',
  ]
  if (themeId) {
    const t = themes.find((x) => x.id === themeId)
    if (!t) throw new ToolError(`There’s no theme “${themeId}”. The themes are: ${themes.map((x) => x.id).join(', ')}.`)
    lines.push('', `Theme "${t.id}":`, JSON.stringify(themeFile(t), null, 2))
  }
  lines.push('', 'Guide:', THEME_GUIDE)
  return lines.join('\n')
}

async function setTheme(args: Args): Promise<string> {
  const themeId = str(args, 'theme_id')
  const appearance = str(args, 'appearance')
  const theme = args.theme
  if (appearance !== undefined && !['light', 'dark', 'system'].includes(appearance)) throw new ToolError('appearance is light, dark or system.')
  if (theme !== undefined && themeId === undefined) throw new ToolError('Give theme_id to publish the theme under.')
  if (themeId === undefined && appearance === undefined) throw new ToolError('Give theme_id, appearance, or both.')
  const done: string[] = []

  if (themeId !== undefined) {
    if (theme !== undefined) {
      if (!isThemeId(themeId)) throw new ToolError(`“${themeId}” can’t name a theme. Use lowercase letters, digits and hyphens, and not a built-in theme’s id (${PRESET_THEMES.map((t) => t.id).join(', ')}) or system, light or dark.`)
      let parsed: Omit<ThemeDef, 'id'>
      try {
        parsed = parseThemeFile(theme, themeId)
      } catch (err) {
        throw new ToolError(err instanceof ThemeFileError ? err.message : String(err))
      }
      if (parsed.css && cssLoadsFiles(parsed.css)) throw new ToolError('The CSS loads files. Remove @import, image-set(), and url() that isn’t data: or #id.')
      const def: ThemeDef = { ...parsed, id: themeId }
      if (isTauri) {
        if (useAppearance.getState().custom.some((t) => t.id === themeId)) throw new ToolError(`“${themeId}” is a theme the person made in Writan. Use another id.`)
        const path = await publishTheme(themeId, themeFile(def)).catch((err) => Promise.reject(new ToolError(`The theme wasn’t published: ${String((err as Error)?.message ?? err)}`)))
        // The folder holds 32 themes and 2 MB at most; past that, a file is left out.
        if (!useAppearance.getState().published.some((t) => t.id === themeId)) throw new ToolError(`The theme was saved to ${path}, but Writan didn’t load it: the themes folder may be full (32 themes, 2 MB).`)
        done.push(`Published “${def.name}” to ${path}.`)
      } else {
        useAppearance.getState().save(def)
        done.push(`Saved “${def.name}” in this browser.`)
      }
    } else if (!allThemes().some((t) => t.id === themeId)) {
      throw new ToolError(`There’s no theme “${themeId}”. The themes are: ${allThemes().map((t) => t.id).join(', ')}.`)
    }
    useAppearance.getState().use(themeId)
    done.push(themeId === DEFAULT_THEME_ID ? 'Writan is back to its own theme.' : `Writan now uses “${themeId}” in light and dark mode.`)
  }
  if (appearance !== undefined) {
    useWorkspace.getState().setThemeMode(appearance as 'light' | 'dark' | 'system')
    done.push(`Appearance: ${appearance}.`)
  }
  done.push('Ask the person how it looks; you can’t see it.')
  return done.join(' ')
}
