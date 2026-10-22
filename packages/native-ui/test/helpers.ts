import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { buildNativeUi } from '../src/build.mjs'
import { createTestHost, type TestHost, type TestHostOptions } from '../src/testing/index.mjs'

export const pkg = join(import.meta.dirname, '..')

const built = new Map<string, Promise<string>>()

/** Builds test/fixtures/<name>.tsx once per test run; resolves with the bundle source. */
export function fixture(name: string, ns = 'demo'): Promise<string> {
  const key = `${name}:${ns}`
  let p = built.get(key)
  if (!p) {
    p = (async () => {
      const dir = mkdtempSync(join(tmpdir(), 'native-ui-test-'))
      try {
        const outfile = join(dir, 'native.js')
        await buildNativeUi({ entry: join(pkg, 'test/fixtures', `${name}.tsx`), outfile, ns, smoke: false })
        return readFileSync(outfile, 'utf8')
      } finally {
        rmSync(dir, { recursive: true, force: true })
      }
    })()
    built.set(key, p)
  }
  return p
}

/** A host running fixture `name`, with its root page mounted. */
export async function mounted(name: string, options: Partial<TestHostOptions> = {}): Promise<TestHost> {
  const host = await createTestHost({ bundle: await fixture(name, options.ns), ...options })
  await host.mount('root')
  return host
}

/** The entries of every message after `from`, decoded loosely: `[code, ...]`. */
export function opsSince(host: TestHost, from: number): unknown[][] {
  return host.messages.slice(from).flatMap((m) => JSON.parse(m) as unknown[][])
}
