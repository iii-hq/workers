/**
 * The mcp-client worker's wire shapes, field for field as serde emits them
 * (snake_case; a Rust `Option<T>` is `T | null`; timestamps are unix ms).
 */

export type ServerState =
  | 'disabled'
  | 'config_error'
  | 'unapproved'
  | 'connecting'
  | 'ready'
  | 'unavailable'
  | 'auth_failed'
  | 'needs_login'
export type Stage = 'config' | 'approval' | 'secret' | 'connect' | 'authorize' | 'tools'
export type ErrorCode =
  | 'invalid_config'
  | 'stdio_disabled'
  | 'unapproved'
  | 'secret_missing'
  | 'secret_not_shared'
  | 'spawn_failed'
  | 'exited'
  | 'connect_failed'
  | 'handshake_failed'
  | 'credential_rejected'
  | 'login_required'
  | 'rate_limited'
  | 'list_failed'
export type AuthKind = 'none' | 'bearer' | 'header' | 'client_credentials' | 'oauth'

/** The MCP tool annotations the server sent, as it sent them. */
export interface ToolAnnotations {
  readOnlyHint?: boolean
  destructiveHint?: boolean
  idempotentHint?: boolean
  openWorldHint?: boolean
  title?: string
}

export interface ToolRow {
  name: string
  function_id: string
  registered: boolean
  annotations: ToolAnnotations | null
}

export interface ResourceFn {
  name: 'list' | 'read' | 'templates'
  function_id: string
  registered: boolean
}

export interface PromptRow {
  name: string
  function_id: string
  registered: boolean
  title: string | null
  arguments: number
  required: number
}

export interface Skipped {
  kind: 'tool' | 'prompt'
  name: string
  reason: string
}

export interface ProcessInfo {
  pid: number | null
  started_at: number | null
  restarts: number
  restart_budget: number
  next_restart_at: number | null
  last_exit: { code: number | null; signal: number | null } | null
}

export interface LoginInfo {
  pending: boolean
  expires_at: number
}

export interface Server {
  name: string
  transport: 'http' | 'stdio'
  target: string
  auth: AuthKind
  state: ServerState
  stage: Stage | null
  since: number
  error: { code: ErrorCode; message: string } | null
  approved: boolean
  fingerprint: string
  protocol_version: string | null
  server_info: { name: string; title: string | null; version: string } | null
  tools_listed: boolean
  tools: ToolRow[]
  skipped: Skipped[]
  has_instructions: boolean
  capabilities: { resources: boolean; prompts: boolean }
  resource_functions: ResourceFn[]
  prompts: PromptRow[]
  list_errors: { prompts: string | null; templates: string | null }
  login: LoginInfo | null
  process: ProcessInfo | null
}

export interface ServersList {
  stdio_enabled: boolean
  callback_url: string
  servers: Server[]
}

export interface Issue {
  /** JSON pointer into the candidate value. */
  path: string
  message: string
}

export interface Effects {
  runs: string | null
  sends: { ref: string; host: string }[]
}

export interface ServerValidation {
  ok: boolean
  errors: Issue[]
  warnings: Issue[]
  fingerprint: string
  effects: Effects
}

export interface ValidateResponse {
  servers: Record<string, ServerValidation>
}

export interface ProcessLogs {
  lines: { ts: number; text: string }[]
  dropped: number
}

export interface LoginStart {
  authorize_url: string
  redirect_uri: string
  expires_at: number
}

/** One row of `engine::functions::list`. */
export interface EngineFunction {
  function_id: string
  description?: string
  metadata?: Record<string, unknown>
}

/** The reply of `engine::functions::info`. */
export interface FunctionInfo {
  function_id: string
  description?: string
  request_schema?: unknown
  response_schema?: unknown
  metadata?: Record<string, unknown>
}
