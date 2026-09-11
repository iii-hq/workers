import { readFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import { isAbsolute, resolve } from 'node:path'
import { parse } from 'yaml'
import { z } from 'zod'

const color = z
  .string()
  .regex(/^(#([0-9a-fA-F]{3}|[0-9a-fA-F]{6}))?$/, 'expected a hex color like #4f8cff or an empty string')

const RuntimeFields = z.object({
  output_dir: z
    .string()
    .default('~/.iii/slides')
    .describe('Directory where exported HTML, PDF and PPTX files are written'),
  default_theme: z.string().default('midnight').describe('Theme id applied to decks created without one'),
  default_model: z
    .string()
    .default('')
    .describe(
      'Router model id used by slides::outline when the call names none; empty = first chat model in the router catalog',
    ),
  default_provider: z
    .string()
    .default('')
    .describe('Router provider id paired with default_model; empty = let the router choose'),
  outline_max_output_tokens: z
    .number()
    .int()
    .positive()
    .default(24_000)
    .describe('Upper bound on tokens the drafting model may produce for one deck'),
  brand_accent: color.default('').describe('Accent color applied to every deck unless the deck overrides it'),
  brand_footer: z.string().default('').describe('Footer text shown on every slide unless the deck overrides it'),
})

export const RuntimeConfigSchema = RuntimeFields
export type RuntimeConfig = z.infer<typeof RuntimeConfigSchema>

const ConfigSchema = RuntimeFields.extend({
  engine_url: z.string().default('ws://127.0.0.1:49134'),
})

export type Config = z.infer<typeof ConfigSchema>

export function runtimeJsonSchema(): Record<string, unknown> {
  const out = z.toJSONSchema(RuntimeConfigSchema) as Record<string, unknown>
  delete out.$schema
  return out
}

export function toRuntime(cfg: Config): RuntimeConfig {
  const { engine_url: _drop, ...runtime } = cfg
  return runtime
}

export function expandHome(path: string, home = homedir()) {
  if (path === '~') return home
  if (path.startsWith('~/')) return resolve(home, path.slice(2))
  return isAbsolute(path) ? path : resolve(path)
}

export async function loadConfig(path: string): Promise<Config> {
  let raw: unknown = {}
  try {
    raw = parse(await readFile(path, 'utf8')) ?? {}
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code !== 'ENOENT') throw err
  }
  return ConfigSchema.parse(raw)
}
