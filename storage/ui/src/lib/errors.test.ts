import { describe, expect, it } from 'vitest'
import { bucketUnavailable, innerError, isMissingFunction } from './errors'

describe('missing functions', () => {
  it('recognises the engine’s function_not_found', () => {
    expect(isMissingFunction({ code: 'function_not_found', message: 'Function storage::testBucket not found' })).toBe(
      true,
    )
    expect(isMissingFunction({ code: 'invocation_failed', message: 'function not found: storage::testBucket' })).toBe(
      true,
    )
    expect(isMissingFunction(new Error('Function storage::testBucket is not registered'))).toBe(true)
    // What the engine answers (iii 0.24.6) for a worker without the function.
    expect(
      isMissingFunction({
        code: 'function_not_found',
        message: 'Function storage::testBucket not found in namespace default.',
        stacktrace: null,
      }),
    ).toBe(true)
  })

  it('never mistakes a worker’s own not-found codes for it', () => {
    expect(isMissingFunction({ code: 'OBJECT_NOT_FOUND', message: 'object not found: a/b' })).toBe(false)
    expect(
      isMissingFunction({
        code: 'invocation_failed',
        message: 'handler error: {"code":"UNKNOWN_BUCKET","message":"function not found in bucket"}',
      }),
    ).toBe(false)
    expect(isMissingFunction({ code: 'TIMEOUT', message: 'timed out' })).toBe(false)
    expect(
      isMissingFunction({
        code: 'invocation_failed',
        message: 'handler error: {"code":"OBJECT_TOO_LARGE","size":59468103,"cap":10485760}',
      }),
    ).toBe(false)
    expect(isMissingFunction(undefined)).toBe(false)
  })
})

describe('worker errors', () => {
  const wire = {
    code: 'invocation_failed',
    message:
      'handler error: {"code":"BUCKET_UNAVAILABLE","bucket":"photos","stage":"secrets","message":"secret STORAGE_PHOTOS_SECRET_ACCESS_KEY not found"}',
  }

  it('peels the transport envelope', () => {
    expect(innerError(wire)).toEqual({
      code: 'BUCKET_UNAVAILABLE',
      bucket: 'photos',
      stage: 'secrets',
      message: 'secret STORAGE_PHOTOS_SECRET_ACCESS_KEY not found',
    })
    expect(innerError({ error: { code: 'X', message: 'y' } })).toEqual({ code: 'X', message: 'y' })
    // A worker error without a message still peels.
    expect(
      innerError({ code: 'invocation_failed', message: 'handler error: {"code":"OBJECT_TOO_LARGE","size":1,"cap":0}' }),
    ).toEqual({ code: 'OBJECT_TOO_LARGE', size: 1, cap: 0 })
    expect(innerError('text')).toBeNull()
    expect(innerError({ code: 'invocation_failed', message: 'handler error: {broken' })).toEqual({
      code: 'invocation_failed',
      message: 'handler error: {broken',
    })
  })

  it('reads BUCKET_UNAVAILABLE with its stage', () => {
    expect(bucketUnavailable(wire)).toEqual({
      bucket: 'photos',
      stage: 'secrets',
      message: 'secret STORAGE_PHOTOS_SECRET_ACCESS_KEY not found',
    })
    expect(bucketUnavailable({ code: 'BUCKET_UNAVAILABLE', message: '' })).toEqual({
      bucket: undefined,
      stage: undefined,
      message: 'The bucket could not be built.',
    })
    expect(bucketUnavailable({ code: 'OBJECT_NOT_FOUND', message: 'x' })).toBeNull()
    expect(bucketUnavailable(new Error('x'))).toBeNull()
  })
})
