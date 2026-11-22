import type { SessionMeta } from '@/lib/sessions/types'

export const ALL_PROJECTS = 'all'

export interface TowerProject {
  id: string
  name: string
  description: string
  repos?: string[]
  total: number
  done: number
  open_questions: number
  counts: Record<string, number>
}

export interface TowerTicket {
  id: string
  key: string
  title: string
  status: string
  priority: string
  assignee?: string | null
  labels: string[]
  pills: Array<{ icon: string; text: string }>
  created_at: string
  updated_at: string
  parent_id?: string | null
  project_id?: string | null
  release_id?: string | null
  open_questions: number
}

export interface TowerRelease {
  id: string
  name: string
  status: 'planned' | 'active' | 'released'
  target_at?: string | null
  released_at?: string | null
  total: number
  done: number
  open_questions: number
  counts: Record<string, number>
  projects: Array<{
    project_id?: string | null
    total: number
    done: number
    open_questions: number
  }>
}

export interface TowerBoard {
  columns: Array<{
    id: string
    label: string
    tickets: TowerTicket[]
    total: number
  }>
  doneStatus: string
  canceledStatus?: string | null
  idPrefix: string
}

export interface HarnessStatus {
  session_id: string
  turn_id?: string | null
  status: string
  children: Array<{ session_id: string }>
  expects_wake: boolean
  open_asks: number
  armed_wakes?: Array<{ created_at: number; expires_at?: number | null }>
}

export interface TowerProgress {
  releaseId: string | null
  releaseName: string
  done: number
  inProgress: number
  total: number
}

interface NeedBase extends NeedSortValue {
  id: string
  sessionId: string
  title: string
  card?: TowerTicket
}

export interface QuestionNeed extends NeedBase {
  kind: 'question'
  questionId: string
  questionClass: AskClass
  question: string
  why?: string
  blocks: string[]
  state: 'open' | 'expired'
}

export interface HeldNeed extends NeedBase {
  kind: 'held'
  functionCallId: string
  functionId: string
  arguments: unknown
}

export interface StalledNeed extends NeedBase {
  kind: 'stalled'
  lastProgress: number
  armedWakes: HarnessStatus['armed_wakes']
}

export type NeedsYouItem = QuestionNeed | HeldNeed | StalledNeed

export interface TowerRun {
  session: SessionMeta
  status: HarnessStatus
  card?: TowerTicket
  subtasks: TowerTicket[] | null
  progress: TowerProgress[]
  needsYou: NeedsYouItem[]
  lastProgress: number
  lastAssistant: string
  state: 'working' | 'waiting' | 'stalled' | 'done' | 'failed'
  profile?: string
  startedBy: 'board dispatch' | 'chat' | 'automation'
  workingDir?: string
  descendantSessionIds: string[]
  metrics?: {
    complete: boolean
    totals: {
      sessions: number
      input_tokens: number | null
      output_tokens: number | null
    }
  }
}

export interface TowerPullRequest {
  repo: string
  number: number
  title: string
  url: string
  headRefName: string
  isDraft: boolean
  createdAt: string
  updatedAt: string
  checks: CheckState
  failing: number
  reviewDecision?: string | null
  mergeable?: string | null
  card?: TowerTicket
  waitReason: string | null
}

export type NeedUrgency = 'now' | 'no-rush'
export type AskClass =
  | 'security'
  | 'publish'
  | 'external_cost'
  | 'data'
  | 'scope'
  | 'run_health'
  | 'routine'

export interface NeedSortValue {
  kind: 'question' | 'held' | 'stalled'
  createdAt: number
  blockedCards: number
  questionClass?: AskClass
}

const URGENT_ASK_CLASSES = new Set<AskClass>([
  'run_health',
  'security',
  'publish',
  'external_cost',
  'data',
])

export function needUrgency(item: NeedSortValue): NeedUrgency {
  if (item.kind !== 'question') return 'now'
  return item.blockedCards > 0 ||
    URGENT_ASK_CLASSES.has(item.questionClass ?? 'scope')
    ? 'now'
    : 'no-rush'
}

/** Now first, then most blocked cards, then oldest. */
export function sortNeeds<T extends NeedSortValue>(items: readonly T[]): T[] {
  return [...items].sort((a, b) => {
    const urgency =
      Number(needUrgency(b) === 'now') - Number(needUrgency(a) === 'now')
    return (
      urgency || b.blockedCards - a.blockedCards || a.createdAt - b.createdAt
    )
  })
}

export interface StallFacts {
  status: string
  runningChildren: number
  openQuestions: number
  expectsWake: boolean
  armedWakes?: Array<{ expires_at?: number | null }>
  lastProgress: number
}

export function isStalled(
  run: StallFacts,
  now = Date.now(),
  stallAfterMin = 30,
): boolean {
  const wakes = run.armedWakes ?? []
  return (
    run.status === 'done' &&
    run.runningChildren === 0 &&
    run.openQuestions === 0 &&
    run.expectsWake &&
    !wakes.some((wake) =>
      typeof wake.expires_at === 'number' ? wake.expires_at > now : false,
    ) &&
    now - run.lastProgress >= stallAfterMin * 60_000
  )
}

export interface LinkableCard {
  key: string
  project_id?: string | null
}

function mentionsCard(text: string, key: string): boolean {
  return new RegExp(
    `(^|[^A-Z0-9])${key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}([^A-Z0-9]|$)`,
    'i',
  ).test(text)
}

export function linkedCard<T extends LinkableCard>(
  title: string,
  headBranch: string,
  cards: readonly T[],
): T | undefined {
  return cards.find(
    (card) =>
      mentionsCard(title, card.key) || mentionsCard(headBranch, card.key),
  )
}

export type CheckState = 'passing' | 'failing' | 'running' | 'none'

export interface PrWaitFacts {
  isDraft?: boolean
  checks: CheckState
  reviewDecision?: string | null
  mergeable?: string | null
  mergeStateStatus?: string | null
}

export function prWaitReason(pr: PrWaitFacts): string | null {
  if (pr.isDraft) return 'Draft'
  if (pr.checks === 'failing') return 'CI failing'
  if (pr.reviewDecision === 'REVIEW_REQUIRED') return 'Review requested'
  if (pr.mergeable === 'CONFLICTING' || pr.mergeStateStatus === 'DIRTY') {
    return 'Merge conflict'
  }
  return null
}

export function isInScope(
  scope: string,
  projectId: string | null | undefined,
): boolean {
  return scope === ALL_PROJECTS || projectId === scope
}
