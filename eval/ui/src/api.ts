import type { Host } from '@iii-dev/console-ui'
import type {
  AnalysisRecord,
  AnalysisResult,
  AnalysisStatus,
  CatalogModel,
  E2eCatalogScenario,
  MonitorConfig,
  MonitorModel,
  MonitorState,
  Recurrence,
  ResolveValidationParams,
  ReproduceChange,
  ReproduceResponse,
  ReviewChange,
  ReviewsResponse,
  SuggestionCheck,
  StartValidationParams,
  SuggestionReview,
  ValidationLink,
  ValidationResolution,
} from './types'

const TIMEOUT_MS = 30_000
/** `eval::attach-validation` reads two E2E executions, 60 s each, then computes the evidence. */
const ATTACH_TIMEOUT_MS = 130_000
/** `eval::start-validation` reads the E2E stacks (10 s) and starts two executions (30 s each). */
const START_VALIDATION_TIMEOUT_MS = 90_000
/** `eval::reproduce` reads the session, assembles the context and counts it before answering. */
const REPRODUCE_TIMEOUT_MS = 120_000
/** Catalog rows per `e2e::dashboard::tests-list` page (the most the E2E returns), and pages read at most. */
const SCENARIO_PAGE_LIMIT = 100
const SCENARIO_MAX_PAGES = 10

export interface EvalApi {
  monitor(checkProviders?: boolean): Promise<MonitorState>
  /**
   * Every save states what it keeps: `codeRepository` absent turns code access off, `dailyCostCapUsd` absent removes
   * the cap.
   */
  configure(
    enabled: boolean,
    model: MonitorModel,
    codeRepository?: string,
    dailyCostCapUsd?: number,
  ): Promise<MonitorConfig>
  /** Pauses or resumes the saved configuration: its directory and cap travel along, so both stay as they were. */
  toggle(config: MonitorConfig): Promise<MonitorConfig>
  analyze(
    sessionId: string,
    reanalyze?: boolean,
  ): Promise<{ evaluation_id: string; status: AnalysisStatus; reused: boolean }>
  /** The newest 200 analyses, or those of one observed turn (`observation_key` of a record), reanalyses included. */
  list(observationKey?: string): Promise<AnalysisRecord[]>
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
  /** `eval::review`: moves the lifecycle, registers the criterion or records the verdict; answers the stored row. */
  review(evaluationId: string, suggestionIndex: number, change: ReviewChange): Promise<SuggestionReview>
  /** `eval::reviews`: the rows somebody acted on and per-analysis counts, of one analysis or of all. */
  reviews(evaluationId?: string): Promise<ReviewsResponse>
  /**
   * `eval::start-validation`: starts a baseline and a candidate E2E execution in Docker. Spends model tokens: only
   * from an explicit user action.
   */
  startValidation(
    evaluationId: string,
    suggestionIndex: number,
    params: StartValidationParams,
  ): Promise<SuggestionReview>
  /**
   * `eval::start-validation` with `dry_run`: the commits the refs resolve to, checked as a start checks them (pushed,
   * different, scenario and runs valid). Registers nothing, calls no E2E and spends nothing: safe while typing.
   */
  resolveValidation(
    evaluationId: string,
    suggestionIndex: number,
    params: ResolveValidationParams,
  ): Promise<ValidationResolution>
  /** `eval::recurrence`: the suggestion's patterns before and from the Harness version that shipped it. */
  recurrence(evaluationId: string, suggestionIndex: number): Promise<Recurrence>
  /**
   * `eval::reproduce`: replays the decision point of a suggestion. Spends model tokens, except with `dryRun`, which
   * only rebuilds the request and checks its fidelity.
   */
  reproduce(
    evaluationId: string,
    suggestionIndex: number,
    params: {
      change?: ReproduceChange
      samples?: number
      extend?: string
      check?: SuggestionCheck
      dryRun?: boolean
    },
  ): Promise<ReproduceResponse>
  /** The E2E's retained executions, raw (`e2e::dashboard::executions-list`). */
  /** The E2E's detailed executions (the newest 100), or only those named: one row is a few KB, the list over a MB. */
  e2eExecutions(ids?: string[]): Promise<unknown>
  /** The harness-e2e scenarios with their descriptions, every page of `e2e::dashboard::tests-list`. */
  e2eScenarios(): Promise<E2eCatalogScenario[]>
  models(): Promise<CatalogModel[]>
}

/** The part of a `e2e::dashboard::tests-list` page this api reads. */
interface TestsPage {
  rows: Array<{ test_id: string; spec?: { title?: string; summary?: string } }>
  next_cursor?: string | null
}

export function createEvalApi(host: Host): EvalApi {
  const trigger = <T>(functionId: string, payload: Record<string, unknown>, timeoutMs = TIMEOUT_MS) =>
    host.iii.trigger<T>(functionId, payload, { timeoutMs })

  const configure: EvalApi['configure'] = (enabled, model, codeRepository, dailyCostCapUsd) =>
    trigger('eval::configure', {
      enabled,
      model: { ...model },
      ...(codeRepository ? { code_repository: codeRepository } : {}),
      ...(dailyCostCapUsd === undefined ? {} : { daily_cost_cap_usd: dailyCostCapUsd }),
    })

  return {
    monitor(checkProviders = false) {
      return trigger('eval::config', { check_providers: checkProviders })
    },
    configure,
    toggle(config) {
      return configure(!config.enabled, config.model, config.code_repository, config.daily_cost_cap_usd)
    },
    analyze(sessionId, reanalyze = false) {
      return trigger('eval::analyze-session', { session_id: sessionId, reanalyze })
    },
    async list(observationKey) {
      const response = await trigger<{ evaluations: AnalysisRecord[] }>('eval::list', {
        limit: 200,
        ...(observationKey ? { observation_key: observationKey } : {}),
      })
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
    reproduce(evaluationId, suggestionIndex, params) {
      return trigger(
        'eval::reproduce',
        {
          evaluation_id: evaluationId,
          suggestion_index: suggestionIndex,
          ...(params.change ? { change: params.change } : {}),
          ...(params.samples === undefined ? {} : { samples: params.samples }),
          ...(params.extend ? { extend: params.extend } : {}),
          ...(params.check ? { check: params.check } : {}),
          ...(params.dryRun ? { dry_run: true } : {}),
        },
        REPRODUCE_TIMEOUT_MS,
      )
    },
    review(evaluationId, suggestionIndex, change) {
      return trigger('eval::review', {
        evaluation_id: evaluationId,
        suggestion_index: suggestionIndex,
        ...change,
      })
    },
    reviews(evaluationId) {
      return trigger('eval::reviews', evaluationId ? { evaluation_id: evaluationId } : {})
    },
    startValidation(evaluationId, suggestionIndex, params) {
      return trigger(
        'eval::start-validation',
        { evaluation_id: evaluationId, suggestion_index: suggestionIndex, ...params },
        START_VALIDATION_TIMEOUT_MS,
      )
    },
    resolveValidation(evaluationId, suggestionIndex, params) {
      return trigger('eval::start-validation', {
        evaluation_id: evaluationId,
        suggestion_index: suggestionIndex,
        ...params,
        dry_run: true,
      })
    },
    recurrence(evaluationId, suggestionIndex) {
      return trigger('eval::recurrence', { evaluation_id: evaluationId, suggestion_index: suggestionIndex })
    },
    e2eExecutions(ids) {
      // The E2E keeps 100 detailed executions: every run that can be attached.
      return trigger('e2e::dashboard::executions-list', ids ? { ids } : { limit: 100 })
    },
    async e2eScenarios() {
      const scenarios: E2eCatalogScenario[] = []
      let cursor: string | null | undefined
      let pages = 0
      do {
        const response = await trigger<TestsPage>('e2e::dashboard::tests-list', {
          limit: SCENARIO_PAGE_LIMIT,
          ...(cursor ? { cursor } : {}),
        })
        for (const row of response.rows) {
          scenarios.push({ id: row.test_id, title: row.spec?.title ?? '', summary: row.spec?.summary ?? '' })
        }
        cursor = response.next_cursor
        pages += 1
      } while (cursor && pages < SCENARIO_MAX_PAGES)
      return scenarios
    },
    async models() {
      const response = await trigger<{ models: CatalogModel[] }>('router::models::list', {})
      return response.models
    },
  }
}
