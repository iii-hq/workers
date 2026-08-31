import { readdirSync, readFileSync } from 'node:fs'
import { extname, join } from 'node:path'
import { describe, expect, it } from 'vitest'

function tsxFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name)
    if (entry.isDirectory()) return tsxFiles(path)
    return extname(entry.name) === '.tsx' ? [path] : []
  })
}

describe('shared table adoption', () => {
  it('keeps native chat renderers on the shared semantic table parts', () => {
    const chatDirectory = new URL('../components/chat', import.meta.url)
      .pathname
    const rawTableFiles = tsxFiles(chatDirectory).filter((file) =>
      readFileSync(file, 'utf8').includes('<table'),
    )

    expect(rawTableFiles).toEqual([])
  })

  it('keeps the function and trigger schema table on the public primitive', () => {
    const source = readFileSync(
      new URL('../../../ui/src/catalog/SchemaTable.tsx', import.meta.url),
      'utf8',
    )

    expect(source).toContain("from '@iii-dev/console-ui'")
    expect(source).toContain('<TableViewport')
    expect(source).toContain('<TableHeader>')
    expect(source).not.toContain('<table')
  })
})
