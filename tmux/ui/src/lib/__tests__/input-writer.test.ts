import { describe, expect, it } from 'vitest'
import { binaryStringToBytes, createInputWriter, decodeBase64, encodeBase64 } from '../input-writer'

function deferred() {
  let resolve: () => void = () => undefined
  const promise = new Promise<void>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

const bytes = (text: string) => new TextEncoder().encode(text)
const text = (data: Uint8Array) => new TextDecoder().decode(data)
const flush = () => new Promise((resolve) => setTimeout(resolve, 0))

describe('createInputWriter', () => {
  it('sends the first keystroke at once and coalesces what is typed while it is in flight', async () => {
    const sent: string[] = []
    const gates: ReturnType<typeof deferred>[] = []
    const write = createInputWriter((data) => {
      sent.push(text(data))
      const gate = deferred()
      gates.push(gate)
      return gate.promise
    })
    write(bytes('l'))
    write(bytes('s'))
    write(bytes(' -la'))
    expect(sent).toEqual(['l'])
    gates[0].resolve()
    await flush()
    expect(sent).toEqual(['l', 's -la'])
    gates[1].resolve()
    await flush()
    expect(sent).toEqual(['l', 's -la'])
  })

  it('splits a paste into chunks no larger than the cap, in order', async () => {
    const sent: Uint8Array[] = []
    const write = createInputWriter(async (data) => {
      sent.push(data)
    }, 4)
    write(bytes('abcdefghij'))
    await flush()
    await flush()
    await flush()
    expect(sent.map(text)).toEqual(['abcd', 'efgh', 'ij'])
    expect(Math.max(...sent.map((chunk) => chunk.length))).toBeLessThanOrEqual(4)
  })

  it('keeps going after a failed send', async () => {
    const sent: string[] = []
    let fail = true
    const write = createInputWriter(async (data) => {
      sent.push(text(data))
      if (fail) {
        fail = false
        throw new Error('gone')
      }
    })
    write(bytes('a'))
    write(bytes('b'))
    await flush()
    await flush()
    expect(sent).toEqual(['a', 'b'])
  })

  it('ignores empty input', async () => {
    const sent: Uint8Array[] = []
    const write = createInputWriter(async (data) => {
      sent.push(data)
    })
    write(new Uint8Array())
    await flush()
    expect(sent).toEqual([])
  })
})

describe('base64 helpers', () => {
  it('round-trips arbitrary bytes, utf-8 included', () => {
    const data = bytes('olá ✳ \u001b[31mred\u001b[0m')
    expect(decodeBase64(encodeBase64(data))).toEqual(data)
  })

  it('keeps only the low byte of a binary string', () => {
    expect([...binaryStringToBytes('ÿĀA')]).toEqual([0xff, 0x00, 0x41])
  })
})
