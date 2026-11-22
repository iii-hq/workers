import type { TowerData } from './data'
import {
  type NeedsYouItem,
  sortNeeds,
  type TowerRun,
  type TowerTicket,
} from './model'

const now = Date.now()
const ago = (minutes: number) => now - minutes * 60_000
const iso = (minutes: number) => new Date(ago(minutes)).toISOString()

const card = (
  key: string,
  title: string,
  status = 'in_progress',
): TowerTicket => ({
  id: key,
  key,
  title,
  status,
  priority: 'medium',
  labels: [],
  pills: [],
  project_id: 'assistente',
  release_id: 'r1',
  parent_id: null,
  created_at: iso(300),
  updated_at: iso(18),
  open_questions: 0,
})
const r1 = card('KAN-616', 'Assistente · R1')
const flaky = card('KAN-680', 'deepwork: flaky tests')
const search = card(
  'KAN-711',
  'Search filters persist across reloads',
  'in_review',
)
const security = card(
  'KAN-597',
  'Decide what the build machine may run and keep',
)
const tasks = [
  card('KAN-635', 'Email link names the container label', 'done'),
  card('KAN-618', 'Spec tags on every notify test', 'done'),
  card('KAN-632', 'Crashloop recovery keeps tracing', 'in_progress'),
  card('KAN-598', 'Run the build in a sandbox', 'todo'),
]
r1.pills = [{ icon: 'check', text: 'validating · round 23/30' }]
r1.open_questions = 2
const items: NeedsYouItem[] = sortNeeds([
  {
    kind: 'question',
    id: 'security',
    sessionId: 'dispatch-r1',
    questionId: 'security',
    questionClass: 'security',
    title: 'What may the assistant’s build machine run and keep?',
    question: 'What may the assistant’s build machine run and keep?',
    why: 'Isolate builds and keep the keys off that machine.',
    blocks: ['KAN-598', 'KAN-632', 'KAN-635'],
    blockedCards: 3,
    card: security,
    state: 'open',
    createdAt: ago(120),
  },
  {
    kind: 'question',
    id: 'verify',
    sessionId: 'dispatch-r1',
    questionId: 'verify',
    questionClass: 'run_health',
    title: 'Verify timed out twice in round 23',
    question: 'Verify timed out twice in round 23',
    why: 'Retry only the checks that did not finish.',
    blocks: ['KAN-618'],
    blockedCards: 1,
    card: r1,
    state: 'expired',
    createdAt: ago(18),
  },
  {
    kind: 'held',
    id: 'hold',
    sessionId: 'dispatch-r1',
    functionCallId: 'push',
    functionId: 'shell::exec',
    arguments: { command: 'git push' },
    title: 'shell::exec',
    card: r1,
    blockedCards: 0,
    createdAt: ago(12),
  },
  {
    kind: 'stalled',
    id: 'stall',
    sessionId: 'dispatch-flaky',
    title: 'deepwork: flaky tests is stalled',
    card: flaky,
    lastProgress: ago(46),
    armedWakes: [{ created_at: ago(46) }],
    blockedCards: 0,
    createdAt: ago(46),
  },
  {
    kind: 'question',
    id: 'csv',
    sessionId: 'import-tool',
    questionId: 'csv',
    questionClass: 'scope',
    title: 'Should the import tool also accept semicolon-separated files?',
    question: 'Should the import tool also accept semicolon-separated files?',
    blocks: [],
    blockedCards: 0,
    state: 'open',
    createdAt: ago(9),
  },
])
const run = (
  id: string,
  title: string,
  state: TowerRun['state'],
  linked?: TowerTicket,
): TowerRun => ({
  session: {
    session_id: id,
    title,
    description: '',
    status: state === 'working' ? 'working' : 'done',
    kind: 'user',
    created_at: ago(120),
    updated_at: ago(41),
    message_count: 6,
    metadata: linked ? { kanban_ticket: linked.key } : {},
  },
  status: {
    session_id: id,
    status: state === 'working' ? 'running' : 'completed',
    children: [],
    expects_wake: state === 'stalled',
    open_asks: state === 'waiting' ? 1 : 0,
  },
  descendantSessionIds: [],
  metrics: {
    complete: true,
    totals: {
      sessions: linked === r1 ? 6 : 1,
      input_tokens: linked === r1 ? 17000000 : 400000,
      output_tokens: linked === r1 ? 1600000 : 22000,
    },
  },
  card: linked,
  subtasks: linked === r1 ? tasks : [],
  progress:
    linked === r1
      ? [
          {
            releaseId: 'r1',
            releaseName: 'R1',
            done: 11,
            inProgress: 2,
            total: 14,
          },
          {
            releaseId: 'r11',
            releaseName: 'R1.1',
            done: 15,
            inProgress: 1,
            total: 16,
          },
          {
            releaseId: 'r12',
            releaseName: 'R1.2',
            done: 7,
            inProgress: 2,
            total: 10,
          },
        ]
      : linked === flaky
        ? [
            {
              releaseId: 'nov',
              releaseName: 'Nov fixes',
              done: 0,
              inProgress: 1,
              total: 2,
            },
          ]
        : [],
  needsYou: items.filter((item) => item.sessionId === id),
  lastProgress: ago(state === 'stalled' ? 46 : 18),
  state,
  lastAssistant:
    state === 'working'
      ? 'Verifying its last cards; the verify checks are running.'
      : state === 'waiting'
        ? 'Asks whether to accept semicolon-separated files.'
        : state === 'stalled'
          ? 'Its turn ended with a wake armed, and nothing it observes has fired.'
          : 'Opened its pull request and moved the card to In review.',
  profile: linked ? 'deepwork' : 'default',
  startedBy: linked ? 'board dispatch' : 'chat',
})
export const towerFixture: TowerData = {
  projects: [
    {
      id: 'assistente',
      name: 'Assistente',
      description: '',
      repos: ['iii-hq/workers'],
      total: 7,
      done: 2,
      open_questions: 3,
      counts: {},
    },
    {
      id: 'empty',
      name: 'Empty project',
      description: '',
      repos: [],
      total: 0,
      done: 0,
      open_questions: 0,
      counts: {},
    },
  ],
  board: {
    columns: ['backlog', 'in_progress', 'in_review', 'todo', 'done'].map(
      (id) => ({
        id,
        label:
          {
            backlog: 'Backlog',
            in_progress: 'In progress',
            in_review: 'In review',
            todo: 'To do',
            done: 'Done',
          }[id] ?? id,
        tickets: [r1, flaky, search, security, ...tasks].filter(
          (ticket) => ticket.status === id,
        ),
        total: [r1, flaky, search, security, ...tasks].filter(
          (ticket) => ticket.status === id,
        ).length,
      }),
    ),
    doneStatus: 'done',
    idPrefix: 'KAN',
  },
  tickets: [r1, flaky, search, security, ...tasks],
  releases: [
    {
      id: 'nov',
      name: 'Workers · Nov wave',
      status: 'released',
      total: 9,
      done: 9,
      open_questions: 0,
      counts: {},
      projects: [],
    },
    {
      id: 'r1',
      name: 'Assistente · R1',
      status: 'active',
      total: 14,
      done: 11,
      open_questions: 2,
      counts: {},
      projects: [],
    },
    {
      id: 'r11',
      name: 'Assistente · R1.1',
      status: 'active',
      total: 16,
      done: 15,
      open_questions: 1,
      counts: {},
      projects: [],
    },
    {
      id: 'r12',
      name: 'Assistente · R1.2',
      status: 'active',
      total: 10,
      done: 7,
      open_questions: 0,
      counts: {},
      projects: [],
    },
    {
      id: 'r2',
      name: 'Assistente · R2',
      status: 'planned',
      target_at: iso(-1440),
      total: 6,
      done: 0,
      open_questions: 1,
      counts: {},
      projects: [],
    },
  ],
  runs: [
    run('dispatch-r1', 'deepwork: Assistente · R1', 'working', r1),
    run('dispatch-flaky', 'deepwork: flaky tests', 'stalled', flaky),
    run('import-tool', 'Import tool: CSV dialects', 'waiting'),
    run('dispatch-search', search.title, 'done', search),
  ],
  items,
  prs: [
    {
      repo: 'iii-hq/iii-cloud',
      number: 451,
      title: 'Assistente R1: alerts that reach people',
      url: 'https://github.com/iii-hq/iii-cloud/pull/451',
      headRefName: 'feat/KAN-616',
      isDraft: true,
      createdAt: iso(35),
      updatedAt: iso(12),
      checks: 'passing',
      failing: 0,
      card: r1,
      waitReason: 'Draft',
    },
    {
      repo: 'iii-hq/workers',
      number: 1834,
      title: search.title,
      url: 'https://github.com/iii-hq/workers/pull/1834',
      headRefName: 'feat/KAN-711',
      isDraft: false,
      createdAt: iso(120),
      updatedAt: iso(35),
      checks: 'passing',
      failing: 0,
      card: search,
      waitReason: 'Review requested',
    },
    {
      repo: 'iii-hq/workers',
      number: 1830,
      title: 'deepwork: wait instead of blocking',
      url: 'https://github.com/iii-hq/workers/pull/1830',
      headRefName: 'fix/deepwork',
      isDraft: false,
      createdAt: iso(300),
      updatedAt: iso(60),
      checks: 'failing',
      failing: 1,
      waitReason: 'CI failing',
    },
  ],
  repositories: ['iii-hq/workers', 'iii-hq/iii-cloud'],
  loading: { board: false, sessions: false, needsYou: false, prs: false },
  errors: { board: null, sessions: null, needsYou: null, prs: null },
  refresh: () => {},
}
