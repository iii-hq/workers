import type { Host } from '@iii-dev/console-ui'
import type {
  ConfigResponse,
  CycleCancelResponse,
  CycleInput,
  CycleListResponse,
  CycleResultResponse,
  CycleStartResponse,
  ExecutionOption,
  LoopConfig,
  StoredConfig,
} from './types'

const TIMEOUT_MS = 30_000
/** The most cycles `eval::list` returns (`MAX_LIST_LIMIT`). */
export const LIST_LIMIT = 200

/** The loop's public functions; nothing else is called. */
export interface EvalApi {
  config(): Promise<ConfigResponse>
  configure(config: LoopConfig): Promise<StoredConfig>
  start(input: CycleInput): Promise<CycleStartResponse>
  /** A new cycle that measures an ended one's finding again. */
  retry(cycleId: string): Promise<CycleStartResponse>
  list(): Promise<CycleListResponse>
  result(cycleId: string): Promise<CycleResultResponse | null>
  cancel(cycleId: string): Promise<CycleCancelResponse>
  executions(query: string): Promise<ExecutionOption[]>
}

export function createEvalApi(host: Host): EvalApi {
  const trigger = <T>(functionId: string, payload: Record<string, unknown>) =>
    host.iii.trigger<T>(functionId, payload, { timeoutMs: TIMEOUT_MS })
  return {
    config: () => trigger('eval::config', {}),
    configure: (config) => trigger('eval::configure', { config }),
    start: (input) => trigger('eval::start', { ...input }),
    retry: (cycleId) => trigger('eval::start', { retry: cycleId }),
    list: () => trigger('eval::list', { limit: LIST_LIMIT }),
    result: (cycleId) => trigger('eval::result', { cycle_id: cycleId }),
    cancel: (cycleId) => trigger('eval::cancel', { cycle_id: cycleId }),
    async executions(query) {
      const response = await trigger<{ executions: ExecutionOption[] }>('eval::executions', query ? { query } : {})
      return response.executions
    },
  }
}
