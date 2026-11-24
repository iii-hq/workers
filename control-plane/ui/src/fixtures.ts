import type { ControlPlaneOverviewData as ControlPlaneData } from './useControlPlaneOverview'
import { type AskEntry, type NeedsYouItem, type ControlPlaneRelease, type ControlPlaneRun, type ControlPlaneTicket } from '../../src/types'
import type { ControlPlaneSummary } from '../../src/summary'

const now = Date.now()
const ago = (minutes: number) => now - minutes * 60_000
const iso = (minutes: number) => new Date(ago(minutes)).toISOString()

export const controlPlaneSummaryFixture: ControlPlaneSummary = {
  checked_at: iso(4),
  unchanged: false,
  brief: {
    headline: 'Assistente R1 is in its last validation round, and R2 waits on a security decision.',
    happened: 'KAN-635 passed the second branch review, and iii-hq/workers#1834 is ready for review.',
    happening: 'KAN-616 is verifying the last R1 cards.',
    remaining: 'KAN-597 holds R2 until you choose how builds are isolated.',
    refs: ['KAN-616', 'KAN-597', 'iii-hq/workers#1834'],
  },
  diary: [{ at: iso(18), text: 'KAN-635 verification passed.', refs: ['KAN-635'] }],
  now: {
    'session:dispatch-r1': {
      text: 'KAN-616 is running the verification checks.',
      refs: ['session:dispatch-r1', 'KAN-616'],
    },
  },
  pr_waits: {
    'iii-hq/iii-cloud#451': {
      text: 'Draft until KAN-616 finishes verification.',
      refs: ['iii-hq/iii-cloud#451', 'KAN-616'],
    },
  },
  needs_you: [],
}

const card = (key: string, title: string, status = 'in_progress'): ControlPlaneTicket => ({
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
const search = card('KAN-711', 'Search filters persist across reloads', 'in_review')
const security = card('KAN-597', 'Decide what the build machine may run and keep')
const tasks = [
  card('KAN-635', 'Email link names the container label', 'done'),
  card('KAN-618', 'Spec tags on every notify test', 'done'),
  card('KAN-632', 'Crashloop recovery keeps tracing', 'in_progress'),
  card('KAN-598', 'Run the build in a sandbox', 'todo'),
]
r1.pills = [{ icon: 'check', text: 'validating · round 23/30' }]
r1.open_questions = 2
const asks: AskEntry[] = [
  {
    session_id: 'dispatch-r1',
    question_id: 'security',
    turn_id: 'turn-22',
    asked_by: 'deepwork',
    questions: [
      {
        header: 'Builds',
        question: 'What may the assistant’s build machine run and keep?',
        options: [
          {
            label: 'Isolate builds, keep the keys off that machine',
            description: 'Builds run in a sandbox and the machine holds no AI key. About 40 s more per build.',
            recommended: true,
          },
          {
            label: 'Keep the keys, allow-list the network',
            description: 'Builds still reach the key, but only listed hosts.',
          },
          {
            label: 'Same machine, as today',
            description: 'No change; R2 accepts the risk.',
          },
        ],
      },
    ],
    about: {
      class: 'security',
      ticket: security.key,
      why: 'R2 starts here. Today the build runs third-party code on the machine that holds the team’s AI key and GitHub access.',
      blocks: ['KAN-598', 'KAN-632', 'KAN-635'],
    },
    asked_at: ago(120),
    state: 'open',
    on_expiry: 'block',
  },
  {
    session_id: 'dispatch-r1',
    question_id: 'verify',
    turn_id: 'turn-23',
    asked_by: 'deepwork',
    questions: [
      {
        header: 'Verify',
        question: 'Verify timed out twice in round 23',
        options: [{ label: 'Retry only the unfinished checks', recommended: true }, { label: 'Run all checks again' }],
      },
    ],
    about: {
      class: 'run_health',
      ticket: r1.key,
      why: 'Retry only the checks that did not finish.',
      blocks: ['KAN-618'],
    },
    asked_at: ago(18),
    state: 'expired',
    on_expiry: 'block',
    expires_in_min: 15,
    expires_at: ago(3),
  },
  {
    session_id: 'import-tool',
    question_id: 'csv',
    turn_id: 'turn-1',
    asked_by: 'default',
    questions: [
      {
        header: 'CSV',
        question: 'Should the import tool also accept semicolon-separated files?',
        options: [{ label: 'Keep commas only' }, { label: 'Accept semicolons too' }],
      },
    ],
    about: { class: 'scope', blocks: [] },
    asked_at: ago(9),
    state: 'open',
    on_expiry: 'block',
  },
]
const items: ControlPlaneData['items'] = [
  {
    kind: 'question',
    id: 'security',
    sessionId: 'dispatch-r1',
    questionId: 'security',
    entry: asks[0],
    currentTurnId: 'turn-23',
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
    entry: asks[1],
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
    entry: asks[2],
    questionClass: 'scope',
    title: 'Should the import tool also accept semicolon-separated files?',
    question: 'Should the import tool also accept semicolon-separated files?',
    blocks: [],
    blockedCards: 0,
    state: 'open',
    createdAt: ago(9),
  },
].map((item) => ({
  ...item,
  urgency: item.kind !== 'question' || item.questionClass === 'security' ? ('now' as const) : ('no-rush' as const),
})) as ControlPlaneData['items']
const run = (id: string, title: string, state: ControlPlaneRun['state'], linked?: ControlPlaneTicket): ControlPlaneRun => ({
  session: {
    session_id: id,
    title,
    created_at: ago(120),
    updated_at: ago(41),
  },
  status: {
    session_id: id,
    status: state === 'working' ? 'running' : 'completed',
    children: [],
    expects_wake: state === 'stalled',
    open_asks: state === 'waiting' ? 1 : 0,
  },
  descendantSessionIds: [],
  context_size: 42000,
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
const releases: ControlPlaneRelease[] = [
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
]
export const controlPlaneFixture: ControlPlaneData = {
  scope: 'all',
  as_of: iso(0),
  sourceHash: 'fixture',
  median_decision_wait_ms: 90 * 60_000,
  prs_interval_min: 5,
  availability: { github: true },
  loadMoreDecided: () => {},
  summary: {
    writing: false,
    result: controlPlaneSummaryFixture,
    valid_hash: 'fixture',
    checked_at: iso(4),
    error: null,
    unavailable: false,
    fresh: true,
    loading: false,
    refreshing: false,
    refresh: () => {},
    now,
  },
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
    columns: ['backlog', 'in_progress', 'in_review', 'todo', 'done'].map((id) => ({
      id,
      label:
        {
          backlog: 'Backlog',
          in_progress: 'In progress',
          in_review: 'In review',
          todo: 'To do',
          done: 'Done',
        }[id] ?? id,
      tickets: [r1, flaky, search, security, ...tasks].filter((ticket) => ticket.status === id),
      total: [r1, flaky, search, security, ...tasks].filter((ticket) => ticket.status === id).length,
    })),
    doneStatus: 'done',
    idPrefix: 'KAN',
  },
  tickets: [r1, flaky, search, security, ...tasks],
  releases,
  releaseCatalog: releases,
  runs: [
    run('dispatch-r1', 'deepwork: Assistente · R1', 'working', r1),
    run('dispatch-flaky', 'deepwork: flaky tests', 'stalled', flaky),
    run('import-tool', 'Import tool: CSV dialects', 'waiting'),
    run('dispatch-search', search.title, 'done', search),
  ],
  items,
  questions: [
    ...asks.map((harness) => ({
      key: `${harness.session_id}/${harness.question_id}`,
      sessionId: harness.session_id,
      questionId: harness.question_id,
      ticketId: harness.about.ticket,
      questions: harness.questions,
      about: harness.about,
      askedAt: harness.asked_at,
      state: harness.state,
      harness,
    })),
    {
      key: 'recorded-1',
      sessionId: 'dispatch-r1',
      questionId: 'recorded-1',
      ticketId: security.id,
      commentId: 'assumption-comment',
      questions: [
        {
          header: 'Cache',
          question: 'Cache artifacts between builds?',
          options: [],
        },
      ],
      about: {},
      askedAt: ago(60),
      state: 'recorded',
      answerText: 'Keep the cache isolated per project.',
      by: { channel: 'agent', user: 'deepwork' },
      recordedBy: 'deepwork',
      outcome: 'answered',
      settledAt: ago(60),
    },
    {
      key: 'decided-1',
      sessionId: 'dispatch-r1',
      questionId: 'decided-1',
      ticketId: security.id,
      questions: [
        {
          header: 'Network',
          question: 'May builds reach the public network?',
          options: [],
        },
      ],
      about: {},
      askedAt: ago(180),
      state: 'answered',
      answerText: 'Allow only the package mirrors.',
      by: { channel: 'kanban', user: 'user' },
      outcome: 'answered',
      settledAt: ago(90),
    },
    {
      key: 'gone-1',
      sessionId: 'deleted',
      questionId: 'gone-1',
      ticketId: security.id,
      questions: [
        {
          header: 'Archive',
          question: 'Keep the previous artifact?',
          options: [],
        },
      ],
      about: {},
      askedAt: ago(180),
      state: 'answered',
      answerText: 'Keep it.',
      by: { channel: 'control-plane', user: 'user' },
      outcome: 'not_delivered',
      settledAt: ago(85),
    },
  ],
  prs: [
    {
      as_of: iso(12),
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
      as_of: iso(12),
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
      as_of: iso(12),
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
  loading: {
    board: false,
    sessions: false,
    needsYou: false,
    questions: false,
    prs: false,
  },
  errors: {
    board: null,
    sessions: null,
    needsYou: null,
    questions: null,
    prs: null,
  },
  refresh: () => {},
}

