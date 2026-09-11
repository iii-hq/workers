import { describe, expect, it } from 'vitest'
import {
  normalizeProviderIconSvg,
  PROVIDER_ICON_SVG_MAX_BYTES,
  providerIconMaskUrl,
  providerInitial,
} from './provider-icon'

const MARK =
  '<svg viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><path d="M0 0h24v24H0z"/></svg>'

describe('normalizeProviderIconSvg', () => {
  it('keeps a plain svg document as is', () => {
    expect(normalizeProviderIconSvg(MARK)).toBe(MARK)
  })

  it('drops the xml prolog, comments and doctype before the root', () => {
    const decorated = `<?xml version="1.0" encoding="UTF-8"?>\n<!-- Source: simple-icons (CC0) -->\n<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "x.dtd">\n${MARK}\n`
    expect(normalizeProviderIconSvg(decorated)).toBe(MARK)
  })

  it('adds the svg namespace when the mark omits it', () => {
    expect(
      normalizeProviderIconSvg(
        '<svg viewBox="0 0 24 24"><path d="M0 0"/></svg>',
      ),
    ).toBe(
      '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M0 0"/></svg>',
    )
    expect(normalizeProviderIconSvg('<svg><path d="M0 0"/></svg>')).toBe(
      '<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0"/></svg>',
    )
  })

  it('refuses anything that is not a self-contained svg document', () => {
    expect(normalizeProviderIconSvg(undefined)).toBeNull()
    expect(normalizeProviderIconSvg('')).toBeNull()
    expect(normalizeProviderIconSvg('   ')).toBeNull()
    expect(normalizeProviderIconSvg('<img src="x">')).toBeNull()
    expect(normalizeProviderIconSvg('<svgx></svgx>')).toBeNull()
    expect(normalizeProviderIconSvg('<svg><path/></svg><script/>')).toBeNull()
    expect(
      normalizeProviderIconSvg(
        `<svg>${'a'.repeat(PROVIDER_ICON_SVG_MAX_BYTES)}</svg>`,
      ),
    ).toBeNull()
  })
})

describe('providerIconMaskUrl', () => {
  it('encodes the document into a quoted svg data url', () => {
    const url = providerIconMaskUrl(MARK)
    expect(url).toMatch(/^url\("data:image\/svg\+xml;charset=utf-8,/)
    expect(url).toMatch(/"\)$/)
    // Quotes and hashes inside the markup would otherwise terminate the url().
    expect(url).not.toContain('"><')
    expect(
      decodeURIComponent(
        url?.slice('url("data:image/svg+xml;charset=utf-8,'.length, -2) ?? '',
      ),
    ).toBe(MARK)
  })

  it('returns null for an unusable mark', () => {
    expect(providerIconMaskUrl(null)).toBeNull()
    expect(providerIconMaskUrl('not svg')).toBeNull()
  })
})

describe('providerInitial', () => {
  it('uses the first letter or digit, upper-cased', () => {
    expect(providerInitial('anthropic')).toBe('A')
    expect(providerInitial('  openai-codex')).toBe('O')
    expect(providerInitial('z.ai')).toBe('Z')
    expect(providerInitial('4o-mini')).toBe('4')
    expect(providerInitial('— ')).toBe('—')
    expect(providerInitial('')).toBe('?')
  })
})
