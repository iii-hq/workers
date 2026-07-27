import type { Host } from '@iii-dev/console-ui'
import type {
  CatalogModel,
  EvalRequest,
  EvalResultResponse,
  EvalStatus,
  EvalStatusResponse,
  EvalSummary,
} from './types'

const TIMEOUT_MS = 30_000

export interface EvalApi {
  list(): Promise<EvalSummary[]>
  start(request: EvalRequest): Promise<{
    evaluation_id: string
    status: EvalStatus
  }>
  status(evaluationId: string): Promise<EvalStatusResponse | null>
  result(evaluationId: string): Promise<EvalResultResponse | null>
  rerun(evaluationId: string, reverseOrder: boolean): Promise<{
    evaluation_id: string
    status: EvalStatus
  }>
  cancel(evaluationId: string): Promise<{ cancelled: boolean; status: EvalStatus }>
  delete(evaluationId: string): Promise<{ deleted: boolean }>
  models(): Promise<CatalogModel[]>
  systemPrompt(provider?: string): Promise<string | null>
}

export function createEvalApi(host: Host): EvalApi {
  const trigger = <T>(functionId: string, payload: Record<string, unknown>) =>
    host.iii.trigger<T>(functionId, payload, { timeoutMs: TIMEOUT_MS })

  return {
    async list() {
      const response = await trigger<{ evaluations: EvalSummary[] }>(
        'eval::list',
        { limit: 50 },
      )
      return response.evaluations
    },
    start(request) {
      return trigger('eval::start', request as unknown as Record<string, unknown>)
    },
    status(evaluationId) {
      return trigger('eval::status', { evaluation_id: evaluationId })
    },
    result(evaluationId) {
      return trigger('eval::result', { evaluation_id: evaluationId })
    },
    rerun(evaluationId, reverseOrder) {
      return trigger('eval::rerun', {
        evaluation_id: evaluationId,
        reverse_order: reverseOrder,
      })
    },
    cancel(evaluationId) {
      return trigger('eval::cancel', { evaluation_id: evaluationId })
    },
    delete(evaluationId) {
      return trigger('eval::delete', { evaluation_id: evaluationId })
    },
    async models() {
      const response = await trigger<{ models: CatalogModel[] }>(
        'router::models::list',
        {},
      )
      return response.models
    },
    async systemPrompt(provider) {
      const response = await trigger<{
        system_prompt?: string | null
      }>(
        'router::system_prompt::get',
        provider ? { provider } : {},
      )
      return response.system_prompt?.trim() || null
    },
  }
}

export function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message
  if (typeof error === 'string') return error
  try {
    return JSON.stringify(error)
  } catch {
    return 'unknown error'
  }
}

export function isTerminal(status: EvalStatus): boolean {
  return (
    status === 'completed' ||
    status === 'failed' ||
    status === 'cancelled'
  )
}
