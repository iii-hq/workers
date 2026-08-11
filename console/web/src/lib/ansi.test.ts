import { describe, expect, it } from 'vitest'
import { ANSI_PARSE_LIMIT, parseAnsi } from './ansi'

const ESC = '\u001b'

describe('parseAnsi', () => {
  it('returns plain text as a single unstyled segment', () => {
    expect(parseAnsi('hello world\n')).toEqual([{ text: 'hello world\n' }])
  })

  it('returns [] for empty input', () => {
    expect(parseAnsi('')).toEqual([])
  })

  it('maps the color subset onto tones', () => {
    expect(parseAnsi(`${ESC}[31mred`)).toEqual([{ text: 'red', tone: 'alert' }])
    expect(parseAnsi(`${ESC}[32mgreen`)).toEqual([
      { text: 'green', tone: 'ok' },
    ])
    expect(parseAnsi(`${ESC}[33myellow`)).toEqual([
      { text: 'yellow', tone: 'warn' },
    ])
    expect(parseAnsi(`${ESC}[34mblue`)).toEqual([
      { text: 'blue', tone: 'accent' },
    ])
    expect(parseAnsi(`${ESC}[35mmagenta`)).toEqual([
      { text: 'magenta', tone: 'accent' },
    ])
    expect(parseAnsi(`${ESC}[36mcyan`)).toEqual([
      { text: 'cyan', tone: 'accent' },
    ])
  })

  it('maps bright colors (90-97) the same as the base ramp', () => {
    expect(parseAnsi(`${ESC}[91mred`)).toEqual([{ text: 'red', tone: 'alert' }])
    expect(parseAnsi(`${ESC}[92mgreen`)).toEqual([
      { text: 'green', tone: 'ok' },
    ])
    expect(parseAnsi(`${ESC}[96mcyan`)).toEqual([
      { text: 'cyan', tone: 'accent' },
    ])
  })

  it('renders black and white as default ink', () => {
    expect(parseAnsi(`${ESC}[30mblack`)).toEqual([{ text: 'black' }])
    expect(parseAnsi(`${ESC}[37mwhite`)).toEqual([{ text: 'white' }])
    expect(parseAnsi(`${ESC}[97mbright white`)).toEqual([
      { text: 'bright white' },
    ])
  })

  it('tracks bold and nests it with color', () => {
    expect(parseAnsi(`${ESC}[31mred ${ESC}[1mbold red${ESC}[0m plain`)).toEqual(
      [
        { text: 'red ', tone: 'alert' },
        { text: 'bold red', tone: 'alert', bold: true },
        { text: ' plain' },
      ],
    )
  })

  it('resets color only on 39, bold only on 22', () => {
    expect(parseAnsi(`${ESC}[1;31mx${ESC}[39my${ESC}[22mz`)).toEqual([
      { text: 'x', tone: 'alert', bold: true },
      { text: 'y', bold: true },
      { text: 'z' },
    ])
  })

  it('treats a bare ESC[m as a full reset', () => {
    expect(parseAnsi(`${ESC}[1;33mx${ESC}[my`)).toEqual([
      { text: 'x', tone: 'warn', bold: true },
      { text: 'y' },
    ])
  })

  it('consumes 256-color params instead of misreading them as codes', () => {
    // Without consumption, the `31` in `38;5;31` would read as red.
    expect(parseAnsi(`${ESC}[38;5;31mx`)).toEqual([{ text: 'x' }])
    expect(parseAnsi(`${ESC}[48;5;31mx`)).toEqual([{ text: 'x' }])
  })

  it('consumes truecolor params instead of misreading them as codes', () => {
    expect(parseAnsi(`${ESC}[38;2;255;0;0mx`)).toEqual([{ text: 'x' }])
    expect(parseAnsi(`${ESC}[58;2;33;33;33mx`)).toEqual([{ text: 'x' }])
  })

  it('applies codes that follow a consumed extended color', () => {
    expect(parseAnsi(`${ESC}[38;5;31;33mx`)).toEqual([
      { text: 'x', tone: 'warn' },
    ])
  })

  it('treats colon-form extended colors as self-contained', () => {
    expect(parseAnsi(`${ESC}[38:5:196mx${ESC}[31my`)).toEqual([
      { text: 'x' },
      { text: 'y', tone: 'alert' },
    ])
  })

  it('never throws on malformed extended colors', () => {
    expect(parseAnsi(`${ESC}[38mx`)).toEqual([{ text: 'x' }])
    expect(parseAnsi(`${ESC}[38;6;31mx`)).toEqual([{ text: 'x' }])
  })

  it('strips non-SGR CSI sequences and merges the surrounding text', () => {
    expect(parseAnsi(`a${ESC}[2Jb${ESC}[1;1Hc`)).toEqual([{ text: 'abc' }])
  })

  it('strips OSC sequences (BEL- and ST-terminated)', () => {
    expect(parseAnsi(`${ESC}]0;title\u0007text`)).toEqual([{ text: 'text' }])
    expect(parseAnsi(`${ESC}]8;;https://x${ESC}\\link`)).toEqual([
      { text: 'link' },
    ])
  })

  it('drops unterminated sequences and lone trailing escapes', () => {
    expect(parseAnsi(`abc${ESC}[31`)).toEqual([{ text: 'abc' }])
    expect(parseAnsi(`abc${ESC}]0;title`)).toEqual([{ text: 'abc' }])
    expect(parseAnsi(`abc${ESC}`)).toEqual([{ text: 'abc' }])
  })

  it('ignores SGR codes outside the subset', () => {
    // Underline (4) and backgrounds (41) carry no tone.
    expect(parseAnsi(`${ESC}[4mx${ESC}[41my`)).toEqual([{ text: 'xy' }])
  })

  it('bails to one stripped plain segment past the parse cap', () => {
    const text = `${'x'.repeat(ANSI_PARSE_LIMIT)}${ESC}[31mred`
    const segments = parseAnsi(text)
    expect(segments).toHaveLength(1)
    expect(segments[0].tone).toBeUndefined()
    expect(segments[0].text).not.toContain(ESC)
    expect(segments[0].text.endsWith('red')).toBe(true)
  })

  it('stays linear-ish on pathological escape soup without throwing', () => {
    const soup = `${ESC}[31m${ESC}[`.repeat(10_000)
    expect(() => parseAnsi(soup)).not.toThrow()
  })
})
