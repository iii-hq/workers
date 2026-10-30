/**
 * A brand tint for a provider's mark, where the brand has one. The marks
 * arrive monochrome (`icon_svg` is painted as a `currentColor` mask, see
 * `lib/provider-icon`), so colour is the console's to add: one 16px glyph
 * beside a label, the way the glyph tones work — never a fill or text.
 * Brands whose mark is black or white (OpenAI, xAI, GitHub) keep the ink.
 */
const BRAND_COLORS: Record<string, string> = {
  anthropic: '#d97757',
  'claude-code': '#d97757',
  'openai-codex': 'var(--color-glyph-purple)',
  'github-copilot': 'var(--color-glyph-blue)',
  deepseek: '#4d6bfe',
  google: 'var(--color-glyph-blue)',
  kimi: 'var(--color-glyph-teal)',
  openrouter: 'var(--color-glyph-purple)',
  zai: 'var(--color-glyph-rose)',
}

export function providerBrandColor(
  providerId: string | null | undefined,
): string | undefined {
  if (!providerId) return undefined
  return BRAND_COLORS[providerId]
}
