/**
 * The SGR parser's own checks: the token mapping (red→alert,
 * green→ok, yellow→warn, blue/cyan/magenta→accent, bold→600),
 * reset/nesting semantics, stripping of everything it does not map
 * (backgrounds, 256-color, cursor movement, OSC), malformed input,
 * and linear-time safety on huge input.
 */
import { describe, expect, it } from 'vitest'
import { type AnsiSpan, parseAnsi } from '../ansi'

const ESC = '\x1b'

function spans(input: string): AnsiSpan[] {
  return parseAnsi(input)
}

describe('parseAnsi', () => {
  it('returns plain text as a single unstyled span', () => {
    expect(spans('hello world')).toEqual([{ text: 'hello world' }])
  })

  it('returns no spans for the empty string', () => {
    expect(spans('')).toEqual([])
  })

  it('maps the four token tones', () => {
    expect(spans(`${ESC}[31mred${ESC}[0m`)).toEqual([{ text: 'red', color: 'alert' }])
    expect(spans(`${ESC}[32mgreen${ESC}[0m`)).toEqual([{ text: 'green', color: 'ok' }])
    expect(spans(`${ESC}[33myellow${ESC}[0m`)).toEqual([{ text: 'yellow', color: 'warn' }])
    for (const code of [34, 35, 36]) {
      expect(spans(`${ESC}[${code}mx${ESC}[0m`)).toEqual([{ text: 'x', color: 'accent' }])
    }
  })

  it('maps the bright variants like their base colors', () => {
    expect(spans(`${ESC}[91mx`)).toEqual([{ text: 'x', color: 'alert' }])
    expect(spans(`${ESC}[92mx`)).toEqual([{ text: 'x', color: 'ok' }])
    expect(spans(`${ESC}[93mx`)).toEqual([{ text: 'x', color: 'warn' }])
    expect(spans(`${ESC}[96mx`)).toEqual([{ text: 'x', color: 'accent' }])
  })

  it('leaves black/white (30/37/90/97) unstyled — the pane ink is the default', () => {
    expect(spans(`${ESC}[30mx${ESC}[37my${ESC}[97mz`)).toEqual([{ text: 'xyz' }])
  })

  it('applies bold, and combined bold+color params', () => {
    expect(spans(`${ESC}[1mloud${ESC}[0m`)).toEqual([{ text: 'loud', bold: true }])
    expect(spans(`${ESC}[1;31mfailure${ESC}[0m`)).toEqual([{ text: 'failure', color: 'alert', bold: true }])
  })

  it('nests: color, then bold on top, then full reset', () => {
    expect(spans(`${ESC}[32mok ${ESC}[1mstrong${ESC}[0m plain`)).toEqual([
      { text: 'ok ', color: 'ok' },
      { text: 'strong', color: 'ok', bold: true },
      { text: ' plain' },
    ])
  })

  it('39 resets the color but keeps bold', () => {
    expect(spans(`${ESC}[1;31mx${ESC}[39my${ESC}[0mz`)).toEqual([
      { text: 'x', color: 'alert', bold: true },
      { text: 'y', bold: true },
      { text: 'z' },
    ])
  })

  it('treats an empty parameter list (ESC[m) as reset', () => {
    expect(spans(`${ESC}[31mred${ESC}[mplain`)).toEqual([{ text: 'red', color: 'alert' }, { text: 'plain' }])
  })

  it('strips unknown SGR codes without changing state', () => {
    // 4 = underline, 44 = bg blue, 38;5;196 = 256-color — all out of the
    // subset. The 38 introducer consumes its 5;196 arguments whole, so
    // no color claim is made.
    expect(spans(`${ESC}[4mx${ESC}[44my${ESC}[38;5;196mz`)).toEqual([{ text: 'xyz' }])
  })

  it('consumes extended-color introducer params — 38;5;31 never reads 31', () => {
    expect(spans(`${ESC}[38;5;31mx`)).toEqual([{ text: 'x' }])
    expect(spans(`${ESC}[38;2;255;0;0mfg truecolor`)).toEqual([{ text: 'fg truecolor' }])
    expect(spans(`${ESC}[48;5;32mbg 256-color`)).toEqual([{ text: 'bg 256-color' }])
    expect(spans(`${ESC}[58;5;33munderline color`)).toEqual([{ text: 'underline color' }])
    // A real code after the consumed sequence still applies.
    expect(spans(`${ESC}[48;2;0;255;0;31mx`)).toEqual([{ text: 'x', color: 'alert' }])
  })

  it('strips non-SGR CSI and OSC sequences entirely', () => {
    // Cursor-up, erase-line, and an OSC window title must all vanish.
    expect(spans(`${ESC}[2Aabove${ESC}[2K cleared`)).toEqual([{ text: 'above cleared' }])
    expect(spans(`${ESC}]0;title${'\x07'}body`)).toEqual([{ text: 'body' }])
  })

  it('keeps a dangling (malformed) escape as-is rather than eating text', () => {
    // A bare ESC with no bracket fails every alternative; the visible
    // text after it must survive.
    const out = spans(`${ESC}oops kept`)
    expect(out.map((s) => s.text).join('')).toBe(`${ESC}oops kept`)
  })

  it('survives a style left open at end of input', () => {
    expect(spans(`${ESC}[31mnever reset`)).toEqual([{ text: 'never reset', color: 'alert' }])
  })

  it('round-trips the text of a fragment-heavy string', () => {
    const input = `a${ESC}[31mb${ESC}[32mc${ESC}[0md${ESC}[1me`
    expect(
      spans(input)
        .map((s) => s.text)
        .join(''),
    ).toBe('abcde')
  })

  it('merges adjacent same-style runs into one span', () => {
    // Two consecutive red segments (with a stripped unknown code in
    // between) must not fragment the span list.
    expect(spans(`${ESC}[31mab${ESC}[4mcd`)).toEqual([{ text: 'abcd', color: 'alert' }])
  })

  it('handles huge input in linear time without span explosion', () => {
    const chunk = `${ESC}[31mx${ESC}[0my`
    const input = chunk.repeat(200_000) // ~2.6 MB, 400k escapes
    const start = Date.now()
    const out = parseAnsi(input)
    const elapsed = Date.now() - start
    expect(out.map((s) => s.text).join('').length).toBe(400_000)
    // Alternating styles cannot merge — but the span count must stay
    // exactly linear in the number of styled runs, nothing quadratic.
    expect(out.length).toBe(400_000)
    expect(elapsed).toBeLessThan(5_000)
  })

  it('handles a huge unstyled input as a single span', () => {
    const input = 'x'.repeat(5 * 1024 * 1024)
    const out = parseAnsi(input)
    expect(out).toHaveLength(1)
    expect(out[0].text.length).toBe(5 * 1024 * 1024)
  })
})
