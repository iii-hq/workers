// Wire shapes from the iii bus. Trimmed to what the demo viewer needs.

export type ContentBlock =
  | { type: "text"; text: string }
  | { type: "thinking"; text: string }
  | { type: string; [k: string]: unknown };

export interface UserMessage {
  role: "user";
  content: ContentBlock[];
  timestamp: number;
}

export interface AssistantMessage {
  role: "assistant";
  content: ContentBlock[];
  model?: string;
  provider?: string;
  stop_reason?: string;
  timestamp: number;
  usage?: {
    input?: number;
    output?: number;
    cache_read?: number;
    cache_write?: number;
  };
}

/** Function result row on the transcript (`role` stays `tool_result` on the wire). */
export interface FunctionResultMessage {
  role: "tool_result" | "function_result";
  content: ContentBlock[];
  /** New field names — prefer these when present. */
  function_call_id?: string;
  function_id?: string;
  /** Legacy field names (one release of stream replay). */
  tool_call_id?: string;
  tool_name?: string;
  is_error: boolean;
  timestamp: number;
}

export type AgentMessage = UserMessage | AssistantMessage | FunctionResultMessage;

export interface SessionRow {
  session_id: string;
  state: string;
  turn_count: number;
  updated_at_ms: number;
  /** Working directory associated with the session, if any. */
  cwd?: string | null;
  /** First text snippet of the last message, used as a session preview. */
  last_message_summary?: string | null;
}

// Per-session working directory. Advisory-only — surfaced in the system prompt
// so the agent prefers paths under cwd. Path scoping/enforcement belongs to
// `policy-denylist`, not the UI.
export interface Workspace {
  cwd: string;
  set_at: number;
}

export interface HarnessStatus {
  ok: boolean;
  name: string;
  version: string;
  expected_workers: string[];
}

export interface ModelInfo {
  id: string;
  display_name?: string;
  provider: string;
  api?: string;
  context_window?: number;
}

export interface AuthStatus {
  configured: boolean;
  source: "stored" | "environment" | "fallback" | "runtime" | null;
  label: string | null;
}

// llm-budget shapes (subset of workers/llm-budget/src/store.rs)
export type BudgetPeriod = "daily" | "weekly" | "monthly";

export interface BudgetAlert {
  alert_id: string;
  threshold_pct: number;
  callback_function_id: string;
  last_fired_period_start?: number | null;
}

export interface Budget {
  id: string;
  workspace_id?: string | null;
  agent_id?: string | null;
  name?: string | null;
  ceiling_usd: number;
  period: BudgetPeriod;
  spent_usd: number;
  period_start_at: number;
  period_resets_at: number;
  enforced: boolean;
  paused: boolean;
  alerts: BudgetAlert[];
  exemptions: unknown[];
  created_at: number;
  updated_at: number;
}

export interface BudgetUsageBucket {
  period: number;
  spent: number;
}

export interface BudgetUsage {
  spent_usd: number;
  by_period: BudgetUsageBucket[];
  records_count: number;
}

export interface BudgetForecast {
  projected_month_usd: number;
  rate_usd_per_day: number;
  remaining_budget_usd: number;
  days_until_breach: number | null;
  on_track: boolean;
}

// shell-filesystem shapes — ToolResult envelope, payload in `details`.
export interface ToolResult<D = unknown> {
  content: Array<{ type: string; text?: string }>;
  details: D;
  terminate?: boolean;
}

export interface FsEntry {
  name: string;
  kind?: "file" | "dir" | "symlink" | string;
  size?: number;
  mode?: string;
  mtime?: string | number;
}

export interface FsLsDetails {
  entries?: FsEntry[];
  error?: string;
}

export interface FsReadDetails {
  size?: number | null;
  mode?: number | string | null;
  mtime?: number | string | null;
  truncated?: boolean;
  bytes_read?: number;
  error?: string;
}

export type EntryId = string;

export type AgentEvent =
  | { type: "agent_start" }
  // Backend (turn-orchestrator/crates/harness-types/src/agent_event.rs) emits
  // bare AgentMessage[]. The reducer's agent_end handler also tolerates the
  // wrapped {entry_id?, message}[] shape for forward-compat with a future
  // backend that threads entry_ids through.
  | { type: "agent_end"; messages: (AgentMessage | { entry_id?: EntryId; message: AgentMessage })[] }
  | { type: "turn_start" }
  | { type: "turn_end"; message: AgentMessage; tool_results?: unknown[]; function_results?: unknown[]; entry_id?: EntryId }
  | { type: "message_start"; message: AgentMessage; entry_id?: EntryId }
  | { type: "message_update"; message: AgentMessage; llm_event: unknown; entry_id?: EntryId }
  | { type: "message_end"; message: AgentMessage; entry_id?: EntryId }
  | {
      type: "tool_execution_start";
      tool_call_id: string;
      tool_name: string;
      args: unknown;
    }
  | {
      type: "tool_execution_update";
      tool_call_id: string;
      tool_name: string;
      args: unknown;
      partial_result: unknown;
    }
  | {
      type: "tool_execution_end";
      tool_call_id: string;
      tool_name: string;
      result: unknown;
      is_error: boolean;
    }
  | {
      type: "function_execution_start";
      function_call_id: string;
      function_id: string;
      args: unknown;
    }
  | {
      type: "function_execution_update";
      function_call_id: string;
      function_id: string;
      args: unknown;
      partial_result: unknown;
    }
  | {
      type: "function_execution_end";
      function_call_id: string;
      function_id: string;
      result: unknown;
      is_error: boolean;
    }
  | {
      type: "approval_requested";
      function_call_id: string;
      function_id: string;
      args: unknown;
      expires_at: number;
      /** Legacy keys — optional for one release of mixed clients. */
      tool_call_id?: string;
      tool_name?: string;
    }
  | {
      type: "approval_resolved";
      function_call_id: string;
      decision: "allow" | "deny";
      reason?: string | null;
      tool_call_id?: string;
    };

export interface PendingApproval {
  function_call_id?: string;
  tool_call_id?: string;
  function_id?: string;
  tool_name?: string;
  args?: unknown;
  expires_at?: number;
}

export interface StreamState {
  messageMap: Map<EntryId, AgentMessage>;
  unkeyedMessages: AgentMessage[];   // backwards-compat for events without entry_id
  messageOrder: EntryId[];
  lastEntryId: EntryId | null;
  pendingApprovals: PendingApproval[];
  status: "idle" | "running" | "ended";
}

export const INITIAL_STREAM_STATE: StreamState = {
  messageMap: new Map(),
  unkeyedMessages: [],
  messageOrder: [],
  lastEntryId: null,
  pendingApprovals: [],
  status: "idle",
};
