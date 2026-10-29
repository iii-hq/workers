import { describe, expect, it } from 'vitest'
import { splitForMiddleTruncation } from './middleTruncate'

describe('splitForMiddleTruncation', () => {
  it('keeps the last namespace segment as the tail', () => {
    expect(splitForMiddleTruncation('iii-directory::search_functions')).toEqual(
      { head: 'iii-directory::', tail: 'search_functions' },
    )
    expect(splitForMiddleTruncation('provider::anthropic::stream')).toEqual({
      head: 'provider::anthropic::',
      tail: 'stream',
    })
  })

  it('prefers a namespace cut over a word cut', () => {
    expect(splitForMiddleTruncation('harness::turn step')).toEqual({
      head: 'harness::',
      tail: 'turn step',
    })
  })

  it('falls back to path and dotted segments', () => {
    expect(splitForMiddleTruncation('POST /v1/messages')).toEqual({
      head: 'POST /v1/',
      tail: 'messages',
    })
    expect(splitForMiddleTruncation('chat.respond.stream')).toEqual({
      head: 'chat.respond.',
      tail: 'stream',
    })
  })

  it('leaves short names whole', () => {
    expect(splitForMiddleTruncation('state::get')).toEqual({
      head: 'state::get',
      tail: '',
    })
  })

  it('keeps a share of the end when no segment is a usable tail', () => {
    const name = 'averyveryverylongfunctionnamewithoutseparators'
    const { head, tail } = splitForMiddleTruncation(name)
    expect(head + tail).toBe(name)
    expect(tail).toBe(name.slice(-18))

    // the last segment is too long to be the tail → cut by share instead
    const long = 'ns::an_extremely_long_final_segment_name'
    const parts = splitForMiddleTruncation(long)
    expect(parts.head + parts.tail).toBe(long)
    expect(parts.tail.length).toBe(16)
  })

  it('always reassembles the original text', () => {
    for (const name of [
      'a::b',
      'x'.repeat(12),
      'queue::publish',
      '::leading',
      'trailing::',
      'session-manager::append_message_to_history',
    ]) {
      const { head, tail } = splitForMiddleTruncation(name)
      expect(head + tail).toBe(name)
    }
  })
})
