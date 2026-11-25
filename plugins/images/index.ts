// The Images plugin's entry (see src/plugins/api.ts): a small viewer for PNG, JPEG, HEIC and WebP with
// a few touch-up tools. It ships with the app and is always on.

import type { PluginModule } from '../../src/plugins/api'
import { mountViewer } from './viewer'

export const apiVersion: PluginModule['apiVersion'] = 1

export const mount: PluginModule['mount'] = (element, context) => mountViewer(element, context)
