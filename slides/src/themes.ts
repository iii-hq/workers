import type { Deck, ThemeOverrides } from './model.js'

export interface Theme {
  id: string
  name: string
  description: string
  dark: boolean
  colors: {
    background: string
    surface: string
    ink: string
    muted: string
    accent: string
    accent_ink: string
  }
  fonts: {
    heading: string
    body: string
    mono: string
  }
  heading_weight: number
  radius: number
}

export const THEMES: Theme[] = [
  {
    id: 'midnight',
    name: 'Midnight',
    description: 'Deep navy with an electric blue accent; confident and technical.',
    dark: true,
    colors: {
      background: '#0b1020',
      surface: '#141b33',
      ink: '#f4f6fb',
      muted: '#9aa5c4',
      accent: '#4f8cff',
      accent_ink: '#ffffff',
    },
    fonts: { heading: 'Inter', body: 'Inter', mono: 'JetBrains Mono' },
    heading_weight: 700,
    radius: 16,
  },
  {
    id: 'paper',
    name: 'Paper',
    description: 'Warm off-white with ink type and a burnt-orange accent; editorial.',
    dark: false,
    colors: {
      background: '#faf7f2',
      surface: '#ffffff',
      ink: '#1c1a17',
      muted: '#6f6a63',
      accent: '#d1552a',
      accent_ink: '#ffffff',
    },
    fonts: { heading: 'Playfair Display', body: 'Source Sans 3', mono: 'JetBrains Mono' },
    heading_weight: 600,
    radius: 8,
  },
  {
    id: 'aurora',
    name: 'Aurora',
    description: 'Near-black with a violet-to-teal gradient accent; launch-ready.',
    dark: true,
    colors: {
      background: '#070912',
      surface: '#111527',
      ink: '#f7f5ff',
      muted: '#a39fc2',
      accent: '#8b5cf6',
      accent_ink: '#ffffff',
    },
    fonts: { heading: 'Space Grotesk', body: 'Inter', mono: 'JetBrains Mono' },
    heading_weight: 700,
    radius: 20,
  },
  {
    id: 'slate',
    name: 'Slate',
    description: 'Cool light grays with a graphite accent; calm and corporate.',
    dark: false,
    colors: {
      background: '#f3f5f8',
      surface: '#ffffff',
      ink: '#1f2933',
      muted: '#627084',
      accent: '#2f6f9f',
      accent_ink: '#ffffff',
    },
    fonts: { heading: 'IBM Plex Sans', body: 'IBM Plex Sans', mono: 'IBM Plex Mono' },
    heading_weight: 600,
    radius: 10,
  },
  {
    id: 'sunrise',
    name: 'Sunrise',
    description: 'Cream background with coral and gold; optimistic and friendly.',
    dark: false,
    colors: {
      background: '#fff8ef',
      surface: '#ffffff',
      ink: '#2a1d16',
      muted: '#7d6a5c',
      accent: '#ff6b57',
      accent_ink: '#ffffff',
    },
    fonts: { heading: 'Poppins', body: 'Nunito', mono: 'JetBrains Mono' },
    heading_weight: 700,
    radius: 24,
  },
  {
    id: 'forest',
    name: 'Forest',
    description: 'Dark evergreen with a lime accent; grounded and bold.',
    dark: true,
    colors: {
      background: '#0d1b14',
      surface: '#15291f',
      ink: '#eef5ef',
      muted: '#9db5a4',
      accent: '#a3e635',
      accent_ink: '#0d1b14',
    },
    fonts: { heading: 'Manrope', body: 'Manrope', mono: 'JetBrains Mono' },
    heading_weight: 800,
    radius: 14,
  },
]

export const DEFAULT_THEME = 'midnight'

export function themeById(id: string | undefined): Theme {
  return THEMES.find((theme) => theme.id === id) ?? (THEMES.find((theme) => theme.id === DEFAULT_THEME) as Theme)
}

export function isThemeId(id: unknown): id is string {
  return typeof id === 'string' && THEMES.some((theme) => theme.id === id)
}

export interface ResolvedTheme extends Theme {
  footer?: string
}

export function resolveTheme(deck: Pick<Deck, 'theme' | 'theme_overrides'>, brand?: ThemeOverrides): ResolvedTheme {
  const base = themeById(deck.theme)
  const overrides: ThemeOverrides = { ...(brand ?? {}), ...(deck.theme_overrides ?? {}) }
  const colors = { ...base.colors }
  if (overrides.accent && isColor(overrides.accent)) {
    colors.accent = overrides.accent
    colors.accent_ink = readableOn(overrides.accent)
  }
  if (overrides.background && isColor(overrides.background)) colors.background = overrides.background
  if (overrides.ink && isColor(overrides.ink)) colors.ink = overrides.ink
  const dark = relativeLuminance(colors.background) < 0.4
  return {
    ...base,
    dark,
    colors: dark === base.dark ? colors : { ...colors, surface: dark ? mix(colors.background, '#ffffff', 0.08) : '#ffffff' },
    fonts: {
      heading: overrides.font_heading ?? base.fonts.heading,
      body: overrides.font_body ?? base.fonts.body,
      mono: base.fonts.mono,
    },
    ...(overrides.footer ? { footer: overrides.footer } : {}),
  }
}

export function isColor(value: string): boolean {
  return /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(value.trim())
}

export function hexToRgb(hex: string): [number, number, number] {
  let value = hex.trim().replace('#', '')
  if (value.length === 3)
    value = value
      .split('')
      .map((char) => char + char)
      .join('')
  const int = Number.parseInt(value, 16)
  return [(int >> 16) & 255, (int >> 8) & 255, int & 255]
}

export function relativeLuminance(hex: string): number {
  const [r, g, b] = hexToRgb(hex).map((channel) => {
    const c = channel / 255
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
  })
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}

export function readableOn(hex: string): string {
  return relativeLuminance(hex) > 0.5 ? '#111111' : '#ffffff'
}

export function mix(a: string, b: string, amount: number): string {
  const [ar, ag, ab] = hexToRgb(a)
  const [br, bg, bb] = hexToRgb(b)
  const channel = (x: number, y: number) => Math.round(x + (y - x) * amount)
  return `#${[channel(ar, br), channel(ag, bg), channel(ab, bb)].map((c) => c.toString(16).padStart(2, '0')).join('')}`
}

export function withAlpha(hex: string, alpha: number): string {
  const [r, g, b] = hexToRgb(hex)
  return `rgba(${r}, ${g}, ${b}, ${alpha})`
}

export function googleFontsHref(theme: Theme): string {
  const families = [...new Set([theme.fonts.heading, theme.fonts.body, theme.fonts.mono])]
  const query = families.map((family) => `family=${encodeURIComponent(family).replace(/%20/g, '+')}:wght@400;500;600;700;800`)
  return `https://fonts.googleapis.com/css2?${query.join('&')}&display=swap`
}
