import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

const repoRoot = fileURLToPath(new URL('../../../../', import.meta.url))
const uiRoot = join(repoRoot, 'console/web/src/components/ui')

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
  'console/web/src/components/chat/ModePicker.tsx',
  'console/web/src/components/chat/ModelPicker.tsx',
  'console/web/src/components/chat/SessionAddonsPicker.tsx',
  'console/web/src/components/chat/SystemPromptPicker.tsx',
]

describe('Console typography contract', () => {
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
      join(repoRoot, 'console/web/src/styles/ui-recipes.css'),
      'utf8',
    )

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
        join(repoRoot, 'console/web/src/components/chat/DirectoryPicker.tsx'),
        'utf8',
      ),
    ).not.toMatch(/\b(?:lowercase|uppercase)\b/)
  })
})
