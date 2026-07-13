import { describe, expect, it } from 'vitest'
import {
  contentEtag,
  isConsoleExtensionCapability,
  verifyExtensionAsset,
} from './contract'

function base64(value: string): string {
  return btoa(value)
}

describe('console extension asset verification', () => {
  it('uses the shared FNV-1a contract', () => {
    expect(contentEtag(new TextEncoder().encode('hello'))).toBe(
      'fnv1a64-a430d84680aabd0b',
    )
  })

  it('accepts matching descriptor and bytes', () => {
    const bytes = new TextEncoder().encode('export const ok = true')
    const etag = contentEtag(bytes)
    expect(
      new TextDecoder().decode(
        verifyExtensionAsset(
          { path: 'extension.js', media_type: 'text/javascript', etag },
          {
            path: 'extension.js',
            media_type: 'text/javascript',
            encoding: 'base64',
            content: base64('export const ok = true'),
            etag,
          },
        ),
      ),
    ).toBe('export const ok = true')
  })

  it('rejects corrupted content', () => {
    const etag = contentEtag(new TextEncoder().encode('expected'))
    expect(() =>
      verifyExtensionAsset(
        { path: 'extension.js', media_type: 'text/javascript', etag },
        {
          path: 'extension.js',
          media_type: 'text/javascript',
          encoding: 'base64',
          content: base64('corrupted'),
          etag,
        },
      ),
    ).toThrow('content verification failed')
  })
})

describe('console extension capability discovery', () => {
  const functionId = 'approval::console-extension'

  it('accepts the versioned internal capability metadata', () => {
    expect(
      isConsoleExtensionCapability(
        {
          function_id: functionId,
          metadata: {
            internal: true,
            capability: 'iii.console-extension',
            api_version: 1,
          },
        },
        functionId,
      ),
    ).toBe(true)
  })

  it('rejects a matching suffix without the capability metadata', () => {
    expect(
      isConsoleExtensionCapability(
        { function_id: functionId, metadata: { internal: true } },
        functionId,
      ),
    ).toBe(false)
  })
})
