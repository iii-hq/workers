import { execFile } from 'node:child_process'
import { readFile } from 'node:fs/promises'
import { promisify } from 'node:util'
import { parse } from 'yaml'

const run = promisify(execFile)

export type ContainerSource = 'path' | 'package' | 'unknown'

export interface DeclaredContainer {
  name: string
  source: ContainerSource
  ref: string
  version: string | null
  start_after: string[]
  environment: string[]
  run: string | null
}

export interface ProjectDeclaration {
  file: string
  namespace: string | null
  engine_url: string | null
  engine_host: string | null
  engine_port: number | null
  startup_timeout: string | null
  stop_timeout: string | null
  containers: DeclaredContainer[]
}

export interface ListeningPort {
  port: number
  address: string
}

export interface ContainerPorts {
  pid: number
  ports: ListeningPort[]
}

function text(value: unknown): string | null {
  if (typeof value === 'string' && value.length > 0) return value
  if (typeof value === 'number') return String(value)
  return null
}

function stringList(value: unknown): string[] {
  if (!Array.isArray(value)) return []
  return value.filter((item): item is string => typeof item === 'string')
}

function endpoint(url: string | null): { host: string | null; port: number | null } {
  if (!url) return { host: null, port: null }
  try {
    const parsed = new URL(url)
    const port = parsed.port ? Number(parsed.port) : null
    return { host: parsed.hostname || null, port: Number.isFinite(port) ? port : null }
  } catch {
    return { host: null, port: null }
  }
}

export function parseWorkerRef(ref: string): { source: ContainerSource; ref: string } {
  if (ref.startsWith('path://')) return { source: 'path', ref: ref.slice('path://'.length) }
  if (ref.startsWith('package://')) return { source: 'package', ref: ref.slice('package://'.length) }
  return { source: 'unknown', ref }
}

export function parseProject(file: string, source: string): ProjectDeclaration {
  const doc = parse(source) as Record<string, unknown> | null
  const root = doc && typeof doc === 'object' ? doc : {}
  const engine = root.engine && typeof root.engine === 'object' ? (root.engine as Record<string, unknown>) : {}
  const engine_url = text(engine.url)
  const { host, port } = endpoint(engine_url)
  const declared =
    root.containers && typeof root.containers === 'object' ? (root.containers as Record<string, unknown>) : {}
  const containers: DeclaredContainer[] = Object.entries(declared).map(([name, raw]) => {
    const entry = raw && typeof raw === 'object' ? (raw as Record<string, unknown>) : {}
    const worker = parseWorkerRef(text(entry.worker) ?? '')
    const env =
      entry.environment && typeof entry.environment === 'object' ? Object.keys(entry.environment as object) : []
    const scripts = entry.scripts && typeof entry.scripts === 'object' ? (entry.scripts as Record<string, unknown>) : {}
    return {
      name,
      source: worker.source,
      ref: worker.ref,
      version: text(entry.version),
      start_after: stringList(entry.start_after),
      environment: env,
      run: text(scripts.run),
    }
  })
  return {
    file,
    namespace: text(root.namespace),
    engine_url,
    engine_host: host,
    engine_port: port,
    startup_timeout: text(root.startup_timeout),
    stop_timeout: text(root.stop_timeout),
    containers,
  }
}

export async function readProject(file: string): Promise<ProjectDeclaration> {
  return parseProject(file, await readFile(file, 'utf8'))
}

export function parseLsof(output: string): Map<number, ListeningPort[]> {
  const byPid = new Map<number, ListeningPort[]>()
  let pid: number | null = null
  for (const line of output.split('\n')) {
    if (line.startsWith('p')) {
      pid = Number(line.slice(1))
      if (!byPid.has(pid)) byPid.set(pid, [])
      continue
    }
    if (!line.startsWith('n') || pid === null) continue
    const name = line.slice(1)
    const at = name.lastIndexOf(':')
    if (at < 0) continue
    const port = Number(name.slice(at + 1))
    if (!Number.isFinite(port)) continue
    const address = name.slice(0, at)
    const list = byPid.get(pid) ?? []
    if (!list.some((entry) => entry.port === port && entry.address === address)) {
      list.push({ port, address })
    }
    byPid.set(pid, list)
  }
  for (const list of byPid.values()) list.sort((a, b) => a.port - b.port)
  return byPid
}

export async function listeningPorts(pids: number[]): Promise<Map<number, ListeningPort[]>> {
  const unique = [...new Set(pids.filter((pid) => Number.isInteger(pid) && pid > 0))]
  if (unique.length === 0) return new Map()
  try {
    const { stdout } = await run('lsof', ['-nP', '-a', '-p', unique.join(','), '-iTCP', '-sTCP:LISTEN', '-Fpn'], {
      timeout: 5_000,
      maxBuffer: 1024 * 1024,
    })
    return parseLsof(stdout)
  } catch (error) {
    const stdout = (error as { stdout?: string }).stdout
    if (typeof stdout === 'string' && stdout.length > 0) return parseLsof(stdout)
    return new Map()
  }
}
