export const INTEGRATED_FONT_IDS = [
  'russh-default',
  'inter',
  'open-sans',
  'source-sans-pro',
  'jetbrains-mono',
  'fira-code',
  'roboto-mono',
  'source-code-pro',
  'cascadia-mono',
  'cascadia-code',
  'ibm-plex-mono',
  'ubuntu-mono',
  'ubuntu-sans-mono',
  'pt-mono',
  'vt323'
] as const

export type IntegratedFontId = (typeof INTEGRATED_FONT_IDS)[number]

export const DEFAULT_UI_FONT_ID: IntegratedFontId = 'russh-default'
export const DEFAULT_TERMINAL_FONT_ID: IntegratedFontId = 'jetbrains-mono'
export const DEFAULT_EDITOR_FONT_ID: IntegratedFontId | null = null

export type IntegratedFontScope = 'ui' | 'terminal' | 'editor'

export const MONOSPACE_FONT_IDS: string[] = [
  'jetbrains-mono',
  'fira-code',
  'roboto-mono',
  'source-code-pro',
  'cascadia-mono',
  'cascadia-code',
  'ibm-plex-mono',
  'ubuntu-mono',
  'ubuntu-sans-mono',
  'pt-mono',
  'vt323'
]

export function isMonospaceFontId(value: unknown): boolean {
  return typeof value === 'string' && MONOSPACE_FONT_IDS.includes(value)
}

export interface IntegratedFontDefinition {
  cssFamily: string
  id: IntegratedFontId
  label: string
  legacyNames: string[]
  scopes: IntegratedFontScope[]
}

const ALL_SCOPES: IntegratedFontScope[] = ['ui', 'terminal', 'editor']

export const INTEGRATED_FONTS: IntegratedFontDefinition[] = [
  {
    cssFamily: 'RuSSH UI Default',
    id: 'russh-default',
    label: 'RuSSH Default',
    legacyNames: [
      'segoe ui variable text',
      'segoe ui',
      'microsoft yahei ui',
      'microsoft yahei',
      'system-ui',
      'ui-sans-serif',
      'sans-serif'
    ],
    scopes: ALL_SCOPES
  },
  {
    cssFamily: 'RuSSH UI Inter',
    id: 'inter',
    label: 'Inter',
    legacyNames: ['inter'],
    scopes: ALL_SCOPES
  },
  {
    cssFamily: 'RuSSH UI Open Sans',
    id: 'open-sans',
    label: 'Open Sans',
    legacyNames: ['open sans'],
    scopes: ALL_SCOPES
  },
  {
    cssFamily: 'RuSSH UI Source Sans Pro',
    id: 'source-sans-pro',
    label: 'Source Sans Pro',
    legacyNames: ['source sans pro'],
    scopes: ALL_SCOPES
  },
  {
    cssFamily: 'RuSSH JetBrains Mono',
    id: 'jetbrains-mono',
    label: 'JetBrains Mono',
    legacyNames: ['jetbrains mono'],
    scopes: ALL_SCOPES
  },
  {
    cssFamily: 'RuSSH Fira Code',
    id: 'fira-code',
    label: 'Fira Code',
    legacyNames: ['fira code'],
    scopes: ALL_SCOPES
  },
  {
    cssFamily: 'RuSSH Roboto Mono',
    id: 'roboto-mono',
    label: 'Roboto Mono',
    legacyNames: ['roboto mono'],
    scopes: ALL_SCOPES
  },
  {
    cssFamily: 'RuSSH Source Code Pro',
    id: 'source-code-pro',
    label: 'Source Code Pro',
    legacyNames: ['source code pro'],
    scopes: ALL_SCOPES
  },
  {
    cssFamily: 'RuSSH Cascadia Mono',
    id: 'cascadia-mono',
    label: 'Cascadia Mono',
    legacyNames: ['cascadia mono', 'consolas', 'lucida console'],
    scopes: ALL_SCOPES
  },
  {
    cssFamily: 'RuSSH Cascadia Code',
    id: 'cascadia-code',
    label: 'Cascadia Code',
    legacyNames: ['cascadia code'],
    scopes: ALL_SCOPES
  },
  {
    cssFamily: 'RuSSH IBM Plex Mono',
    id: 'ibm-plex-mono',
    label: 'IBM Plex Mono',
    legacyNames: ['ibm plex mono'],
    scopes: ALL_SCOPES
  },
  {
    cssFamily: 'RuSSH Ubuntu Mono',
    id: 'ubuntu-mono',
    label: 'Ubuntu Mono',
    legacyNames: ['ubuntu mono'],
    scopes: ALL_SCOPES
  },
  {
    cssFamily: 'RuSSH Ubuntu Sans Mono',
    id: 'ubuntu-sans-mono',
    label: 'Ubuntu Sans Mono',
    legacyNames: ['ubuntu sans mono'],
    scopes: ALL_SCOPES
  },
  {
    cssFamily: 'RuSSH PT Mono',
    id: 'pt-mono',
    label: 'PT Mono',
    legacyNames: ['pt mono'],
    scopes: ALL_SCOPES
  },
  {
    cssFamily: 'RuSSH VT323',
    id: 'vt323',
    label: 'VT323',
    legacyNames: ['vt323'],
    scopes: ALL_SCOPES
  }
]

export const INTEGRATED_UI_FALLBACK_FAMILIES = ['sans-serif'] as const
export const INTEGRATED_TERMINAL_FALLBACK_FAMILIES = [
  'Consolas',
  'Courier New',
  'monospace'
] as const

const FONT_ID_SET = new Set<string>(INTEGRATED_FONT_IDS)

function normalizeLegacyFontToken(value: string) {
  return value
    .trim()
    .replace(/^['"]|['"]$/g, '')
    .toLowerCase()
}

function splitLegacyFontFamily(value: string) {
  return value
    .split(',')
    .map((token) => normalizeLegacyFontToken(token))
    .filter(Boolean)
}

export function isIntegratedFontId(value: unknown): value is IntegratedFontId {
  return typeof value === 'string' && FONT_ID_SET.has(value)
}

export function normalizeIntegratedFontId(
  value: unknown,
  fallback: IntegratedFontId = DEFAULT_UI_FONT_ID
): IntegratedFontId {
  return isIntegratedFontId(value) ? value : fallback
}

export function normalizeIntegratedUiFontId(value: unknown): IntegratedFontId {
  return normalizeIntegratedFontId(value, DEFAULT_UI_FONT_ID)
}

export function normalizeIntegratedTerminalFontId(value: unknown): IntegratedFontId {
  return isMonospaceFontId(value) ? (value as IntegratedFontId) : DEFAULT_TERMINAL_FONT_ID
}

export function normalizeIntegratedEditorFontId(value: unknown): IntegratedFontId | null {
  return value === null || value === undefined ? null : normalizeIntegratedTerminalFontId(value)
}

export function resolveLegacyTerminalFontId(value: unknown): IntegratedFontId {
  if (isIntegratedFontId(value)) {
    return value
  }

  return resolveLegacyTerminalFontIdOrNull(value) ?? DEFAULT_TERMINAL_FONT_ID
}

export function resolveLegacyTerminalFontIdOrNull(value: unknown): IntegratedFontId | null {
  if (isIntegratedFontId(value)) {
    return value
  }

  if (typeof value !== 'string') {
    return null
  }

  const tokens = splitLegacyFontFamily(value)

  for (const font of INTEGRATED_FONTS) {
    if (font.legacyNames.some((name) => tokens.includes(name))) {
      return font.id
    }
  }

  return null
}

export function getIntegratedFont(id: unknown, fallback: IntegratedFontId = DEFAULT_UI_FONT_ID) {
  const fontId = normalizeIntegratedFontId(id, fallback)
  return INTEGRATED_FONTS.find((font) => font.id === fontId) ?? INTEGRATED_FONTS[0]
}

function quoteFontFamily(value: string) {
  if (/^[A-Za-z0-9_-]+$/.test(value) || value === 'monospace' || value === 'sans-serif') {
    return value
  }

  return `"${value.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`
}

export function formatIntegratedUiFontStack(id: unknown): string {
  const font = getIntegratedFont(id, DEFAULT_UI_FONT_ID)
  return [font.cssFamily, ...INTEGRATED_UI_FALLBACK_FAMILIES].map(quoteFontFamily).join(', ')
}

export function formatIntegratedTerminalFontStack(id: unknown): string {
  const font = getIntegratedFont(id, DEFAULT_TERMINAL_FONT_ID)
  return [font.cssFamily, ...INTEGRATED_TERMINAL_FALLBACK_FAMILIES].map(quoteFontFamily).join(', ')
}
