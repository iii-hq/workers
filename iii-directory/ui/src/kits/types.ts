/**
 * Wire shapes of `directory::download-kit` and `directory::kits::*`
 * (iii-directory/src/kits/*.rs). The plan is the contract between the
 * functions, the Kits page and the chat cards.
 */

export type Decision = 'overwrite' | 'keep' | 'kit' | 'mine' | 'merged' | 'remove'
export type SourceKind = 'kit' | 'worker' | 'local' | 'global' | 'builtin'
export type Change = 'added' | 'modified' | 'removed' | 'kept'
export type LocalState = 'absent' | 'intact' | 'edited' | 'missing' | 'occupied'
export type WorkerAction = 'add' | 'update' | 'redeclare' | 'none' | 'remove'
export type PlanKind = 'install' | 'update' | 'remove'
export type FileState = 'intact' | 'edited' | 'missing' | 'skipped'
export type StepState = 'pending' | 'running' | 'done' | 'failed' | 'skipped'

export interface Author {
  handle: string
  name?: string | null
  verified?: boolean
}

export interface Deprecation {
  message?: string | null
  replaced_by?: string | null
}

export interface AgentEntry {
  id: string
  path: string
  sha256: string
  name?: string | null
  description?: string | null
  logo?: string | null
  icon?: string | null
  color?: string | null
  model?: string | null
  reasoning_effort?: string | null
  extends?: string | null
  hidden?: boolean | null
  skills?: string[]
  functions?: string[]
}

export interface SkillEntry {
  id: string
  path: string
  sha256: string
  title?: string | null
  description?: string | null
  type?: string | null
  used_by?: string[]
}

export interface Collision {
  owner: SourceKind
  kit?: string
  worker?: string
  version?: string
  sha256?: string
  modified?: boolean
}

export interface AgentChanges {
  fields?: Record<string, [string | null, string | null]>
  functions_added?: string[]
  functions_removed?: string[]
  skills_added?: string[]
  skills_removed?: string[]
}

export interface PlanFile {
  path: string
  source: string
  kind: 'agent' | 'skill'
  id: string
  change: Change
  local: LocalState
  base?: string
  theirs?: string
  ours?: string
  collision?: Collision
  merge?: 'clean' | 'conflicts'
  conflicts?: number
  default: Decision | null
  options: Decision[]
  skipped?: boolean
  note?: string
  agent?: AgentEntry
  agent_changes?: AgentChanges
  body_changed?: boolean
  skill?: SkillEntry
}

export interface PlanWorker {
  name: string
  range?: string
  range_from?: string
  installed: string | null
  declared?: string
  container?: string
  action: WorkerAction
  to?: string
  type?: string
  description?: string
  path?: boolean
  used_by?: string[]
  registry_url: string
}

export interface Issue {
  code: string
  path?: string
  message: string
}

export interface KitFunction {
  id: string
  used_by?: string[]
  status?: string | null
  worker?: string | null
  worker_version?: string | null
  description?: string | null
}

export interface Capabilities {
  workers_added?: string[]
  functions_added?: { agent: string; function: string }[]
  models_changed?: { agent: string; from?: string | null; to?: string | null }[]
}

export interface PlanCounts {
  agents: number
  skills: number
  workers: number
  functions: number
  added: number
  modified: number
  removed: number
  unchanged: number
  collisions: number
  conflicts: number
  decisions_required: number
}

export interface Plan {
  plan_id: string
  kind: PlanKind
  kit: string
  from: string | null
  to: string | null
  requested: string
  bump?: string
  major: boolean
  author?: Author
  description?: string
  license?: string
  repo?: string
  published_at?: string
  registry_url: string
  notes?: string
  deprecation?: Deprecation
  workers: PlanWorker[]
  files: PlanFile[]
  capabilities: Capabilities
  functions: KitFunction[]
  warnings: Issue[]
  blocking: Issue[]
  counts: PlanCounts
  created_at: string
  expires_at: string
  review: string
}

export interface PlanContents {
  blobs: Record<string, string>
  ours: Record<string, string>
  merged: Record<string, string>
}

export interface ApplyStep {
  id: string
  label: string
  state: StepState
  detail?: string | null
}

export interface ApplyProgress {
  steps: ApplyStep[]
  written?: Record<string, string>
  error?: string | null
  started_at?: string | null
}

export interface ApplyReport {
  plan_id: string
  kind: PlanKind | null
  kit: string
  from: string | null
  to: string | null
  steps: ApplyStep[]
  files_written: string[]
  files_merged: string[]
  files_removed: string[]
  files_kept: string[]
  workers_added: string[]
  workers_updated: string[]
  workers_removed: string[]
  agents: string[]
  skills_changed: boolean
  agents_changed: boolean
  lock_path: string
}

export type PlanStatus = 'planned' | 'applied' | 'up_to_date' | 'replanned'

export interface KitPlanResponse {
  status: PlanStatus
  kit: string
  message: string
  plan?: Plan
  report?: ApplyReport
}

export interface KitUpdate {
  current: string
  available?: string | null
  latest?: string | null
  major: boolean
  ignored?: string | null
  deprecation?: Deprecation | null
  not_found?: boolean
}

export interface FileCounts {
  agents: number
  skills: number
  intact: number
  edited: number
  missing: number
  skipped: number
}

export interface InstalledKit {
  kit: string
  version: string
  requested: string
  installed_at: string
  workers: Record<string, string>
  files: FileCounts
  update?: KitUpdate | null
  registry_url: string
  pending: string[]
}

export interface PlanSummary {
  plan_id: string
  kind: PlanKind
  kit: string
  from: string | null
  to: string | null
  major: boolean
  counts: PlanCounts
  warnings: number
  blocking: number
  created_at: string
  expires_at: string
}

export interface KitsListing {
  kits: InstalledKit[]
  pending: PlanSummary[]
  updates_checked_at?: string | null
  updates_error?: string | null
  attention: number
  lock_path: string
}

export interface InstalledFile {
  path: string
  kind: 'agent' | 'skill'
  id: string
  state: FileState
  sha256: string
  local_sha256?: string | null
}

export interface WorkerStatus {
  name: string
  range: string
  declared?: string | null
  installed?: string | null
  satisfied?: boolean | null
  path?: boolean
  registry_url: string
}

export interface KitDetail {
  id: string
  author?: Author
  description?: string | null
  version: string
  license?: string | null
  repo?: string | null
  notes?: string | null
  published_at?: string | null
  agents?: AgentEntry[]
  skills?: SkillEntry[]
  workers?: {
    name: string
    range: string
    resolved?: string | null
    type?: string | null
    description?: string | null
  }[]
}

export interface KitInfo extends InstalledKit {
  file_list: InstalledFile[]
  detail?: KitDetail | null
  readme?: string | null
  worker_status: WorkerStatus[]
}

/** `directory::kits::on-change` payloads. */
export interface KitsChange {
  op: 'plan' | 'discard' | 'progress' | 'apply' | 'updates'
  kit?: string | null
  plan_id?: string
  steps?: ApplyStep[]
  error?: string | null
}
