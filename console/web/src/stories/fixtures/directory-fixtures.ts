import type { FunctionTriggerMessage } from '@/types/chat'
import { wrapHarness } from './sandbox-fixtures'

const now = Date.now()
const NOW_ISO = new Date(now).toISOString()
const YESTERDAY_ISO = new Date(now - 86_400_000).toISOString()
const LAST_WEEK_ISO = new Date(now - 7 * 86_400_000).toISOString()

function base(
  id: string,
  functionId: string,
  input: unknown,
  output?: unknown,
  extra?: Partial<FunctionTriggerMessage>,
): FunctionTriggerMessage {
  return {
    id,
    role: 'function-trigger',
    functionId,
    input,
    output,
    durationMs: 28,
    createdAt: now,
    ...extra,
  }
}

/* ---------------- directory::skills::list ---------------- */

export const directorySkillsListDone = base(
  'dir-skills-list',
  'directory::skills::list',
  { prefix: 'sandbox/' },
  wrapHarness({
    skills: [
      {
        id: 'sandbox/skills/sandbox/create',
        title: 'sandbox::create',
        type: 'how-to',
        function_id: 'sandbox::create',
        description:
          'Create a fresh sandbox VM. Pair with sandbox::exec to run commands inside it.',
        bytes: 1840,
        modified_at: YESTERDAY_ISO,
      },
      {
        id: 'sandbox/skills/sandbox/exec',
        title: 'sandbox::exec',
        type: 'how-to',
        function_id: 'sandbox::exec',
        description: 'Run a single command in an existing sandbox.',
        bytes: 1602,
        modified_at: YESTERDAY_ISO,
      },
      {
        id: 'sandbox/index',
        title: 'Sandbox worker',
        type: 'index',
        function_id: null,
        description:
          'Read-write VMs for the agent. Use create to provision one, exec/run to interact.',
        bytes: 920,
        modified_at: LAST_WEEK_ISO,
      },
    ],
  }),
)

export const directorySkillsListEmpty = base(
  'dir-skills-list-empty',
  'directory::skills::list',
  { search: 'no-such-skill' },
  { skills: [] },
)

export const directorySkillsListRunning = base(
  'dir-skills-list-running',
  'directory::skills::list',
  {},
  undefined,
  { running: true },
)

/* ---------------- directory::skills::get ---------------- */

export const directorySkillsGetDone = base(
  'dir-skills-get',
  'directory::skills::get',
  { id: 'sandbox/skills/sandbox/create' },
  wrapHarness({
    id: 'sandbox/skills/sandbox/create',
    title: 'sandbox::create',
    type: 'how-to',
    function_id: 'sandbox::create',
    body: `# sandbox::create

Create a fresh sandbox VM.

## Example

\`\`\`json
{
  "image": "motia/node:20",
  "cpus": 2,
  "memory_mb": 512
}
\`\`\`
`,
    modified_at: YESTERDAY_ISO,
  }),
)

/* ---------------- directory::skills::index ---------------- */

export const directorySkillsIndexDone = base(
  'dir-skills-index',
  'directory::skills::index',
  {},
  wrapHarness({
    body: `## sandbox

Read-write VMs for the agent. Use \`create\` to provision one.

[Read more](sandbox/index.md)

## todo-app

Demo todo app with create/list/get/update/delete + an HTML page.

[Read more](todo-app/index.md)
`,
    workers_count: 2,
  }),
)

/* ---------------- directory::skills::download ---------------- */

export const directorySkillsDownloadRepo = base(
  'dir-skills-download-repo',
  'directory::skills::download',
  {
    repo: 'https://github.com/iii-hq/sandbox',
    skill: 'sandbox',
    branch: 'main',
  },
  wrapHarness({
    namespace: 'sandbox',
    skills_written: [
      'sandbox/index',
      'sandbox/skills/sandbox/create',
      'sandbox/skills/sandbox/exec',
      'sandbox/skills/sandbox/stop',
    ],
    prompts_written: ['sandbox/prompts/getting-started'],
    source: {
      kind: 'repo',
      repo: 'https://github.com/iii-hq/sandbox',
      skill: 'sandbox',
      branch: 'main',
    },
  }),
)

export const directorySkillsDownloadRegistry = base(
  'dir-skills-download-reg',
  'directory::skills::download',
  { worker: 'pdfkit', version: '1.0.0' },
  {
    namespace: 'pdfkit',
    skills_written: ['pdfkit/index', 'pdfkit/skills/pdfkit/render'],
    prompts_written: [],
    source: {
      kind: 'registry',
      worker: 'pdfkit',
      spec: { kind: 'version', value: '1.0.0' },
    },
  },
)

/* ---------------- directory::prompts::list ---------------- */

export const directoryPromptsListDone = base(
  'dir-prompts-list',
  'directory::prompts::list',
  {},
  wrapHarness({
    prompts: [
      {
        name: 'sandbox/getting-started',
        description: 'Walks the agent through creating + using a sandbox VM.',
        modified_at: YESTERDAY_ISO,
      },
      {
        name: 'todo-app/quickstart',
        description: 'Set up the demo todo worker and exercise every function.',
        modified_at: LAST_WEEK_ISO,
      },
    ],
  }),
)

/* ---------------- directory::prompts::get ---------------- */

export const directoryPromptsGetDone = base(
  'dir-prompts-get',
  'directory::prompts::get',
  { name: 'sandbox/getting-started' },
  wrapHarness({
    name: 'sandbox/getting-started',
    description: 'Walks the agent through creating + using a sandbox VM.',
    body: `Hello! Here's how to spin up a sandbox:

1. Call \`sandbox::create\` to provision a VM.
2. Use \`sandbox::exec\` to run shell commands inside it.
3. When done, call \`sandbox::stop\` to release resources.
`,
    modified_at: NOW_ISO,
  }),
)

/* ---------------- directory::registry::workers::list ---------------- */

export const directoryRegistryListDone = base(
  'dir-reg-list',
  'directory::registry::workers::list',
  { search: 'pdf' },
  wrapHarness({
    workers: [
      {
        name: 'pdfkit',
        description: 'Render HTML to PDF with WeasyPrint.',
        type: 'binary',
        version: '1.0.0',
        repo: 'https://github.com/iii-hq/pdfkit',
        config: {},
        supported_targets: ['darwin-arm64', 'linux-x86_64'],
        total_downloads: 4823,
        dependencies: [],
        author: { name: 'iii-hq', verified: true },
      },
      {
        name: 'pdfsign',
        description: 'Cryptographically sign PDFs.',
        type: 'image',
        version: '0.4.2',
        image: 'ghcr.io/iii-hq/pdfsign:0.4.2',
        config: {},
        supported_targets: [],
        total_downloads: 412,
        dependencies: [{ name: 'pdfkit', version: '>=1.0.0' }],
        author: { name: 'jane', verified: false },
      },
    ],
    pagination: {
      next_cursor: 'eyJjcz0yfQ==',
      has_more: true,
      page_size: 20,
    },
  }),
)

/* ---------------- directory::registry::workers::info ---------------- */

export const directoryRegistryInfoDone = base(
  'dir-reg-info',
  'directory::registry::workers::info',
  { name: 'pdfkit', tag: 'latest' },
  wrapHarness({
    worker: {
      name: 'pdfkit',
      description: 'Render HTML to PDF with WeasyPrint.',
      type: 'binary',
      version: '1.0.0',
      repo: 'https://github.com/iii-hq/pdfkit',
      config: {},
      supported_targets: ['darwin-arm64', 'linux-x86_64'],
      total_downloads: 4823,
      dependencies: [],
      author: { name: 'iii-hq', verified: true },
    },
    readme: '# pdfkit\n\nRender HTML → PDF. Powered by WeasyPrint.\n',
    api_reference: {
      functions: [
        {
          name: 'pdfkit::render',
          description: 'Render an HTML string to PDF bytes.',
          request_schema: null,
          response_schema: null,
          metadata: null,
        },
      ],
      triggers: [],
    },
    skills_tree: {
      skills: [
        { path: 'pdfkit/index' },
        { path: 'pdfkit/skills/pdfkit/render' },
      ],
      prompts: [],
    },
  }),
)

export const directoryFixtures = [
  directorySkillsListDone,
  directorySkillsListEmpty,
  directorySkillsListRunning,
  directorySkillsGetDone,
  directorySkillsIndexDone,
  directorySkillsDownloadRepo,
  directorySkillsDownloadRegistry,
  directoryPromptsListDone,
  directoryPromptsGetDone,
  directoryRegistryListDone,
  directoryRegistryInfoDone,
] as const
