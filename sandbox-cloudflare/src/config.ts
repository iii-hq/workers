import { readFileSync } from 'node:fs'

export interface Config {
  bridge_url_env: string
  bridge_token_env: string
  max_concurrent_sandboxes: number
  default_idle_timeout_secs: number
  image_allowlist: string[]
}

export const DEFAULT_CONFIG: Config = {
  bridge_url_env: 'CLOUDFLARE_BRIDGE_URL',
  bridge_token_env: 'CLOUDFLARE_BRIDGE_TOKEN',
  max_concurrent_sandboxes: 10,
  default_idle_timeout_secs: 300,
  image_allowlist: [],
}

function parseYamlish(raw: string): Record<string, string | number | string[]> {
  const out: Record<string, string | number | string[]> = {}
  let currentList: string[] | null = null
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
  }
  return out
}

export function loadConfig(path: string): Config {
  try {
    const raw = readFileSync(path, 'utf8')
    return { ...DEFAULT_CONFIG, ...(parseYamlish(raw) as Partial<Config>) }
  } catch {
    return DEFAULT_CONFIG
  }
}

export function imageAllowed(cfg: Config, image: string): boolean {
  return cfg.image_allowlist.length === 0 || cfg.image_allowlist.includes(image)
}
