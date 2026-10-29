import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import { generate, staleFiles } from '../scripts/gen-catalog.mjs'
import { pkg } from './helpers'

const catalog = JSON.parse(readFileSync(join(pkg, 'catalog.json'), 'utf8'))

describe('the generated catalog', () => {
  it('matches catalog.json (run `pnpm gen` after changing it)', () => {
    expect(staleFiles()).toEqual([])
  })

  it('generates every type, enum and code', () => {
    const files = generate(catalog)
    const ts = files['src/generated/catalog.ts']
    for (const type of catalog.types) expect(ts).toContain(`  ${type.name}: {\n    tag: ${type.tag},`)
    for (const name of Object.keys(catalog.enums)) expect(ts).toContain(`  ${name}: [`)
    for (const op of catalog.protocol.vmToHost) expect(ts).toContain(`  ${op.name}: ${op.code},`)
    for (const r of catalog.protocol.requests) expect(ts).toContain(`  ${r.name}: ${r.method},`)
    expect(ts).toContain(`maxMessageBytes: ${catalog.limits.maxMessageBytes},`)
    const props = files['src/generated/props.ts']
    for (const type of catalog.types)
      expect(props).toMatch(new RegExp(`export interface ${type.name === 'List' ? 'ListBase' : type.name}Props`))
    expect(props).toContain('onLongPress?: (x: number, y: number) => void')
    expect(props).toContain('onChange?: (checked: boolean) => void')
  })

  it('keeps tags unique and in their groups', () => {
    const tags = catalog.types.map((t: { tag: number }) => t.tag)
    expect(new Set(tags).size).toBe(tags.length)
    for (const type of catalog.types) {
      const propTags = type.props.map((p: { tag: number }) => p.tag)
      expect(new Set(propTags).size).toBe(propTags.length)
      expect(Math.min(...propTags, 16)).toBeGreaterThanOrEqual(catalog.firstTypePropTag)
    }
  })
})

describe('the package', () => {
  it('sends its package.json version in ready', () => {
    const version = JSON.parse(readFileSync(join(pkg, 'package.json'), 'utf8')).version
    expect(readFileSync(join(pkg, 'src/version.ts'), 'utf8')).toContain(`'${version}'`)
  })
})
