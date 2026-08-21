import { z } from 'zod';

export const RuntimeSchema = z.enum(['local', 'cloud']);
export type Runtime = z.infer<typeof RuntimeSchema>;

export const TokenUsageSchema = z.object({
  input_tokens: z.number().int().nonnegative(),
  output_tokens: z.number().int().nonnegative(),
  cache_read_tokens: z.number().int().nonnegative(),
  cache_write_tokens: z.number().int().nonnegative(),
  total_tokens: z.number().int().nonnegative(),
  reasoning_tokens: z.number().int().nonnegative().optional(),
});
export type TokenUsage = z.infer<typeof TokenUsageSchema>;

export const UsageCostSchema = z.object({
  raw_cost_cents: z.number().nonnegative(),
  charged_cents: z.number().nonnegative(),
});
export type UsageCost = z.infer<typeof UsageCostSchema>;

export const RepositorySchema = z.object({
  url: z.string().min(1),
  starting_ref: z.string().optional(),
  pr_url: z.string().url().optional(),
});
export type Repository = z.infer<typeof RepositorySchema>;

export const SessionRecordSchema = z.object({
  session_id: z.string(),
  agent_id: z.string(),
  runtime: RuntimeSchema,
  backend: z.enum(['cli-acp', 'sdk-bridge']).optional(),
  workspace: z.string(),
  name: z.string().nullable(),
  model: z.string(),
  tools: z.array(z.string()),
  repositories: z.array(RepositorySchema),
  work_on_current_branch: z.boolean(),
  auto_create_pr: z.boolean(),
  status: z.enum(['working', 'done', 'error', 'cancelled', 'recovery-required']),
  agent_created: z.boolean(),
  turns: z.number().int().nonnegative(),
  active_turn: z.number().int().positive().nullable(),
  active_run_id: z.string().nullable(),
  last_run_id: z.string().nullable(),
  create_idempotency_key: z.string(),
  send_idempotency_key: z.string().nullable(),
  send_started: z.boolean(),
  cancel_requested: z.boolean(),
  claim_id: z.string().nullable(),
  claim_started_at_ms: z.number().int().nonnegative().nullable(),
  pending_prompt_sha256: z.string().nullable(),
  usage: TokenUsageSchema.nullable(),
  cost: UsageCostSchema.nullable(),
  updated_at_ms: z.number().int().nonnegative(),
});
export type SessionRecord = z.infer<typeof SessionRecordSchema>;

export const ModelParameterValueSchema = z.object({
  id: z.string(),
  value: z.string(),
});

export const ModelParameterDefinitionSchema = z.object({
  id: z.string(),
  display_name: z.string(),
  values: z.array(z.object({ value: z.string(), display_name: z.string() })),
});

export const ModelVariantSchema = z.object({
  params: z.array(ModelParameterValueSchema),
  display_name: z.string(),
  description: z.string(),
  is_default: z.boolean(),
});

export const CursorModelSchema = z.object({
  id: z.string(),
  display_name: z.string(),
  description: z.string(),
  parameters: z.array(ModelParameterDefinitionSchema),
  variants: z.array(ModelVariantSchema),
});
export type CursorModel = z.infer<typeof CursorModelSchema>;

export const AgentInfoSchema = z.object({
  agent_id: z.string(),
  name: z.string(),
  summary: z.string(),
  status: z.string(),
  archived: z.boolean(),
  created_at: z.string().nullable(),
  last_modified: z.string().nullable(),
  runtime: RuntimeSchema.nullable(),
  cwd: z.string().nullable(),
  repositories: z.array(z.string()),
  metadata: z.record(z.string(), z.string()),
});
export type AgentInfo = z.infer<typeof AgentInfoSchema>;

export const RunSnapshotSchema = z.object({
  run_id: z.string(),
  agent_id: z.string(),
  status: z.string(),
  result: z.string(),
  model: z.string(),
  duration_ms: z.number().int().nonnegative().nullable(),
  created_at: z.string().nullable(),
  usage: TokenUsageSchema.nullable(),
});
export type RunSnapshot = z.infer<typeof RunSnapshotSchema>;

export const RunUsageSchema = z.object({
  run_id: z.string(),
  usage: TokenUsageSchema,
  cost: UsageCostSchema.nullable(),
});

export const AgentUsageSchema = z.object({
  usage: TokenUsageSchema.nullable(),
  cost: UsageCostSchema.nullable(),
  runs: z.array(RunUsageSchema),
});
export type AgentUsage = z.infer<typeof AgentUsageSchema>;

export type TextContent = { type: 'text'; text: string };
export type ThinkingContent = { type: 'thinking'; text: string; signature?: string };
export type FunctionCallContent = {
  type: 'function_call';
  id: string;
  function_id: string;
  arguments: unknown;
};
export type FunctionResultContent = {
  type: 'function_result';
  function_call_id: string;
  content: ContentBlock[];
  is_error?: boolean;
};
export type ContentBlock =
  | TextContent
  | ThinkingContent
  | FunctionCallContent
  | FunctionResultContent;

export type AssistantMessage = {
  role: 'assistant';
  content: ContentBlock[];
  stop_reason: 'end' | 'length' | 'aborted' | 'error';
  error_message?: string | null;
  usage?: TokenUsage | null;
  model: string;
  provider: 'cursor';
  timestamp: number;
};

export type FunctionResultMessage = {
  role: 'function_result';
  function_call_id: string;
  function_id: string;
  content: ContentBlock[];
  details: unknown;
  is_error: boolean;
  timestamp: number;
};

export type AgentMessage = AssistantMessage | FunctionResultMessage;

export type AgentEvent =
  | { type: 'message_update'; llm_event: { type: 'text_delta' | 'thinking_delta'; delta: string } }
  | { type: 'message_complete'; message: AssistantMessage; body_streamed?: boolean }
  | {
      type: 'function_execution_start';
      function_call_id: string;
      function_id: string;
      args: unknown;
    }
  | {
      type: 'function_execution_end';
      function_call_id: string;
      function_id: string;
      result: { content: ContentBlock[]; details: unknown };
      is_error: boolean;
      duration_ms: number;
    }
  | { type: 'turn_end'; message: AssistantMessage; function_results: FunctionResultMessage[] }
  | { type: 'agent_end'; messages: AgentMessage[] };

export const Int64WireSchema = z.union([z.number(), z.string().regex(/^\d+$/)]);
export const StructWireSchema = z.record(z.string(), z.unknown());
export const EnumWireSchema = z.union([z.string(), z.number().int()]);

export const ModelSelectionWireSchema = z
  .object({
    id: z.string().optional(),
    params: z
      .array(z.object({ id: z.string().optional(), value: z.string().optional() }).passthrough())
      .optional(),
  })
  .passthrough();

export const TokenUsageWireSchema = z
  .object({
    inputTokens: Int64WireSchema.optional(),
    outputTokens: Int64WireSchema.optional(),
    cacheReadTokens: Int64WireSchema.optional(),
    cacheWriteTokens: Int64WireSchema.optional(),
    totalTokens: Int64WireSchema.optional(),
    reasoningTokens: Int64WireSchema.optional(),
  })
  .passthrough();

export const UsageCostWireSchema = z
  .object({ rawCostCents: z.number().optional(), chargedCents: z.number().optional() })
  .passthrough();

export const RunResultWireSchema = z
  .object({
    runId: z.string().optional(),
    agentId: z.string().optional(),
    status: EnumWireSchema.optional(),
    result: z.string().optional(),
    model: ModelSelectionWireSchema.optional(),
    durationMs: Int64WireSchema.optional(),
    createdAt: z.string().optional(),
    usage: TokenUsageWireSchema.optional(),
  })
  .passthrough();

export const RunStreamMessageWireSchema = z
  .object({
    sdkMessage: z
      .object({ type: z.string(), message: StructWireSchema.default({}) })
      .passthrough()
      .optional(),
    result: z
      .object({
        agentId: z.string().optional(),
        runId: z.string().optional(),
        status: EnumWireSchema.optional(),
        errorCode: z.string().optional(),
        result: RunResultWireSchema.optional(),
      })
      .passthrough()
      .optional(),
    done: z
      .object({ agentId: z.string().optional(), runId: z.string().optional() })
      .passthrough()
      .optional(),
    interactionUpdate: z
      .object({ type: z.string(), update: StructWireSchema.default({}) })
      .passthrough()
      .optional(),
    step: z
      .object({ type: z.string(), step: StructWireSchema.default({}) })
      .passthrough()
      .optional(),
    offset: z.string().optional(),
  })
  .passthrough();
export type RunStreamMessageWire = z.infer<typeof RunStreamMessageWireSchema>;

export const CreateAgentResponseWireSchema = z
  .object({ agentId: z.string(), model: ModelSelectionWireSchema.optional() })
  .passthrough();

export const GetAgentResponseWireSchema = z
  .object({
    agent: z
      .object({
        agentId: z.string(),
        name: z.string().optional(),
        summary: z.string().optional(),
        status: EnumWireSchema.optional(),
        createdAt: z.string().optional(),
        lastModified: z.string().optional(),
        archived: z.boolean().optional(),
        local: z.object({ cwd: z.string().optional() }).passthrough().optional(),
        cloud: z
          .object({
            repos: z.array(z.string()).optional(),
            metadata: z.record(z.string(), z.string()).optional(),
          })
          .passthrough()
          .optional(),
      })
      .passthrough(),
  })
  .passthrough();

export const GetRunResponseWireSchema = z.object({ run: RunResultWireSchema }).passthrough();

export const ListModelsResponseWireSchema = z
  .object({
    items: z.array(
      z
        .object({
          id: z.string(),
          displayName: z.string().optional(),
          description: z.string().optional(),
          parameters: z
            .array(
              z
                .object({
                  id: z.string(),
                  displayName: z.string().optional(),
                  values: z
                    .array(
                      z
                        .object({ value: z.string(), displayName: z.string().optional() })
                        .passthrough(),
                    )
                    .optional(),
                })
                .passthrough(),
            )
            .optional(),
          variants: z
            .array(
              z
                .object({
                  params: z
                    .array(z.object({ id: z.string(), value: z.string() }).passthrough())
                    .optional(),
                  displayName: z.string().optional(),
                  description: z.string().optional(),
                  isDefault: z.boolean().optional(),
                })
                .passthrough(),
            )
            .optional(),
        })
        .passthrough(),
    ),
  })
  .passthrough();

export const GetUsageResponseWireSchema = z
  .object({
    usage: z
      .object({
        usage: TokenUsageWireSchema.optional(),
        cost: UsageCostWireSchema.optional(),
        runs: z
          .array(
            z
              .object({
                runId: z.string(),
                usage: TokenUsageWireSchema.optional(),
                cost: UsageCostWireSchema.optional(),
              })
              .passthrough(),
          )
          .optional(),
      })
      .passthrough()
      .optional(),
  })
  .passthrough();
