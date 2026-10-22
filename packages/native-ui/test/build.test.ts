import { createHash } from 'node:crypto'
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, describe, expect, it } from 'vitest'
import { buildNativeUi } from '../src/build.mjs'
import { pkg } from './helpers'

const dir = mkdtempSync(join(tmpdir(), 'native-ui-build-'))
afterAll(() => rmSync(dir, { recursive: true, force: true }))

const write = (name: string, source: string) => {
  const file = join(dir, name)
  writeFileSync(file, source)
  return file
}

describe('buildNativeUi', () => {
  it('writes an IIFE bundle with its hash, after the smoke run', async () => {
    const outfile = join(dir, 'counter.js')
    const r = await buildNativeUi({ entry: join(pkg, 'conformance/apps/counter.tsx'), outfile, ns: 'demo' })
    const source = readFileSync(outfile, 'utf8')
    expect(r.outfile).toBe(outfile)
    expect(r.bytes).toBe(Buffer.byteLength(source))
    expect(r.sha256).toBe(createHash('sha256').update(source).digest('hex'))
    expect(r.ready?.pages).toEqual([{ name: 'root', modal: false, fullScreen: false }])
    expect(r.warnings).toEqual([])
    expect(source).toMatch(/^("use strict";)?\(\(\)=>\{/)
    expect(source).not.toMatch(/\bimport\b|\brequire\(/)
    expect(r.bytes).toBeLessThan(64 * 1024)
  })

  it('refuses a bad namespace', async () => {
    await expect(buildNativeUi({ entry: 'x.tsx', outfile: join(dir, 'x.js'), ns: 'Bad NS' })).rejects.toThrow(
      /ns must match/,
    )
  })

  it('does not write a bundle that fails the smoke run', async () => {
    const entry = write(
      'no-root.tsx',
      "import { defineApp, Text } from '@iii-dev/native-ui'\ndefineApp({ pages: { main: () => <Text>hi</Text> } })\n",
    )
    const outfile = join(dir, 'no-root.js')
    await expect(buildNativeUi({ entry, outfile, ns: 'demo' })).rejects.toThrow(/root/)
    expect(existsSync(outfile)).toBe(false)
  })

  it('fails above limits.bundleFailBytes and warns above limits.bundleWarnBytes', async () => {
    const big = (kib: number) =>
      `import { defineApp, Text } from '@iii-dev/native-ui'\nconst blob = ${JSON.stringify('x'.repeat(kib * 1024))}\ndefineApp({ pages: { root: () => <Text>{blob.slice(0, 3)}</Text> } })\n`
    const warn = await buildNativeUi({
      entry: write('warn.tsx', big(160)),
      outfile: join(dir, 'warn.js'),
      ns: 'demo',
      log: { info() {}, warn() {}, error() {} },
    })
    expect(warn.warnings[0]).toMatch(/above 128\.0 KiB/)
    await expect(
      buildNativeUi({ entry: write('fail.tsx', big(300)), outfile: join(dir, 'fail.js'), ns: 'demo' }),
    ).rejects.toThrow(/above the 256\.0 KiB limit/)
    expect(existsSync(join(dir, 'fail.js'))).toBe(false)
  })

  it('rebuilds in watch mode and leaves the last good bundle on a broken change', async () => {
    const entry = write(
      'watched.tsx',
      "import { defineApp, Text } from '@iii-dev/native-ui'\ndefineApp({ pages: { root: () => <Text>one</Text> } })\n",
    )
    const outfile = join(dir, 'watched.js')
    const lines: string[] = []
    const log = {
      info: (m: string) => lines.push(m),
      warn: (m: string) => lines.push(m),
      error: (m: string) => lines.push(m),
    }
    const watcher = await buildNativeUi({ entry, outfile, ns: 'demo', watch: true, log })
    const until = async (check: () => boolean) => {
      for (let i = 0; i < 100 && !check(); i++) await new Promise((r) => setTimeout(r, 50))
      return check()
    }
    try {
      expect(await until(() => existsSync(outfile) && readFileSync(outfile, 'utf8').includes('one'))).toBe(true)
      writeFileSync(
        entry,
        "import { defineApp, Text } from '@iii-dev/native-ui'\ndefineApp({ pages: { root: () => <Text>two</Text> } })\n",
      )
      expect(await until(() => readFileSync(outfile, 'utf8').includes('two'))).toBe(true)
      writeFileSync(entry, "import { defineApp } from '@iii-dev/native-ui'\ndefineApp({ pages: {} })\n")
      expect(await until(() => lines.some((l) => l.includes('build failed')))).toBe(true)
      expect(readFileSync(outfile, 'utf8')).toContain('two')
    } finally {
      await watcher.stop()
    }
  })
})
