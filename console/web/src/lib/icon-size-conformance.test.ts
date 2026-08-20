import { readdirSync, readFileSync } from 'node:fs'
import { extname, join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

const repoRoot = fileURLToPath(new URL('../../../../', import.meta.url))
const sourceRoots = [
  join(repoRoot, 'console/web/src'),
  ...readdirSync(repoRoot, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => join(repoRoot, entry.name, 'ui/src')),
]

function sourceFiles(root: string): string[] {
  try {
    return readdirSync(root, { withFileTypes: true }).flatMap((entry) => {
      const path = join(root, entry.name)
      if (entry.isDirectory()) return sourceFiles(path)
      return ['.tsx', '.jsx'].includes(extname(path)) ? [path] : []
    })
  } catch {
    return []
  }
}

describe('Console icon size contract', () => {
  it('keeps explicitly sized application icons at 16px or larger', () => {
    const offenders: string[] = []
    for (const path of sourceRoots.flatMap(sourceFiles)) {
      if (path.endsWith('icon-size-conformance.test.ts')) continue
      const source = readFileSync(path, 'utf8')
      if (
        /<[A-Z][^>]*\bsize=(?:\{(?:[0-9]|1[0-5])\}|["'](?:[0-9]|1[0-5])["'])/.test(
          source,
        ) ||
        /\bsize\s*=\s*(?:[0-9]|1[0-5])\b/.test(source) ||
        /<svg\b[^>]*\b(?:width|height)=(?:\{(?:[0-9]|1[0-5])\}|["'](?:[0-9]|1[0-5])["'])/.test(
          source,
        ) ||
        /\b(?:size-(?:2\.5|3|3\.5)|w-(?:2\.5|3|3\.5)\s+h-(?:2\.5|3|3\.5)|h-(?:2\.5|3|3\.5)\s+w-(?:2\.5|3|3\.5))\b/.test(
          source,
        )
      ) {
        offenders.push(relative(repoRoot, path))
      }
    }
    expect(offenders).toEqual([])
  })

  it('defines the shared icon and tab icon at 16px', () => {
    const css = readFileSync(
      join(repoRoot, 'console/web/src/styles/ui-recipes.css'),
      'utf8',
    )
    expect(css).toMatch(
      /\.iii-ui-icon\s*\{[^}]*width:\s*1rem;[^}]*height:\s*1rem;/s,
    )
    expect(css).toMatch(
      /\.iii-ui-tab__icon\s*\{[^}]*width:\s*1rem;[^}]*height:\s*1rem;/s,
    )
  })
})
