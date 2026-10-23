import { describe, expect, it } from 'vitest'
import { cloudAttachmentSpec } from '../src/cloud-attachments'

describe('Cloud attachment selection', () => {
  it.each([
    ['photo.jpg', 'image/jpeg', 'image', 5_000_000, true],
    ['clip.mp4', 'video/mp4', 'video', 16_000_000, true],
    ['recording.ogg', 'audio/ogg;codecs=opus', 'audio', 16_000_000, false],
    ['sticker.webp', 'image/webp', 'sticker', 500_000, false],
    ['report.pdf', 'application/pdf', 'document', 64 * 1024 * 1024, true],
  ])('maps %s to the installed upload contract', (name, type, mediaType, maxBytes, caption) => {
    expect(cloudAttachmentSpec({ name, type, size: 100 })).toMatchObject({ mediaType, maxBytes, caption })
  })
  it('respects a smaller active worker cap and checks Unicode filename bytes', () => {
    expect(cloudAttachmentSpec({ name: 'report.txt', type: '', size: 100 }, 100).maxBytes).toBe(100)
    expect(() => cloudAttachmentSpec({ name: 'report.txt', type: '', size: 101 }, 100)).toThrow('exceeds')
    expect(() => cloudAttachmentSpec({ name: `${'é'.repeat(128)}.txt`, type: 'text/plain', size: 1 })).toThrow('255 UTF-8 bytes')
  })
  it.each([
    { name: 'report.md', type: 'text/plain', size: 1 },
    { name: 'report.txt', type: 'text/markdown', size: 1 },
    { name: 'report.pdf', type: 'image/png', size: 1 },
    { name: '/tmp/report.txt', type: 'text/plain', size: 1 },
    { name: 'report.txt', type: 'text/plain', size: 0 },
  ])('rejects unsupported or mismatched input without reading bytes: $name/$type', (file) => {
    expect(() => cloudAttachmentSpec(file)).toThrow()
  })
})
