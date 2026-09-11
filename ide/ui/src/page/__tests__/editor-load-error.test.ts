import { describe, expect, it } from 'vitest'
import { isMissingFileError, loadErrorMessage, MISSING_FILE_MESSAGE } from '../load-error'

const missing =
  'handler error: {"code":"C211","message":"/w/demo-b.txt: not found or not accessible. Verify the path with coder::list-folder or coder::tree."}'

describe('loadErrorMessage', () => {
  it('turns a missing-file handler error into plain words', () => {
    expect(loadErrorMessage(missing)).toBe(MISSING_FILE_MESSAGE)
  })

  it('unwraps other handler errors to their message', () => {
    const raw = 'handler error: {"code":"C400","message":"permission denied: /etc/hosts"}'
    expect(loadErrorMessage(raw)).toBe('permission denied: /etc/hosts')
  })

  it('leaves plain text alone', () => {
    expect(loadErrorMessage('network unreachable')).toBe('network unreachable')
  })
})

describe('isMissingFileError', () => {
  it('recognises the worker code and its wording, with or without JSON', () => {
    expect(isMissingFileError(missing)).toBe(true)
    expect(isMissingFileError('handler error: {"code":"C211","message":"redacted"}')).toBe(true)
    expect(isMissingFileError('/w/x: not found or not accessible')).toBe(true)
  })

  it('leaves other failures alone', () => {
    expect(isMissingFileError('handler error: {"code":"C400","message":"permission denied"}')).toBe(false)
    expect(isMissingFileError('network unreachable')).toBe(false)
    expect(isMissingFileError('handler error: {not json')).toBe(false)
  })
})
