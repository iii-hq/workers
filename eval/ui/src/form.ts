import type { ComparisonDimension, EvalRequest, JsonValue } from './types'

export type SystemPromptSource = 'default' | 'custom' | 'none'

export interface EvalFormState {
  dimension: ComparisonDimension
  model: string
  provider: string
  runs: string
  controlLabel: string
  treatmentLabel: string
  controlPrompt: string
  treatmentPrompt: string
  sharedSystemPromptSource: SystemPromptSource
  sharedSystemPrompt: string
  sharedUserPrompt: string
  controlSystemPromptSource: SystemPromptSource
  controlSystemPrompt: string
  treatmentSystemPromptSource: SystemPromptSource
  treatmentSystemPrompt: string
  evaluatorMode: 'normalized_text' | 'exact' | 'custom'
  expectedFormat: 'text' | 'json'
  expectedValue: string
  evaluatorFunctionId: string
  evaluatorArguments: string
  maxTotalTokens: string
  maxCostUsd: string
  invocationTimeoutSeconds: string
  scenarioTimeoutSeconds: string
  maxTurns: string
  maxOutputTokensPerCall: string
  maxFunctionCallErrors: string
  maxErrorSpans: string
  systemPromptStrategy: 'override' | 'enrich'
  mode: '' | 'ask' | 'agent'
  thinkingLevel: '' | 'minimal' | 'low' | 'medium' | 'high' | 'xhigh'
  functionsJson: string
  outputJson: string
  metadataJson: string
  providerOptionsJson: string
}

export const DEFAULT_FORM: EvalFormState = {
  // Matches the first tab in NewEvaluationForm — the leading option is the
  // one that opens selected.
  dimension: 'system_prompt',
  model: '',
  provider: '',
  runs: '1',
  controlLabel: 'baseline',
  treatmentLabel: 'candidate',
  controlPrompt: '',
  treatmentPrompt: '',
  sharedSystemPromptSource: 'default',
  sharedSystemPrompt: '',
  sharedUserPrompt: '',
  controlSystemPromptSource: 'custom',
  controlSystemPrompt: '',
  treatmentSystemPromptSource: 'custom',
  treatmentSystemPrompt: '',
  evaluatorMode: 'normalized_text',
  expectedFormat: 'text',
  expectedValue: '',
  evaluatorFunctionId: '',
  evaluatorArguments: '{}',
  maxTotalTokens: '',
  maxCostUsd: '',
  invocationTimeoutSeconds: '',
  scenarioTimeoutSeconds: '',
  maxTurns: '',
  maxOutputTokensPerCall: '',
  maxFunctionCallErrors: '',
  maxErrorSpans: '',
  systemPromptStrategy: 'override',
  mode: 'agent',
  thinkingLevel: '',
  functionsJson: '{"allow":["*"],"deny":["approval::*","configuration::*"],"expose":"agent_trigger"}',
  outputJson: '{"type":"text"}',
  metadataJson: '',
  providerOptionsJson: '',
}

export type BuildResult =
  | { request: EvalRequest; errors: Record<string, never> }
  | { request: null; errors: Record<string, string> }

export function buildRequest(form: EvalFormState): BuildResult {
  const errors: Record<string, string> = {}
  const model = form.model.trim()
  const provider = form.provider.trim()
  if (!model) errors.model = 'model is required'

  const runs = integer(form.runs, 'runs', errors, 1)
  if (runs !== null && runs > 20) errors.runs = 'runs must be at most 20'
  const maxTotalTokens = optionalInteger(form.maxTotalTokens, 'maxTotalTokens', errors, 1)
  const invocationTimeoutSeconds = optionalInteger(
    form.invocationTimeoutSeconds,
    'invocationTimeoutSeconds',
    errors,
    1,
  )
  const scenarioTimeoutSeconds = optionalInteger(form.scenarioTimeoutSeconds, 'scenarioTimeoutSeconds', errors, 1)
  const maxTurns = optionalInteger(form.maxTurns, 'maxTurns', errors, 1)
  const maxOutputTokensPerCall = optionalInteger(
    form.maxOutputTokensPerCall,
    'maxOutputTokensPerCall',
    errors,
    1,
  )
  const maxFunctionCallErrors = optionalInteger(
    form.maxFunctionCallErrors,
    'maxFunctionCallErrors',
    errors,
    0,
  )
  const maxErrorSpans = optionalInteger(form.maxErrorSpans, 'maxErrorSpans', errors, 0)
  const maxCostUsd = optionalNumber(form.maxCostUsd, 'maxCostUsd', errors, 0)

  if (form.dimension === 'prompt') {
    if (!form.controlPrompt.trim()) errors.controlPrompt = 'baseline prompt is required'
    if (!form.treatmentPrompt.trim()) errors.treatmentPrompt = 'candidate prompt is required'
    if (form.sharedSystemPromptSource === 'custom' && !form.sharedSystemPrompt.trim()) {
      errors.sharedSystemPrompt = 'custom system prompt is required'
    }
    if (form.controlPrompt.trim() && form.treatmentPrompt.trim() && form.controlPrompt === form.treatmentPrompt) {
      errors.treatmentPrompt = 'candidate prompt must differ from baseline'
    }
  } else {
    if (form.controlSystemPromptSource === 'custom' && !form.controlSystemPrompt.trim()) {
      errors.controlSystemPrompt = 'baseline system prompt is required'
    }
    if (form.treatmentSystemPromptSource === 'custom' && !form.treatmentSystemPrompt.trim()) {
      errors.treatmentSystemPrompt = 'candidate system prompt is required'
    }
    if (!form.sharedUserPrompt.trim()) errors.sharedUserPrompt = 'shared user prompt is required'
    const controlSystemPrompt = systemPromptValue(form.controlSystemPromptSource, form.controlSystemPrompt)
    const treatmentSystemPrompt = systemPromptValue(form.treatmentSystemPromptSource, form.treatmentSystemPrompt)
    if (!errors.controlSystemPrompt && !errors.treatmentSystemPrompt && controlSystemPrompt === treatmentSystemPrompt) {
      errors.treatmentSystemPrompt = 'candidate system prompt must differ from baseline'
    }
  }

  const evaluatorEnabled =
    form.evaluatorMode === 'custom' ? Boolean(form.evaluatorFunctionId.trim()) : Boolean(form.expectedValue.trim())
  let evaluator: EvalRequest['evaluator']
  if (evaluatorEnabled && form.evaluatorMode !== 'custom') {
    const expected =
      form.evaluatorMode === 'exact' && form.expectedFormat === 'json'
        ? parseJson(form.expectedValue, 'expectedValue', errors)
        : form.expectedValue
    evaluator = {
      function_id: form.evaluatorMode === 'normalized_text' ? 'eval::assert::normalized_text' : 'eval::assert::exact',
      arguments: { expected: expected ?? null },
    }
  } else if (evaluatorEnabled) {
    evaluator = {
      function_id: form.evaluatorFunctionId.trim(),
      arguments: parseJson(form.evaluatorArguments, 'evaluatorArguments', errors) ?? {},
    }
  }

  const functions = optionalObject(form.functionsJson, 'functionsJson', errors)
  const output = optionalObject(form.outputJson, 'outputJson', errors)
  const metadata = optionalObject(form.metadataJson, 'metadataJson', errors)
  const providerOptions = optionalObject(form.providerOptionsJson, 'providerOptionsJson', errors)

  if (Object.keys(errors).length > 0) return { request: null, errors }

  const request: EvalRequest = {
    dimension: form.dimension,
    model: {
      model,
      ...(provider ? { provider } : {}),
      system_prompt_strategy: form.systemPromptStrategy,
      ...(form.mode ? { mode: form.mode } : {}),
      ...(form.thinkingLevel ? { thinking_level: form.thinkingLevel } : {}),
      ...(providerOptions ? { provider_options: providerOptions } : {}),
    },
    control: {
      ...(form.controlLabel.trim() ? { label: form.controlLabel.trim() } : {}),
      prompt: form.dimension === 'prompt' ? form.controlPrompt : form.sharedUserPrompt,
      system_prompt:
        form.dimension === 'prompt'
          ? systemPromptValue(form.sharedSystemPromptSource, form.sharedSystemPrompt)
          : systemPromptValue(form.controlSystemPromptSource, form.controlSystemPrompt),
    },
    treatment: {
      ...(form.treatmentLabel.trim() ? { label: form.treatmentLabel.trim() } : {}),
      prompt: form.dimension === 'prompt' ? form.treatmentPrompt : form.sharedUserPrompt,
      system_prompt:
        form.dimension === 'prompt'
          ? systemPromptValue(form.sharedSystemPromptSource, form.sharedSystemPrompt)
          : systemPromptValue(form.treatmentSystemPromptSource, form.treatmentSystemPrompt),
    },
    ...(evaluator ? { evaluator } : {}),
    runs: runs as number,
    execution_order: 'balanced_control_first',
    limits: {
      execution: {
        ...(invocationTimeoutSeconds === null
          ? {}
          : { invocation_timeout_seconds: invocationTimeoutSeconds }),
        ...(scenarioTimeoutSeconds === null ? {} : { scenario_timeout_seconds: scenarioTimeoutSeconds }),
        ...(maxTurns === null ? {} : { max_turns: maxTurns }),
        ...(maxOutputTokensPerCall === null
          ? {}
          : { max_output_tokens_per_call: maxOutputTokensPerCall }),
        ...(maxTotalTokens === null ? {} : { max_total_tokens: maxTotalTokens }),
        ...(maxCostUsd === null ? {} : { max_cost_usd: maxCostUsd }),
      },
      evaluation: {
        ...(maxFunctionCallErrors === null ? {} : { max_function_call_errors: maxFunctionCallErrors }),
        ...(maxErrorSpans === null ? {} : { max_error_spans: maxErrorSpans }),
      },
    },
    ...(functions ? { functions: functions as unknown as EvalRequest['functions'] } : {}),
    ...(output ? { output: output as unknown as EvalRequest['output'] } : {}),
    ...(metadata ? { metadata } : {}),
  }
  return { request, errors: {} }
}

function systemPromptValue(source: SystemPromptSource, customPrompt: string): string | null {
  if (source === 'none') return null
  if (source === 'default') return ''
  return customPrompt
}

function integer(raw: string, field: string, errors: Record<string, string>, minimum: number): number | null {
  const value = Number(raw)
  if (!Number.isInteger(value) || value < minimum) {
    errors[field] = `must be an integer of at least ${minimum}`
    return null
  }
  return value
}

function optionalInteger(raw: string, field: string, errors: Record<string, string>, minimum: number): number | null {
  if (!raw.trim()) return null
  return integer(raw, field, errors, minimum)
}

function optionalNumber(raw: string, field: string, errors: Record<string, string>, minimum: number): number | null {
  if (!raw.trim()) return null
  const value = Number(raw)
  if (!Number.isFinite(value) || value < minimum) {
    errors[field] = `must be a number of at least ${minimum}`
    return null
  }
  return value
}

function parseJson(raw: string, field: string, errors: Record<string, string>): JsonValue | null {
  try {
    return JSON.parse(raw) as JsonValue
  } catch (error) {
    errors[field] = error instanceof Error ? `invalid JSON: ${error.message}` : 'invalid JSON'
    return null
  }
}

function parseObject(raw: string, field: string, errors: Record<string, string>): Record<string, JsonValue> | null {
  const value = parseJson(raw, field, errors)
  if (value === null || Array.isArray(value) || typeof value !== 'object') {
    if (!errors[field]) errors[field] = 'must be a JSON object'
    return null
  }
  return value
}

function optionalObject(raw: string, field: string, errors: Record<string, string>): Record<string, JsonValue> | null {
  if (!raw.trim()) return null
  return parseObject(raw, field, errors)
}
