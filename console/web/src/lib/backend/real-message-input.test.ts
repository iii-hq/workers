import { describe, expect, it } from 'vitest'
import { buildMessageInput } from './real'

const image = { type: 'image' as const, mime: 'image/png', data: 'AAAA' }
const file = {
  type: 'file' as const,
  attachment_id: 'a_1',
  name: 'report.pdf',
  mime: 'application/pdf',
  size: 10,
}

describe('buildMessageInput', () => {
  /* The string form keeps the common send byte-identical to what it always
     was; anything attached switches to the structured message. */
  it('keeps the string sugar only when nothing is attached', () => {
    expect(buildMessageInput('hi', [], [], [])).toBe('hi')
    expect(buildMessageInput('hi', ['<attached-file />'])).not.toBe('hi')
    expect(buildMessageInput('hi', [], [image])).not.toBe('hi')
    expect(buildMessageInput('hi', [], [], [file])).not.toBe('hi')
  })

  it('orders text, then file references, then images', () => {
    const message = buildMessageInput(
      'hi',
      ['<attached-file />'],
      [image],
      [file],
    )
    expect(message).toMatchObject({
      role: 'user',
      content: [
        { type: 'text', text: 'hi' },
        { type: 'text', text: '<attached-file />' },
        file,
        image,
      ],
    })
  })

  it('carries a file reference alone, without an expansion', () => {
    const message = buildMessageInput('hi', [], [], [file])
    expect(message).toMatchObject({
      content: [{ type: 'text', text: 'hi' }, file],
    })
  })
})
