import { build, defineConfig } from 'vite'
import type { Plugin } from 'vite'
import react from '@vitejs/plugin-react'
import { cpSync, existsSync, readFileSync, readdirSync, statSync } from 'node:fs'
import { extname, join, normalize, resolve } from 'node:path'

const { version } = JSON.parse(readFileSync(new URL('./package.json', import.meta.url), 'utf8')) as { version: string }
const { version: mathjaxVersion } = JSON.parse(readFileSync(new URL('./node_modules/mathjax-full/package.json', import.meta.url), 'utf8')) as { version: string }

const host = process.env.TAURI_DEV_HOST

/**
 * Plugins are built apart from the app (`npm run plugins`, into plugins/<id>/dist).
 * Download plugins are fetched when turned on. Ones with `"bundled": true` (Images) are
 * copied into the app (see bundledPlugins). The dev and preview servers serve both at
 * /plugins/<id>/.
 */
function servePlugins(): Plugin {
  const types: Record<string, string> = { '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.woff2': 'font/woff2', '.svg': 'image/svg+xml', '.png': 'image/png' }
  const middleware = (req: { url?: string }, res: import('node:http').ServerResponse, next: () => void) => {
    const m = /^\/plugins\/([a-z0-9-]+)\/([^?#]+)/.exec(req.url ?? '')
    if (!m) return next()
    const file = join(import.meta.dirname, 'plugins', m[1], 'dist', normalize(decodeURIComponent(m[2])).replace(/^(\.\.[/\\])+/, ''))
    if (!existsSync(file) || !statSync(file).isFile()) return next()
    res.setHeader('Content-Type', types[extname(file)] ?? 'application/octet-stream')
    res.end(readFileSync(file))
  }
  return {
    name: 'workspace:serve-plugins',
    configureServer: (server) => void server.middlewares.use(middleware),
    configurePreviewServer: (server) => void server.middlewares.use(middleware),
  }
}

/** Builds each bundled plugin (`"bundled": true` in its plugin.json) and copies it into the app at /plugins/<id>/. */
function bundledPlugins(): Plugin {
  let outDir = 'dist'
  return {
    name: 'workspace:bundled-plugins',
    apply: 'build',
    configResolved: (config) => void (outDir = resolve(config.root, config.build.outDir)),
    async closeBundle() {
      const root = join(import.meta.dirname, 'plugins')
      for (const id of readdirSync(root)) {
        const manifest = join(root, id, 'plugin.json')
        if (!existsSync(manifest) || !(JSON.parse(readFileSync(manifest, 'utf8')) as { bundled?: boolean }).bundled) continue
        await build({ configFile: join(root, id, 'vite.config.ts') })
        cpSync(join(root, id, 'dist'), join(outDir, 'plugins', id), { recursive: true })
      }
    },
  }
}

export default defineConfig({
  plugins: [react(), servePlugins(), bundledPlugins()],
  define: { __APP_VERSION__: JSON.stringify(version), PACKAGE_VERSION: JSON.stringify(mathjaxVersion) },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: 'ws', host, port: 1421 } : undefined,
    watch: { ignored: ['**/src-tauri/**'] },
  },
  envPrefix: ['VITE_', 'TAURI_ENV_*'],
  build: {
    // Safari 15 ships with macOS 12, the oldest version we support.
    target: 'safari15',
    chunkSizeWarningLimit: 4000,
    sourcemap: false,
  },
  worker: { format: 'es' },
})
