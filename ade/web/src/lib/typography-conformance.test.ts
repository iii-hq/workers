import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

const repoRoot = fileURLToPath(new URL('../../../../', import.meta.url))
const srcRoot = join(repoRoot, 'ade/web/src')
const uiRoot = join(srcRoot, 'components/ui')

/** Every console `.tsx` outside the marketing demo, stories and tests. */
function consoleSources(): string[] {
  return readdirSync(srcRoot, { recursive: true, encoding: 'utf8' })
    .filter(
      (file) =>
        file.endsWith('.tsx') &&
        !file.startsWith('demo/') &&
        !file.endsWith('.stories.tsx') &&
        !file.endsWith('.test.tsx'),
    )
    .sort()
}

const humanFacingPrimitives = [
  'Badge.tsx',
  'Button.tsx',
  'Cell.tsx',
  'Dialog.tsx',
  'DropdownMenu.tsx',
  'EmptyState.tsx',
  'Input.tsx',
  'ModeToggle.tsx',
  'PageChrome.tsx',
  'Pagination.tsx',
  'Select.tsx',
  'Selector.tsx',
  'StatusPanel.tsx',
  'Tabs.tsx',
  'Tooltip.tsx',
]

const globalSelectorChrome = [
  'ade/web/src/components/chat/ModelPicker.tsx',
  'ade/web/src/components/chat/SessionAddonsPicker.tsx',
  'ade/web/src/components/chat/SystemPromptPicker.tsx',
]

describe('Console typography contract', () => {
  it('keeps case transforms inside the Eyebrow recipe only', () => {
    // `uppercase` is the eyebrow (`Eyebrow` / `.iii-ui-eyebrow`); authored
    // copy and machine identifiers keep their casing everywhere else, and
    // `lowercase` has no sanctioned home at all.
    const offenders = consoleSources().filter((file) => {
      const source = readFileSync(join(srcRoot, file), 'utf8')
      if (/\blowercase\b/.test(source)) return true
      return (
        file !== 'components/ui/Eyebrow.tsx' && /\buppercase\b/.test(source)
      )
    })

    expect(offenders).toEqual([])
  })

  it('keeps UI text at or above 11px outside data visualisations', () => {
    // `micro` (9–10px) exists for diagram labels inside charts only; every
    // kept site carries a `diagram micro` comment beside it.
    const offenders = consoleSources().filter((file) => {
      const source = readFileSync(join(srcRoot, file), 'utf8')
      const tiny = source.match(/text-\[(?:9|10|10\.5)px\]/g)?.length ?? 0
      const marked = source.match(/diagram micro/g)?.length ?? 0
      return tiny > marked
    })

    expect(offenders).toEqual([])
  })

  it('keeps human-facing shared chrome sans and free of case transforms', () => {
    const offenders = humanFacingPrimitives.filter((file) =>
      /\b(?:font-mono|lowercase|uppercase)\b/.test(
        readFileSync(join(uiRoot, file), 'utf8'),
      ),
    )

    expect(offenders).toEqual([])
  })

  it('keeps shared list, card, chip, and tab recipes naturally cased', () => {
    const css = readFileSync(
      join(repoRoot, 'ade/web/src/styles/ui-recipes.css'),
      'utf8',
    )
      // The eyebrow is the one mono-caps recipe by design (`Eyebrow`).
      .replace(/\.iii-ui-eyebrow\s*\{[^}]*\}/, '')

    expect(css).not.toMatch(/font-family:\s*var\(--font-mono\)/)
    expect(css).not.toMatch(/text-transform:\s*(?:lowercase|uppercase)/)
    expect(css).toMatch(/\.iii-ui-tab\s*\{[^}]*font-weight:\s*600;/s)
  })

  it('keeps global selector chrome sans and naturally cased', () => {
    const offenders = globalSelectorChrome.filter((path) =>
      /\b(?:font-mono|lowercase|uppercase)\b/.test(
        readFileSync(join(repoRoot, path), 'utf8'),
      ),
    )

    expect(offenders).toEqual([])
    expect(
      readFileSync(
        join(repoRoot, 'ade/web/src/components/chat/DirectoryPicker.tsx'),
        'utf8',
      ),
    ).not.toMatch(/\b(?:lowercase|uppercase)\b/)
  })
})
