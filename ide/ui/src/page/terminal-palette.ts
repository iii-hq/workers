export interface TerminalAnsiPalette {
  black: string
  red: string
  green: string
  yellow: string
  blue: string
  magenta: string
  cyan: string
  white: string
  brightBlack: string
  brightRed: string
  brightGreen: string
  brightYellow: string
  brightBlue: string
  brightMagenta: string
  brightCyan: string
  brightWhite: string
}

const DARK_PALETTE: TerminalAnsiPalette = {
  black: '#484f58',
  red: '#ff7b72',
  green: '#3fb950',
  yellow: '#d29922',
  blue: '#58a6ff',
  magenta: '#bc8cff',
  cyan: '#39c5cf',
  white: '#b1bac4',
  brightBlack: '#6e7681',
  brightRed: '#ffa198',
  brightGreen: '#56d364',
  brightYellow: '#e3b341',
  brightBlue: '#79c0ff',
  brightMagenta: '#d2a8ff',
  brightCyan: '#56d4dd',
  brightWhite: '#f0f6fc',
}

const LIGHT_PALETTE: TerminalAnsiPalette = {
  black: '#24292f',
  red: '#cf222e',
  green: '#116329',
  yellow: '#7d4e00',
  blue: '#0550ae',
  magenta: '#8250df',
  cyan: '#1b7c83',
  white: '#6e7781',
  brightBlack: '#57606a',
  brightRed: '#a40e26',
  brightGreen: '#0f5323',
  brightYellow: '#633c01',
  brightBlue: '#0a3069',
  brightMagenta: '#6639ba',
  brightCyan: '#106b70',
  brightWhite: '#24292f',
}

export function isLightColor(value: string): boolean {
  const parsed = parseColor(value)
  if (!parsed) return false
  const [r, g, b] = parsed
  return (r * 299 + g * 587 + b * 114) / 1000 > 140
}

export function terminalAnsiPalette(background: string): TerminalAnsiPalette {
  return isLightColor(background) ? LIGHT_PALETTE : DARK_PALETTE
}

function parseColor(value: string): [number, number, number] | null {
  const text = value.trim()
  const rgb = text.match(
    /^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)/i,
  )
  if (rgb) {
    return [Number(rgb[1]), Number(rgb[2]), Number(rgb[3])]
  }
  const hex = text.match(/^#([0-9a-f]{3,8})$/i)
  if (!hex) return null
  const digits = hex[1]
  if (digits.length === 3 || digits.length === 4) {
    const [r, g, b] = [...digits.slice(0, 3)].map((digit) =>
      Number.parseInt(`${digit}${digit}`, 16),
    )
    return [r, g, b]
  }
  if (digits.length === 6 || digits.length === 8) {
    return [
      Number.parseInt(digits.slice(0, 2), 16),
      Number.parseInt(digits.slice(2, 4), 16),
      Number.parseInt(digits.slice(4, 6), 16),
    ]
  }
  return null
}
