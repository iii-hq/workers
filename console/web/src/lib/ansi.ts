/**
 * Minimal ANSI SGR parser behind the shared `AnsiText` component — the
 * console's one mapping from terminal escape codes to design tokens.
 *
 * Deliberately a subset, not a terminal emulator: the SGR codes worker
 * CLIs actually color with (0, 1, 22, 30-37, 90-97, 39) map onto tones;
 * extended-color introducers (38/48/58) are consumed together with their
 * `5;n` / `2;r;g;b` params so the params are never misread as standalone
 * codes; every other CSI/OSC sequence is stripped so raw `\x1b[` bytes
 * never leak into rendered text. Pathological input bails to stripped
 * plain text past `ANSI_PARSE_LIMIT` instead of building span soup.
 */

/** Design-token tone a colored segment renders with. */
export type AnsiTone = 'alert' | 'ok' | 'warn' | 'accent'

export interface AnsiSegment {
  text: string
  /** Absent = default ink (black/white/default-fg codes land here too). */
  tone?: AnsiTone
  bold?: boolean
}

/** Past this many chars the parse bails to one stripped plain segment. */
export const ANSI_PARSE_LIMIT = 200_000

/* Fallback strip for the bail path — single-pass regexes, no spans.
   0x1B is the ANSI ESC introducer; 0x07 the BEL terminator for OSC. */
// biome-ignore lint/suspicious/noControlCharactersInRegex: stripping ANSI CSI sequences.
const CSI_REGEX = /\x1b\[[0-9:;<=>?]*[ -/]*[@-~]/g
// biome-ignore lint/suspicious/noControlCharactersInRegex: stripping ANSI OSC (ESC ] … BEL/ST) sequences.
const OSC_REGEX = /\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)?/g
// biome-ignore lint/suspicious/noControlCharactersInRegex: stripping stray ESC introducers.
const ESC_REGEX = /\x1b.?/g

function stripAnsi(s: string): string {
  return s.replace(CSI_REGEX, '').replace(OSC_REGEX, '').replace(ESC_REGEX, '')
}

function toneForColorCode(code: number): AnsiTone | undefined {
  switch (code) {
    case 31:
    case 91:
      return 'alert'
    case 32:
    case 92:
      return 'ok'
    case 33:
    case 93:
      return 'warn'
    case 34:
    case 94:
    case 35:
    case 95:
    case 36:
    case 96:
      return 'accent'
    default:
      // Black (30/90), white (37/97): default ink — the console's text
      // color IS the terminal foreground, in both themes.
      return undefined
  }
}

interface SgrState {
  tone: AnsiTone | undefined
  bold: boolean
}

/** Apply one SGR parameter string (the bytes between `ESC[` and `m`). */
function applySgr(raw: string, state: SgrState): void {
  const params = raw.split(';')
  for (let k = 0; k < params.length; k++) {
    const param = params[k]
    // Empty param = 0 per the spec (`ESC[m` is a reset); the colon form
    // (`38:5:196`) parses to its introducer and carries its own params.
    const parsed = Number.parseInt(param, 10)
    const code = Number.isNaN(parsed) ? 0 : parsed
    switch (code) {
      case 0:
        state.tone = undefined
        state.bold = false
        break
      case 1:
        state.bold = true
        break
      case 22:
        state.bold = false
        break
      case 39:
        state.tone = undefined
        break
      case 38:
      case 48:
      case 58: {
        // Colon sub-params are self-contained in this one param.
        if (param.includes(':')) break
        const mode = params[k + 1]
        if (mode === '5') {
          k += 2
        } else if (mode === '2') {
          k += 4
        } else {
          // Malformed extended color — stop rather than misread its
          // params as standalone codes.
          k = params.length
        }
        break
      }
      default:
        if ((code >= 30 && code <= 37) || (code >= 90 && code <= 97)) {
          state.tone = toneForColorCode(code)
        }
        // Backgrounds, underline, everything else: ignored.
        break
    }
  }
}

/**
 * Parse terminal output into styled segments. Never throws; malformed or
 * unterminated sequences drop silently, and adjacent same-style runs
 * merge so plain output stays one segment.
 */
export function parseAnsi(text: string): AnsiSegment[] {
  if (text.length === 0) return []
  if (text.length > ANSI_PARSE_LIMIT) return [{ text: stripAnsi(text) }]

  const segments: AnsiSegment[] = []
  const state: SgrState = { tone: undefined, bold: false }
  let plainStart = 0
  let i = 0

  const flush = (end: number) => {
    if (end <= plainStart) return
    const chunk = text.slice(plainStart, end)
    const last = segments[segments.length - 1]
    if (
      last &&
      last.tone === state.tone &&
      (last.bold ?? false) === state.bold
    ) {
      last.text += chunk
      return
    }
    const segment: AnsiSegment = { text: chunk }
    if (state.tone !== undefined) segment.tone = state.tone
    if (state.bold) segment.bold = true
    segments.push(segment)
  }

  while (i < text.length) {
    const esc = text.indexOf('\u001b', i)
    if (esc === -1) break
    flush(esc)
    const introducer = text[esc + 1]
    if (introducer === '[') {
      // CSI: params 0x30-0x3F, intermediates 0x20-0x2F, final 0x40-0x7E.
      let j = esc + 2
      while (j < text.length) {
        const c = text.charCodeAt(j)
        if (c < 0x30 || c > 0x3f) break
        j++
      }
      const paramsEnd = j
      while (j < text.length) {
        const c = text.charCodeAt(j)
        if (c < 0x20 || c > 0x2f) break
        j++
      }
      if (j < text.length) {
        if (text[j] === 'm') applySgr(text.slice(esc + 2, paramsEnd), state)
        i = j + 1
      } else {
        // Unterminated CSI — drop the tail.
        i = text.length
      }
    } else if (introducer === ']') {
      // OSC: runs to BEL or ST (ESC \); unterminated swallows the tail —
      // the sequence's body is control payload, never visible text.
      let j = esc + 2
      let end = text.length
      while (j < text.length) {
        const c = text.charCodeAt(j)
        if (c === 0x07) {
          end = j + 1
          break
        }
        if (c === 0x1b && text[j + 1] === '\\') {
          end = j + 2
          break
        }
        j++
      }
      i = end
    } else if (introducer === undefined) {
      // Lone trailing ESC.
      i = text.length
    } else {
      // Other ESC sequence (charset selection, …): ESC + one byte.
      i = esc + 2
    }
    plainStart = i
  }
  flush(text.length)
  return segments
}
