/** Builds the app once per run (with the SDK's size check and smoke run) and hands the bundle to every test file. */
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { buildNativeUi } from '@iii-dev/native-ui/build'
import type { TestProject } from 'vitest/node'

declare module 'vitest' {
  export interface ProvidedContext {
    bundle: string
    bundleBytes: number
  }
}

export default async function setup(project: TestProject) {
  const dir = mkdtempSync(join(tmpdir(), 'browser-native-ui-'))
  try {
    const outfile = join(dir, 'native.js')
    const result = await buildNativeUi({
      entry: join(import.meta.dirname, '../src/app.tsx'),
      outfile,
      ns: 'browser',
      log: { info() {}, warn() {}, error() {} } as unknown as Console,
    })
    project.provide('bundle', readFileSync(outfile, 'utf8'))
    project.provide('bundleBytes', result.bytes)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
}
