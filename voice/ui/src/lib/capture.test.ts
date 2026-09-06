import { describe, expect, it } from 'vitest'
import { resampleTo16kMonoInt16 } from './capture'

describe('resampleTo16kMonoInt16', () => {
  it('keeps length unchanged when already 16kHz', () => {
    const input = new Float32Array([0, 0.25, -0.25, 1, -1])
    const out = resampleTo16kMonoInt16(input, 16000)
    expect(out.length).toBe(input.length)
  })

  it('clamps out-of-range samples to the int16 extremes', () => {
    const out = resampleTo16kMonoInt16(new Float32Array([2, -2, 0]), 16000)
    expect(out[0]).toBe(32767)
    expect(out[1]).toBe(-32768)
    expect(out[2]).toBe(0)
  })

  it('downsamples 48kHz to exactly 1600 samples for a 100ms batch', () => {
    const inputRate = 48000
    const input = new Float32Array(inputRate / 10)
    for (let i = 0; i < input.length; i++) {
      input[i] = Math.sin((i / inputRate) * 2 * Math.PI * 440)
    }
    const out = resampleTo16kMonoInt16(input, inputRate)
    expect(out.length).toBe(1600)
  })

  it('produces silence for a silent input', () => {
    const out = resampleTo16kMonoInt16(new Float32Array(4800), 48000)
    expect(out.every((sample) => sample === 0)).toBe(true)
  })

  it('is pure: the same input always produces the same output', () => {
    const input = new Float32Array([0.1, -0.4, 0.9, -0.9, 0.02])
    const a = resampleTo16kMonoInt16(input, 44100)
    const b = resampleTo16kMonoInt16(input, 44100)
    expect(Array.from(a)).toEqual(Array.from(b))
    expect(input).toEqual(new Float32Array([0.1, -0.4, 0.9, -0.9, 0.02]))
  })
})
