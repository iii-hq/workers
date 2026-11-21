import type { ExtensionIii } from '@iii-dev/console-ui'
import { resolveConfigurationId } from '@iii-dev/console-ui/configuration'
import { errorCode } from '@iii-dev/console-ui/format'
import type {
  EngineFunction,
  FunctionInfo,
  LoginStart,
  ProcessLogs,
  ServerState,
  ServersList,
  ValidateResponse,
} from './types'

const WORKER = 'mcp-client'
/** Past the worker's own 120 s call timeout, so its error arrives first. */
const CALL_TIMEOUT_MS = 130000
const SECRETS_TIMEOUT_MS = 10000

type Json = Record<string, unknown>

const isObject = (value: unknown): value is Json => typeof value === 'object' && value !== null && !Array.isArray(value)
const serversOf = (root: unknown): Json => (isObject(root) && isObject(root.mcpServers) ? root.mcpServers : {})

export type Api = ReturnType<typeof createApi>

/** Every call the page makes: the worker's functions, its configuration entry and the engine's function list. */
export function createApi(iii: ExtensionIii) {
  let kept: string | null = null
  // The fallback is not kept, so a worker that starts later is asked again.
  const configId = async () => {
    if (kept) return kept
    try {
      kept = await resolveConfigurationId(iii, WORKER)
      return kept
    } catch {
      return WORKER
    }
  }
  const readRoot = async (id: string): Promise<unknown> => {
    try {
      return (await iii.call<{ value?: unknown }>('configuration::get', { id, raw: true }))?.value
    } catch (err) {
      if (errorCode(err) === 'NOT_FOUND') return undefined
      throw err
    }
  }
  // Re-read right before the write, so an edit made elsewhere since the page read it survives.
  const writeServers = async (change: (servers: Json) => Json) => {
    const id = await configId()
    const root = await readRoot(id)
    const value = { ...(isObject(root) ? root : {}), mcpServers: change(serversOf(root)) }
    await iii.call('configuration::set', { id, value })
  }

  return {
    /** The client itself, for a view that observes a source of its own. */
    iii,
    listServers: () => iii.call<ServersList>(`${WORKER}::servers::list`, {}),
    reconnect: (server: string) => iii.call<{ state: ServerState }>(`${WORKER}::servers::reconnect`, { server }),
    approve: (server: string, fingerprint?: string) =>
      iii.call<{ approved: boolean }>(`${WORKER}::servers::approve`, { server, fingerprint }),
    validate: (mcpServers: Json) => iii.call<ValidateResponse>(`${WORKER}::config::validate`, { mcpServers }),
    processLogs: (server: string) => iii.call<ProcessLogs>(`${WORKER}::process::logs`, { server }),
    loginStart: (server: string) => iii.call<LoginStart>(`${WORKER}::login::start`, { server }),
    loginComplete: (callbackUrl: string) =>
      iii.call<{ server: string; state: ServerState }>(`${WORKER}::login::complete`, { callback_url: callbackUrl }),
    loginCancel: (server: string) => iii.call<Json>(`${WORKER}::login::cancel`, { server }),
    signOut: (server: string) => iii.call<{ state: ServerState }>(`${WORKER}::login::sign-out`, { server }),
    listFunctions: async (server?: string): Promise<EngineFunction[]> => {
      const { functions } = await iii.call<{ functions: EngineFunction[] }>('engine::functions::list', {
        include_internal: false,
      })
      return functions.filter(
        (fn) =>
          fn.function_id.startsWith('mcp::') && (server === undefined || fn.function_id.split('::')[1] === server),
      )
    },
    functionInfo: (functionId: string) =>
      iii.call<FunctionInfo>('engine::functions::info', { function_id: functionId }),
    readConfig: async (): Promise<{ id: string; mcpServers: Json }> => {
      const id = await configId()
      return { id, mcpServers: serversOf(await readRoot(id)) }
    },
    writeEntry: (name: string, entry: unknown) =>
      writeServers((servers) => {
        const next = { ...servers }
        if (entry === null) delete next[name]
        else next[name] = entry
        return next
      }),
    writeConfig: (mcpServers: Json) => writeServers(() => mcpServers),
    /** Let mcp-client read the secret behind `reference`. `secrets::access` replaces the consumer list, so the
     * list sent is the one just read plus mcp-client; a failed read writes nothing. No value is read. */
    shareSecret: async (reference: string): Promise<void> => {
      const at = reference.indexOf('://')
      const name = reference.slice(at + 3)
      const store = reference.slice(0, at).toLowerCase() === 'env' ? { store: 'env' } : {}
      const options = { timeoutMs: SECRETS_TIMEOUT_MS }
      const { consumers } = await iii.call<{ consumers?: unknown }>('secrets::get', { name, ...store }, options)
      // Sending a list without the ones already there would revoke them.
      if (!Array.isArray(consumers)) throw new Error(`secrets::get returned no consumer list for ${name}`)
      const next = [...new Set([...consumers.map(String), WORKER])]
      await iii.call('secrets::access', { name, consumers: next, ...store }, options)
    },
    call: (functionId: string, payload: Json) => iii.call(functionId, payload, { timeoutMs: CALL_TIMEOUT_MS }),
  }
}
