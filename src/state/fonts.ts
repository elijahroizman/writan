// Fonts Writan can add on request (src-tauri/src/fonts.rs), so the app stays small and each
// font arrives only for people who want it. Michelangelus is Microsoft's free typeface drawn from
// Michelangelo's handwriting; its license forbids passing it on, so it comes from Microsoft. The
// others are open-source and come from Google Fonts or their maker. Once one is there, Word
// documents (through the engine's customFonts) and slides (a page-wide FontFace) can use it.
//
// On Linux, Writan also offers Microsoft's core fonts (Arial, Times New Roman, Courier New…),
// which Mac and Windows already have. Without them, documents use the bundled look-alikes with
// the same measurements (src/engine/fonts). They download only after the person accepts
// Microsoft's license; the engine asks for a document's own font first, so once they're
// registered here they replace the look-alikes.

import { create } from 'zustand'
import { isLinux, isTauri } from '../lib/platform'
import MICROSOFT_FONTS_EULA from './microsoft-fonts-eula.txt?raw'

export interface ExtraFace {
  file: string
  /** For a download with several families (Microsoft's core fonts); otherwise the font's. */
  family?: string
  weight: number
  style: 'normal' | 'italic'
  hash: string
}

export interface ExtraFont {
  id: string
  family: string
  maker: string
  /** The generic family behind it in Settings, where its name is shown in the font itself. */
  fallback: 'serif' | 'sans-serif' | 'monospace' | 'cursive'
  about: string
  /** Download size in bytes. */
  size: number
  licenseUrl: string
  faces: ExtraFace[]
}

/** Keep in step with FONTS in src-tauri/src/fonts.rs: same ids, files and hashes. */
export const EXTRA_FONTS: ExtraFont[] = [
  {
    id: 'michelangelus',
    family: 'Michelangelus',
    maker: 'Microsoft',
    fallback: 'serif',
    about: 'A serif drawn from Michelangelo’s handwriting. Microsoft doesn’t allow apps to include it, so it comes from Microsoft.',
    size: 410_000,
    licenseUrl: 'https://www.microsoft.com/en-us/download/details.aspx?id=108856',
    faces: [
      { file: 'Michelangelus-Regular.ttf', weight: 400, style: 'normal', hash: '8e21078b09f6befa90ece78b55b8140d4a3b778eb48e68a7c899e02074429a4a' },
      { file: 'Michelangelus-Italic.ttf', weight: 400, style: 'italic', hash: '36ef7b5a9a1f25c29c8ad3478d649fba2f64549bc50aab165dc55fb4418741da' },
      { file: 'Michelangelus-Bold.ttf', weight: 700, style: 'normal', hash: 'c5f07eb1d008b2ad1f75a9b5e8f91ba45827ef2b6d0f6989d0371a6168e1125f' },
      { file: 'Michelangelus-BoldItalic.ttf', weight: 700, style: 'italic', hash: 'f4815c2e4212be963d24c913c689dd2a97dc607e0e9ff0587f2ab90ad2641a52' },
    ],
  },
  {
    id: 'ubuntu',
    family: 'Ubuntu',
    maker: 'Dalton Maag for Canonical',
    fallback: 'sans-serif',
    about: 'Ubuntu’s rounded, friendly sans-serif.',
    size: 1_422_860,
    licenseUrl: 'https://github.com/google/fonts/blob/47d54ca1ebb14fd864695f1d39dc734f39017c33/ufl/ubuntu/UFL.txt',
    faces: [
      { file: 'Ubuntu-Regular.ttf', weight: 400, style: 'normal', hash: '3128df86a31805618436d0ae5651ba4285d0c9de0a39057d025f64ee33bceb64' },
      { file: 'Ubuntu-Italic.ttf', weight: 400, style: 'italic', hash: '4ab857e72f781a8967a6e4a9ac8858fbd6b3a9f9782db349d4b62b78ed02860b' },
      { file: 'Ubuntu-Bold.ttf', weight: 700, style: 'normal', hash: '679b5c1e09cab3156bb8ef529735f9382bf31ca7ac737382ab959297f8d82ad4' },
      { file: 'Ubuntu-BoldItalic.ttf', weight: 700, style: 'italic', hash: '875d776e7f33c50b1d1b594791da0eba9865648f232f08bcba00bba9dfa01d96' },
    ],
  },
  {
    id: 'jetbrains-mono',
    family: 'JetBrains Mono',
    maker: 'JetBrains',
    fallback: 'monospace',
    about: 'A monospaced face made for code, with tall letters that are easy to tell apart.',
    size: 5_622_857,
    licenseUrl: 'https://github.com/JetBrains/JetBrainsMono/blob/v2.304/OFL.txt',
    faces: [
      { file: 'JetBrainsMono-Regular.ttf', weight: 400, style: 'normal', hash: 'a0bf60ef0f83c5ed4d7a75d45838548b1f6873372dfac88f71804491898d138f' },
      { file: 'JetBrainsMono-Italic.ttf', weight: 400, style: 'italic', hash: '9d0a1f7a708e6af183f1193b7e81d40da294f5c67682c085d8401c60aac8ded4' },
      { file: 'JetBrainsMono-Bold.ttf', weight: 700, style: 'normal', hash: '5590990c82e097397517f275f430af4546e1c45cff408bde4255dad142479dcb' },
      { file: 'JetBrainsMono-BoldItalic.ttf', weight: 700, style: 'italic', hash: '4039d5ce0ed225bf9c8b2c8c6436290ae2f356b7e90d70fa666227238324aa3b' },
    ],
  },
  {
    id: 'atkinson-hyperlegible',
    family: 'Atkinson Hyperlegible',
    maker: 'Braille Institute',
    fallback: 'sans-serif',
    about: 'Drawn for readers with low vision: every letter is shaped to be hard to mistake for another.',
    size: 219_784,
    licenseUrl: 'https://github.com/google/fonts/blob/47d54ca1ebb14fd864695f1d39dc734f39017c33/ofl/atkinsonhyperlegible/OFL.txt',
    faces: [
      { file: 'AtkinsonHyperlegible-Regular.ttf', weight: 400, style: 'normal', hash: '7fb917c89019896d0b52ee84b7cbb3304c18cb90b19a62f5e32712bd23e97669' },
      { file: 'AtkinsonHyperlegible-Italic.ttf', weight: 400, style: 'italic', hash: '021beda4d3c6edfc78872e436d74009f9a1bcb294331908fe5747c61d3dcc514' },
      { file: 'AtkinsonHyperlegible-Bold.ttf', weight: 700, style: 'normal', hash: '5a3b0c8cc8ca545155150b4512a1fa248298df121c50d6557e651e61fbdab92f' },
      { file: 'AtkinsonHyperlegible-BoldItalic.ttf', weight: 700, style: 'italic', hash: '7a46f1b0ef2f64b61065c1a6b5a11fcb9b019b44062d88ba121a083cc39e86bf' },
    ],
  },
  {
    id: 'courier-prime',
    family: 'Courier Prime',
    maker: 'Quote-Unquote Apps',
    fallback: 'monospace',
    about: 'A better-looking Courier, made for screenplays.',
    size: 304_896,
    licenseUrl: 'https://github.com/google/fonts/blob/47d54ca1ebb14fd864695f1d39dc734f39017c33/ofl/courierprime/OFL.txt',
    faces: [
      { file: 'CourierPrime-Regular.ttf', weight: 400, style: 'normal', hash: '72f793376f8e2841656bf21d77a5de010f2929bd6956a22ee848ad0c7eb978af' },
      { file: 'CourierPrime-Italic.ttf', weight: 400, style: 'italic', hash: 'f1b9a5829789f7e56432a9f3bc7665ef4531dbba1c112639e48bef39621a006b' },
      { file: 'CourierPrime-Bold.ttf', weight: 700, style: 'normal', hash: 'ff1f38786c849d1c41fa8e447960abdb2bd75fdfb0cfcdeb524fad65a5af3638' },
      { file: 'CourierPrime-BoldItalic.ttf', weight: 700, style: 'italic', hash: '3355273f3ea6d6362658f9564f4e83d59e156cf1a182e3c1592b5c5943627dcc' },
    ],
  },
  {
    id: 'instrument-serif',
    family: 'Instrument Serif',
    maker: 'Instrument',
    fallback: 'serif',
    about: 'A tall, condensed display serif for headings and title slides. Regular and italic.',
    size: 141_604,
    licenseUrl: 'https://github.com/google/fonts/blob/47d54ca1ebb14fd864695f1d39dc734f39017c33/ofl/instrumentserif/OFL.txt',
    faces: [
      { file: 'InstrumentSerif-Regular.ttf', weight: 400, style: 'normal', hash: '498efd461f6ddfcb7a111bf9a565709d2085d48201d501ead960d93e84ffbb88' },
      { file: 'InstrumentSerif-Italic.ttf', weight: 400, style: 'italic', hash: '08939b8bdf534afec24ae0ef5e03f948940cd9a8fe08e7fecbad040e62327385' },
    ],
  },
  {
    id: 'comic-neue',
    family: 'Comic Neue',
    maker: 'Craig Rozynski',
    fallback: 'cursive',
    about: 'Comic Sans, redrawn to be tidier. For posters, worksheets and party invitations.',
    size: 223_284,
    licenseUrl: 'https://github.com/google/fonts/blob/47d54ca1ebb14fd864695f1d39dc734f39017c33/ofl/comicneue/OFL.txt',
    faces: [
      { file: 'ComicNeue-Regular.ttf', weight: 400, style: 'normal', hash: 'a0ee5a37c8b27c4db0700137d928598b1e23b0089e1546a8961909176b779360' },
      { file: 'ComicNeue-Italic.ttf', weight: 400, style: 'italic', hash: 'e06bfd1552f5c9464c5665733ffd69239b0593885dbb9e059688a5900f78cf98' },
      { file: 'ComicNeue-Bold.ttf', weight: 700, style: 'normal', hash: '3e7e5fccfd7e0788f317b43312151c1bd5cf058c9697a8d83eac3939050bd61e' },
      { file: 'ComicNeue-BoldItalic.ttf', weight: 700, style: 'italic', hash: '5c312c2a2fa64eee82f3b87fcfab8f3b12a5e59b043124401d322eb323cfbf16' },
    ],
  },
]

const coreFace = (family: string, file: string, weight: number, style: 'normal' | 'italic', hash: string): ExtraFace => ({ family, file, weight, style, hash })

/**
 * Microsoft's core fonts for the web, unpacked from its original installers. Keep in step with
 * "ms-core-fonts" in src-tauri/src/fonts.rs. Not in EXTRA_FONTS: it is one download of many
 * families, offered on Linux only, behind Microsoft's license.
 */
export const CORE_FONTS: ExtraFont & { license: string; families: string[] } = {
  id: 'ms-core-fonts',
  family: 'Arial',
  maker: 'Microsoft',
  fallback: 'sans-serif',
  about: 'Arial, Times New Roman, Courier New, Georgia, Verdana, Trebuchet MS, Comic Sans MS, Impact, Arial Black, Andale Mono and Webdings.',
  size: 3_934_864,
  licenseUrl: 'https://sourceforge.net/projects/corefonts/',
  license: MICROSOFT_FONTS_EULA,
  families: ['Arial', 'Times New Roman', 'Courier New', 'Georgia', 'Verdana', 'Trebuchet MS', 'Comic Sans MS', 'Impact', 'Arial Black', 'Andale Mono', 'Webdings'],
  faces: [
    coreFace('Andale Mono', 'andalemo.ttf', 400, 'normal', '48d9bc613917709d3b0e0f4a6d4fe33a5c544c5035dffe9e90bc11e50e822071'),
    coreFace('Arial', 'arial.ttf', 400, 'normal', '35c0f3559d8db569e36c31095b8a60d441643d95f59139de40e23fada819b833'),
    coreFace('Arial', 'arialbd.ttf', 700, 'normal', '4044aa6b5bebbc36980206b45b0aaaaa5681552a48bcadb41746d5d1d71fd7b4'),
    coreFace('Arial', 'ariali.ttf', 400, 'italic', '70ade233175a6a6675e4501461af9326e6f78b1ffdf787ca0da5ab0fc8c9cfd6'),
    coreFace('Arial', 'arialbi.ttf', 700, 'italic', '2f371cd9d96b3ac544519d85c16dc43ceacdfcea35090ee8ddf3ec5857c50328'),
    coreFace('Arial Black', 'ariblk.ttf', 400, 'normal', 'dad7c04acb26e23dfe4780e79375ca193ddaf68409317e81577a30674668830e'),
    coreFace('Comic Sans MS', 'comic.ttf', 400, 'normal', 'b82c53776058f291382ff7e008d4675839d2dc21eb295c66391f6fb0655d8fc0'),
    coreFace('Comic Sans MS', 'comicbd.ttf', 700, 'normal', '873361465d994994762d0b9845c99fc7baa2a600442ea8db713a7dd19f8b0172'),
    coreFace('Courier New', 'cour.ttf', 400, 'normal', '6715838c52f813f3821549d3f645db9a768bd6f3a43d8f85a89cb6875a546c61'),
    coreFace('Courier New', 'courbd.ttf', 700, 'normal', 'edf8a7c5bfcac2e1fe507faab417137cbddc9071637ef4648238d0768c921e02'),
    coreFace('Courier New', 'couri.ttf', 400, 'italic', 'f3f6b09855b6700977e214aab5eb9e5be6813976a24f894bd7766e92c732fbe1'),
    coreFace('Courier New', 'courbi.ttf', 700, 'italic', '66dbfa20b534fba0e203da140fec7276a45a1069e424b1b9c35547538128bbe8'),
    coreFace('Georgia', 'georgia.ttf', 400, 'normal', '7d0bb20c632bb59e81a0885f573bd2173f71f73204de9058feb68ce032227072'),
    coreFace('Georgia', 'georgiab.ttf', 700, 'normal', '82d2fbadb88a8632d7f2e8ad50420c9fd2e7d3cbc0e90b04890213a711b34b93'),
    coreFace('Georgia', 'georgiai.ttf', 400, 'italic', '1523f19bda6acca42c47c50da719a12dd34f85cc2606e6a5af15a7728b377b60'),
    coreFace('Georgia', 'georgiaz.ttf', 700, 'italic', 'c983e037d8e4e694dd0fb0ba2e625bca317d67a41da2dc81e46a374e53d0ec8a'),
    coreFace('Impact', 'impact.ttf', 400, 'normal', '00f1fc230ac99f9b97ba1a7c214eb5b909a78660cb3826fca7d64c3af5a14848'),
    coreFace('Times New Roman', 'times.ttf', 400, 'normal', '4e98adeff8ccc8ef4e3ece8d4547e288ff85fdc9c7ca711a4599c234874bbe86'),
    coreFace('Times New Roman', 'timesbd.ttf', 700, 'normal', '4357b63cef20c01661a53c5dae70ffd20cb4765503aaed6d38b17a57c5a90bff'),
    coreFace('Times New Roman', 'timesi.ttf', 400, 'italic', 'c25ae529b4cecdbca148b6ccb862ee0abad770af8b1fd29c8dba619d1b8da78a'),
    coreFace('Times New Roman', 'timesbi.ttf', 700, 'italic', '192e1b0d18e90334e999a99f8c32808d6a2e74b3698b8cd90c943c2249a46549'),
    coreFace('Trebuchet MS', 'trebuc.ttf', 400, 'normal', 'ec3ffb302488251e1b67fb09dd578b364c5339e27c1cfb26eb627666236453d0'),
    coreFace('Trebuchet MS', 'trebucbd.ttf', 700, 'normal', 'f65941f9487c0a0a3b7445996ecbbd24466d7ae76ea2a597ced55f438fa63838'),
    coreFace('Trebuchet MS', 'trebucit.ttf', 400, 'italic', 'db56fdac7d3ba20b7aededcb6ee86c46687489d17b759e1708ea4e2d21e38410'),
    coreFace('Trebuchet MS', 'trebucbi.ttf', 700, 'italic', 'c0a6bdf31f9f2953b2f08a0c1734c892bc825f0fb17c604d420f7acf203a213b'),
    coreFace('Verdana', 'verdana.ttf', 400, 'normal', '96ed14949ca4b7392cff235b9c41d55c125382abbe0c0d3c2b9dd66897cae0cb'),
    coreFace('Verdana', 'verdanab.ttf', 700, 'normal', 'c8f5065ba91680f596af3b0378e2c3e713b95a523be3d56ae185ca2b8f5f0b23'),
    coreFace('Verdana', 'verdanai.ttf', 400, 'italic', '91b59186656f52972531a11433c866fd56e62ec4e61e2621a2dba70c8f19a828'),
    coreFace('Verdana', 'verdanaz.ttf', 700, 'italic', '698e220f48f4a40e77af7eb34958c8fd02f1e18c3ba3f365d93bfa2ed4474c80'),
    coreFace('Webdings', 'webdings.ttf', 400, 'normal', '10d099c88521b1b9e380b7690cbe47b54bb19396ca515358cfdc15ac249e2f5d'),
  ],
}

const ALL_FONTS = [...EXTRA_FONTS, CORE_FONTS]

const invoke = async <T>(cmd: string, args?: Record<string, unknown>) => (await import('@tauri-apps/api/core')).invoke<T>(cmd, args)

// The Word engine caches font bytes by URL; these never leave the app (docx/extraFonts.ts answers them).
export const FONT_HOST = 'https://fonts.workspace.invalid/'
export const FONT_CACHE = 'workspace-extra-fonts'

interface FontsState {
  installed: Record<string, boolean>
  busy: Record<string, boolean>
  errors: Record<string, string>
  /** Whether the system has Microsoft's core fonts itself; null until asked (Linux only). */
  coreOnSystem: boolean | null
  refresh(): Promise<void>
  /** `acceptedLicense`: the person accepted the font's license (Microsoft's core fonts). */
  install(id: string, acceptedLicense?: boolean): Promise<void>
  remove(id: string): Promise<void>
}

export const useExtraFonts = create<FontsState>((set, get) => ({
  installed: {},
  busy: {},
  errors: {},
  coreOnSystem: null,

  refresh: async () => {
    if (!isTauri) return
    if (isLinux) void invoke<boolean>('system_has_core_fonts').then((has) => set({ coreOnSystem: has }), () => set({ coreOnSystem: false }))
    const ids = await invoke<string[]>('installed_fonts').catch(() => [])
    set({ installed: Object.fromEntries(ids.map((id) => [id, true])) })
    for (const id of ids) void registerPageFont(id)
  },

  install: async (id, acceptedLicense) => {
    if (get().busy[id]) return
    set({ busy: { ...get().busy, [id]: true }, errors: { ...get().errors, [id]: '' } })
    try {
      await invoke('install_font', { id, acceptedLicense: !!acceptedLicense })
      set({ installed: { ...get().installed, [id]: true } })
      await registerPageFont(id)
    } catch (err) {
      set({ errors: { ...get().errors, [id]: String(err) } })
    } finally {
      set({ busy: { ...get().busy, [id]: false } })
    }
  },

  remove: async (id) => {
    try {
      await invoke('remove_font', { id })
      set({ installed: { ...get().installed, [id]: false } })
      unregisterPageFont(id)
      await globalThis.caches?.delete(FONT_CACHE).catch(() => false)
    } catch (err) {
      set({ errors: { ...get().errors, [id]: String(err) } })
    }
  },
}))

/** Families the slide editor's font menus should offer. */
export function extraFamilies(installed: Record<string, boolean>): string[] {
  return ALL_FONTS.filter((f) => installed[f.id]).flatMap((f) => [...new Set(f.faces.map((face) => face.family ?? f.family))])
}

/** Whether to offer Microsoft's core fonts: in the Linux app, when the system lacks them. */
export function offerCoreFonts(s: Pick<FontsState, 'coreOnSystem' | 'installed'>): boolean {
  return isTauri && isLinux && (s.coreOnSystem === false || !!s.installed[CORE_FONTS.id])
}

export async function faceBytes(id: string, file: string): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>('font_file', { id, file })
}

/** Faces added to document.fonts, by font id. */
const registered = new Map<string, FontFace[]>()

/** Slides draw text with the page's own fonts, so the faces are added to document.fonts. */
async function registerPageFont(id: string) {
  const font = ALL_FONTS.find((f) => f.id === id)
  if (!font || registered.has(id)) return
  const faces: FontFace[] = []
  // Claimed while loading so a second refresh doesn't load them twice; a failure frees it to retry.
  registered.set(id, faces)
  try {
    for (const face of font.faces) {
      const loaded = await new FontFace(face.family ?? font.family, await faceBytes(id, face.file), { weight: String(face.weight), style: face.style }).load()
      // Removed while this was loading.
      if (registered.get(id) !== faces) return
      document.fonts.add(loaded)
      faces.push(loaded)
    }
    // Documents already open measured with the look-alikes (or nothing): lay them out again.
    if (faces.length) void import('../engine/host').then((h) => h.fontsChanged())
  } catch (err) {
    console.warn(`Couldn't load ${font.family}`, err)
    unregisterPageFont(id)
  }
}

function unregisterPageFont(id: string) {
  const faces = registered.get(id) ?? []
  for (const face of faces) document.fonts.delete(face)
  registered.delete(id)
  if (faces.length) void import('../engine/host').then((h) => h.fontsChanged())
}
