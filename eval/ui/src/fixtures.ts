// Test fixtures shared by the unit tests: the limits the monitor reports today.
import type { MonitorLimits } from './types'

export const LIMITS: MonitorLimits = {
  analysis_budget_ms: 1_800_000,
  judge_timeout_ms: 60_000,
  model_context_bytes: 196_608,
  assets_bytes: 2_097_152,
  investigation_max_turns: 1,
  investigation_max_output_tokens: 16_384,
  investigation_max_total_tokens: 200_000,
  investigation_code_max_turns: 32,
  investigation_code_max_total_tokens: 800_000,
  queue_concurrency: 8,
  max_active_analyses: 500,
  low_confidence: 0.8,
  audit_sample_percent: 5,
  retention_days: 30,
  retention_max_terminal: 1000,
}
