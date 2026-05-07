import { readFileSync } from 'node:fs'

export interface Config {
  api_base: string
  oidc_token_env: string
  fallback_token_env: string
  team_id_env: string
  project_id_env: string
  max_concurrent_sandboxes: number
  default_idle_timeout_secs: number
  default_runtime: string
  image_allowlist: string[]
}

export const DEFAULT_CONFIG: Config = {
  api_base: 'https://api.vercel.com',
  oidc_token_env: 'VERCEL_OIDC_TOKEN',
  fallback_token_env: 'VERCEL_TOKEN',
  team_id_env: 'VERCEL_TEAM_ID',
  project_id_env: 'VERCEL_PROJECT_ID',
  max_concurrent_sandboxes: 10,
  default_idle_timeout_secs: 300,
  default_runtime: 'node24',
  image_allowlist: [],
}

// Minimal YAML-ish loader that handles the flat key:value shape the rest of
// the worker family uses for config.yaml. Avoids pulling in a YAML dep for
// what is essentially a key/value file.
function parseYamlish(raw: string): Record<string, string | number | string[]> {
  const out: Record<string, string | number | string[]> = {}
  let currentList: string[] | null = null
  let currentKey = ''
  for (const line of raw.split('\n')) {
    const stripped = line.replace(/#.*$/, '').trimEnd()
    if (!stripped.trim()) continue
    if (stripped.startsWith('  - ') && currentList !== null) {
      currentList.push(
        stripped
          .slice(4)
          .trim()
          .replace(/^["']|["']$/g, ''),
      )
      continue
    }
    const m = /^([a-zA-Z_][a-zA-Z0-9_]*)\s*:\s*(.*)$/.exec(stripped)
    if (!m) continue
    const [, key, val] = m
    if (val.trim() === '') {
      currentKey = key
      currentList = []
      out[key] = currentList
    } else if (val.trim() === '[]') {
      out[key] = []
      currentList = null
    } else if (/^-?\d+$/.test(val.trim())) {
      out[key] = Number(val.trim())
      currentList = null
    } else {
      out[key] = val.trim().replace(/^["']|["']$/g, '')
      currentList = null
    }
    if (currentKey && key !== currentKey) currentKey = ''
  }
  return out
}

export function loadConfig(path: string): Config {
  try {
    const raw = readFileSync(path, 'utf8')
    const parsed = parseYamlish(raw)
    return {
      ...DEFAULT_CONFIG,
      ...(parsed as Partial<Config>),
    }
  } catch {
    return DEFAULT_CONFIG
  }
}

export function imageAllowed(cfg: Config, image: string): boolean {
  return cfg.image_allowlist.length === 0 || cfg.image_allowlist.includes(image)
}
