export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | { [key: string]: JsonValue }

export type EvalStatus =
  | 'queued'
  | 'running'
  | 'completed'
  | 'failed'
  | 'cancelled'

export type EvalRunStatus =
  | 'pending'
  | 'running'
  | 'completed'
  | 'failed'
  | 'cancelled'

export type ComparisonDimension = 'prompt' | 'system_prompt'
export type VariantRole = 'control' | 'treatment'
export type ExecutionOrder = 'balanced_control_first' | 'balanced_treatment_first'

export interface EvalVariant {
  label?: string
  prompt: string
  system_prompt: string | null
}

export interface EvalModelConfig {
  model: string
  provider?: string
  system_prompt_strategy: 'override' | 'enrich' | 'disabled'
  mode?: 'ask' | 'agent'
  thinking_level?: 'minimal' | 'low' | 'medium' | 'high' | 'xhigh'
  provider_options?: Record<string, JsonValue>
}

export interface EvalLimits {
  execution: {
    invocation_timeout_seconds?: number
    scenario_timeout_seconds?: number
    max_turns?: number
    max_output_tokens_per_call?: number
    max_total_tokens?: number
    max_cost_usd?: number
  }
  evaluation: {
    max_function_call_errors?: number
    max_error_spans?: number
  }
}

export interface FunctionPolicy {
  allow: string[]
  deny: string[]
  expose: 'agent_trigger' | 'native'
}

export type OutputContract =
  | { type: 'text' }
  | { type: 'json'; schema?: JsonValue }

export interface EvalRequest {
  dimension: ComparisonDimension
  model: EvalModelConfig
  control: EvalVariant
  treatment: EvalVariant
  evaluator?: {
    function_id: string
    arguments: JsonValue
  }
  runs: number
  execution_order: ExecutionOrder
  source_evaluation_id?: string
  limits: EvalLimits
  functions?: FunctionPolicy
  output?: OutputContract
  metadata?: JsonValue
}

export interface EvalSummary {
  evaluation_id: string
  status: EvalStatus
  dimension: ComparisonDimension
  model: string
  provider?: string
  control_label?: string
  treatment_label?: string
  total_runs: number
  terminal_runs: number
  passed_runs: number
  failed_runs: number
  eligible?: boolean
  created_at: number
  updated_at: number
  completed_at?: number
  error?: string
}

export interface EvalStatusResponse {
  evaluation_id: string
  status: EvalStatus
  total_runs: number
  terminal_runs: number
  passed_runs: number
  failed_runs: number
  active?: {
    run_id: string
    role: VariantRole
    iteration: number
    session_id: string
    turn_id?: string
    started_at: number
  }
  created_at: number
  updated_at: number
  completed_at?: number
  error?: string
}

export interface Benchmark {
  wall_time_ms: number
  sessions: number
  turns: number
  function_calls: number
  function_call_errors: number
  input_tokens?: number
  output_tokens?: number
  total_tokens?: number
  cache_read_tokens?: number
  cache_write_tokens?: number
  reasoning_tokens?: number
  cost_usd?: number
  trace_count?: number
  span_count?: number
  error_span_count?: number
  trace_duration_ms?: number
}

export interface VariantAggregate {
  runs: number
  evaluated_runs: number
  passed: number
  pass_rate: number
  benchmarked_runs: number
  median_score?: number
  median_wall_time_ms?: number
  median_input_tokens?: number
  median_output_tokens?: number
  median_total_tokens?: number
  median_reasoning_tokens?: number
  median_function_calls?: number
  median_function_call_errors?: number
  median_cost_usd?: number
  median_trace_count?: number
  median_span_count?: number
  median_error_span_count?: number
  median_trace_duration_ms?: number
}

export interface AggregateDelta {
  pass_rate: number
  median_score?: number
  median_wall_time_ms?: number
  median_total_tokens?: number
  median_reasoning_tokens?: number
  median_function_calls?: number
  median_function_call_errors?: number
  median_cost_usd?: number
  median_trace_count?: number
  median_span_count?: number
  median_error_span_count?: number
  median_trace_duration_ms?: number
}

export interface EvalRun {
  run_id: string
  role: VariantRole
  iteration: number
  execution_position: number
  pair_position: number
  status: EvalRunStatus
  session_id: string
  turn_id?: string
  passed?: boolean
  started_at: number
  completed_at?: number
  output?: JsonValue
  metrics?: JsonValue
  evaluation?: {
    passed: boolean
    score?: number
    reason?: string
    details?: JsonValue
  }
  benchmark?: Benchmark
  failures?: Array<{
    phase: string
    message: string
    function_id?: string
  }>
}

export interface EvalReport {
  schema_version: string
  evaluation_id: string
  source_evaluation_id?: string
  execution_order_policy: ExecutionOrder
  effective_execution_order: string[]
  dimension: ComparisonDimension
  model: EvalModelConfig
  evaluator?: {
    function_id: string
    arguments_sha256: string
  }
  control: {
    label?: string
    prompt_sha256: string
    system_prompt_sha256?: string
  }
  treatment: {
    label?: string
    prompt_sha256: string
    system_prompt_sha256?: string
  }
  shared_artifacts: {
    model_sha256: string
    function_policy_sha256: string
    limits_sha256: string
    output_sha256: string
  }
  control_aggregate: VariantAggregate
  treatment_aggregate: VariantAggregate
  delta: AggregateDelta
  order_sensitivity: OrderSensitivity
  eligible?: boolean
  runs: EvalRun[]
  created_at: number
  completed_at: number
}

export interface RoleOrderSensitivity {
  first_runs: number
  first_passed: number
  second_runs: number
  second_passed: number
  differs: boolean
}

export interface OrderSensitivity {
  detected: boolean
  control: RoleOrderSensitivity
  treatment: RoleOrderSensitivity
}

export interface EvalProgress {
  effective_execution_order: string[]
  control_aggregate: VariantAggregate
  treatment_aggregate: VariantAggregate
  delta: AggregateDelta
  order_sensitivity: OrderSensitivity
  runs: EvalRun[]
}

export interface EvalResultResponse {
  status: EvalStatus
  request: EvalRequest
  progress: EvalProgress
  report?: EvalReport
}

export interface CatalogModel {
  id: string
  provider: string
  display_name?: string
  context_window: number
  max_output_tokens: number
  supports_thinking?: boolean
  supports_xhigh?: boolean
}
