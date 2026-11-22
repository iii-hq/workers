import type { ExtensionIii } from '@iii-dev/console-ui'
import { resolveConfigurationId } from '@iii-dev/console-ui/configuration'
import { errorCode } from '@iii-dev/console-ui/format'
import { heldSecret, mergeConsumers } from './importer'
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
const SECRETS = { timeoutMs: 10000 }

type Json = Record<string, unknown>

const isObject = (value: unknown): value is Json => typeof value === 'object' && value !== null && !Array.isArray(value)
const serversOf = (root: unknown): Json => (isObject(root) && isObject(root.mcpServers) ? root.mcpServers : {})
/** `secrets::*` names a secret by its bare name, and an `env://` one by the env store too. */
const secretArgs = (reference: string): { name: string; store?: string } => {
  const at = reference.indexOf('://')
  const name = reference.slice(at + 3)
  return reference.slice(0, at).toLowerCase() === 'env' ? { name, store: 'env' } : { name }
}

export type Api = ReturnType<typeof createApi>

/** Every call the page makes: the worker's functions, its configuration entry and the engine's function list. */
export function createApi(iii: ExtensionIii) {
  let kept: string | null = null
  // Never a guessed id: a failed call rejects, writes nothing, and the next call asks again.
  const configId = async () => (kept ??= await resolveConfigurationId(iii, WORKER))
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

  /** Who may read the secret behind `reference` now. No value is read. An env variable with no grant reads as
   * none (it can be shared whether or not it is set); a vault name with no secret is refused. */
  const secretConsumers = async (reference: string): Promise<string[]> => {
    const args = secretArgs(reference)
    const meta = await iii.call<{ consumers?: unknown } | null>('secrets::get', args, SECRETS)
    if (meta == null) {
      if (args.store) return []
      throw new Error(`\`${args.name}\` is not in the vault.`)
    }
    // Sending a list without the ones already there would revoke them.
    if (!Array.isArray(meta.consumers)) throw new Error(`secrets::get returned no consumer list for ${args.name}`)
    return meta.consumers.map(String)
  }
  /** `secrets::access` replaces the list: send the one just read, mcp-client added. */
  const grantSecret = async (reference: string, consumers: string[]): Promise<void> => {
    await iii.call(
      'secrets::access',
      { ...secretArgs(reference), consumers: mergeConsumers(consumers, [WORKER]) },
      SECRETS,
    )
  }

  /** Who reads the vault secret `name`, or null when there is none. No value is read. */
  const secretReaders = async (name: string): Promise<string[] | null> => {
    let meta: { consumers?: unknown } | null
    try {
      meta = await iii.call<{ consumers?: unknown } | null>('secrets::get', { name }, SECRETS)
    } catch (err) {
      if (errorCode(err)?.toLowerCase() === 'function_not_found')
        throw new Error('The secrets worker is not running. Start it, then import again.')
      throw err
    }
    if (meta == null) return null
    // Sending a list without the ones already there would revoke them.
    if (meta.consumers != null && !Array.isArray(meta.consumers))
      throw new Error(`secrets::get returned no consumer list for ${name}`)
    return Array.isArray(meta.consumers) ? meta.consumers.map(String) : []
  }

  /** Let mcp-client read the secret behind `reference`: read, then grant. A failed read writes nothing. */
  const shareSecret = async (reference: string): Promise<void> =>
    grantSecret(reference, await secretConsumers(reference))

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
    secretConsumers,
    grantSecret,
    shareSecret,
    secretReaders,
    /** Store `value` under `name` in the vault for mcp-client. A name another worker reads is refused, since the
     * vault keeps no history; one only mcp-client reads is replaced, keeping its description. The value is sent
     * once and never read back. */
    storeSecret: async (name: string, value: string): Promise<void> => {
      const readers = await secretReaders(name)
      if (readers?.some((r) => r !== WORKER)) throw new Error(`${heldSecret(name, readers)} Nothing was stored.`)
      const described = readers === null ? { description: 'Stored by the MCP client page' } : {}
      await iii.call(
        'secrets::set',
        { name, value, consumers: mergeConsumers(readers ?? [], [WORKER]), ...described },
        { timeoutMs: 20000 },
      )
    },
    /** Let mcp-client read the env variable `name`. */
    shareEnv: (name: string): Promise<void> => shareSecret(`env://${name}`),
    call: (functionId: string, payload: Json) => iii.call(functionId, payload, { timeoutMs: CALL_TIMEOUT_MS }),
  }
}
