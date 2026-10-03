import type { Host } from '@iii-dev/console-ui'
import type {
  AnalysisRecord,
  AnalysisResult,
  AnalysisStatus,
  CatalogModel,
  MonitorConfig,
  MonitorModel,
  MonitorState,
  ProposeValidationResponse,
  ValidationLink,
} from './types'

const TIMEOUT_MS = 30_000
/** `eval::attach-validation` reads two E2E executions, 10 s each. */
const ATTACH_TIMEOUT_MS = 45_000
/** `eval::propose-validation` lists the E2E runs (10 s) and asks Jev (70 s). */
const PROPOSE_TIMEOUT_MS = 90_000

export interface EvalApi {
  monitor(checkProviders?: boolean): Promise<MonitorState>
  /** `codeRepository` absent turns code access off: every save states the directory it keeps. */
  configure(enabled: boolean, model: MonitorModel, codeRepository?: string): Promise<MonitorConfig>
  /** Pauses or resumes the saved configuration: its directory travels along, so code access stays as it was. */
  toggle(config: MonitorConfig): Promise<MonitorConfig>
  analyze(
    sessionId: string,
    reanalyze?: boolean,
  ): Promise<{ evaluation_id: string; status: AnalysisStatus; reused: boolean }>
  list(): Promise<AnalysisRecord[]>
  result(evaluationId: string): Promise<AnalysisResult | null>
  cancel(evaluationId: string): Promise<{ cancelled: boolean; status: AnalysisStatus }>
  delete(evaluationId: string): Promise<{ deleted: boolean }>
  attachValidation(request: {
    evaluationId: string
    suggestionIndex: number
    baselineExecutionId: string
    candidateExecutionId: string
    dryRun?: boolean
  }): Promise<{ link: ValidationLink; saved: boolean }>
  proposeValidation(evaluationId: string, suggestionIndex: number): Promise<ProposeValidationResponse>
  /** The E2E's retained executions, raw (`e2e::dashboard::executions-list`). */
  e2eExecutions(): Promise<unknown>
  models(): Promise<CatalogModel[]>
}

export function createEvalApi(host: Host): EvalApi {
  const trigger = <T>(functionId: string, payload: Record<string, unknown>, timeoutMs = TIMEOUT_MS) =>
    host.iii.trigger<T>(functionId, payload, { timeoutMs })

  const configure: EvalApi['configure'] = (enabled, model, codeRepository) =>
    trigger('eval::configure', {
      enabled,
      model: { ...model },
      ...(codeRepository ? { code_repository: codeRepository } : {}),
    })

  return {
    monitor(checkProviders = false) {
      return trigger('eval::config', { check_providers: checkProviders })
    },
    configure,
    toggle(config) {
      return configure(!config.enabled, config.model, config.code_repository)
    },
    analyze(sessionId, reanalyze = false) {
      return trigger('eval::analyze-session', { session_id: sessionId, reanalyze })
    },
    async list() {
      const response = await trigger<{ evaluations: AnalysisRecord[] }>('eval::list', { limit: 200 })
      return response.evaluations
    },
    result(evaluationId) {
      return trigger('eval::result', { evaluation_id: evaluationId })
    },
    cancel(evaluationId) {
      return trigger('eval::cancel', { evaluation_id: evaluationId })
    },
    delete(evaluationId) {
      return trigger('eval::delete', { evaluation_id: evaluationId })
    },
    attachValidation(request) {
      return trigger(
        'eval::attach-validation',
        {
          evaluation_id: request.evaluationId,
          suggestion_index: request.suggestionIndex,
          baseline_execution_id: request.baselineExecutionId,
          candidate_execution_id: request.candidateExecutionId,
          dry_run: request.dryRun ?? false,
        },
        ATTACH_TIMEOUT_MS,
      )
    },
    proposeValidation(evaluationId, suggestionIndex) {
      return trigger(
        'eval::propose-validation',
        { evaluation_id: evaluationId, suggestion_index: suggestionIndex },
        PROPOSE_TIMEOUT_MS,
      )
    },
    e2eExecutions() {
      // The E2E keeps 100 detailed executions: every run that can be attached.
      return trigger('e2e::dashboard::executions-list', { limit: 100 })
    },
    async models() {
      const response = await trigger<{ models: CatalogModel[] }>('router::models::list', {})
      return response.models
    },
  }
}
