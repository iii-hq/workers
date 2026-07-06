import type { FunctionCallMessage } from '@/types/chat'
import { wrapHarness } from './sandbox-fixtures'

const now = Date.now()
/** Fixed mtime so stories render stable dates (2026-03-14T00:53:20Z). */
const MTIME = 1773456800

function byteLen(text: string): number {
  return new TextEncoder().encode(text).length
}

/** Render the wire's `numbered: true` `N→` prefixes for a 1-based window. */
function numberWindow(text: string, from: number, to: number): string {
  const body = text
    .split('\n')
    .slice(from - 1, to)
    .map((line, i) => `${from + i}→${line}`)
    .join('\n')
  return `${body}\n`
}

/** Minimal demo worker — mirrors the quick-start in workers/iii/skills/SKILL.md. */
const DEMO_WORKER_TS = `import { registerWorker } from 'iii-sdk'

const iii = registerWorker(process.env.III_ENGINE_URL!, { workerName: 'demo' })

iii.registerFunction(
  'demo::add',
  async (payload: { a: number; b: number }) => {
    return { c: payload.a + payload.b }
  },
  {
    description: 'Add two numbers.',
    request_format: {
      type: 'object',
      properties: {
        a: { type: 'number' },
        b: { type: 'number' },
      },
      required: ['a', 'b'],
    },
    response_format: {
      type: 'object',
      properties: { c: { type: 'number' } },
      required: ['c'],
    },
  },
)
`

/** Substantial skill excerpt — enough lines for markdown + TS fences. 42 lines. */
const III_SKILL_MD = `---
name: iii
description: >-
  WebSocket-routed worker mesh — the engine's Function/Trigger/Worker model and
  the iii-sdk surface for authoring them.
---

# iii

iii is a WebSocket-routed worker mesh. One engine process (default port \`49134\`)
holds a live registry of every connected worker, every function those workers
expose, and every trigger bound to them. Workers are independent OS processes
that open a WebSocket to the engine and register **Functions** (\`service::name\`
handlers) and **Triggers** (the events that invoke those Functions).

\`\`\`ts
import { registerWorker } from 'iii-sdk'

const iii = registerWorker(process.env.III_ENGINE_URL!, { workerName: 'demo' })

iii.registerFunction('demo::add', async (payload: { a: number; b: number }) => {
  return { c: payload.a + payload.b }
})
\`\`\`

## The four primitives

| Primitive | What it is | Owned by |
|---|---|---|
| Engine | One coordinator process. Routes every invocation. | The operator |
| Worker | A process that opens a WebSocket to the engine. | Anyone who writes one |
| Function | A named handler inside a worker, id \`service::name\`. | The registering worker |
| Trigger | A \`(type, config, function_id)\` triple. | A worker + a caller |

## Need a capability? Discover before you build

1. **Look at what is already registered** — \`engine::functions::list\`.
2. **Search the public registry** — \`directory::registry::workers::list\`.
3. **Build a worker** — only when steps 1 and 2 come up empty.

> Discover in order. Don't jump to a worker you remember; the registry may hold
> a better fit.
`

/** Replacement for the discovery section (SKILL.md lines 35–42). */
const DISCOVERY_SECTION_V2 = `## Need a capability? Discover before you build — in this order

The most common harness mistake is reimplementing something that already exists.
Work the steps in order; stop at the first that satisfies the need.

**1. Look at what is already registered in the engine.**

\`\`\`jsonc
// engine::functions::list   — every function on this engine.
//   Filter with { prefix: 'svc::' } or { search: 'resize' }.
// engine::workers::list      — every connected worker.
\`\`\`

If a registered function fits, just call it:
\`iii.trigger({ function_id, payload })\`.

**2. Search the public registry** via \`directory::registry::workers::list\`.

**3. Build a worker** only when steps 1 and 2 both come up empty.

## Trust runtime probes over introspection

Probe with \`iii.trigger(...)\` before re-registering: a successful call proves
the registration is live regardless of what \`engine::*::list\` reported.
`

const DISCOVERY_V2_LINES = DISCOVERY_SECTION_V2.replace(/\n$/, '').split('\n')

/* The update_lines op replaces SKILL.md lines 35..42 (the old discovery
 * section through EOF). The post-apply echo region is the new section plus
 * 2 lines of leading context (the table row + blank at 33–34); there is no
 * trailing context at EOF. update_file.rs::build_line_echo keeps the first
 * and last 8 lines and elides the middle. */
const DISCOVERY_ECHO_REGION = [
  '| Trigger | A `(type, config, function_id)` triple. | A worker + a caller |',
  '',
  ...DISCOVERY_V2_LINES,
]
const DISCOVERY_ECHO_LINES = [
  ...DISCOVERY_ECHO_REGION.slice(0, 8),
  ...DISCOVERY_ECHO_REGION.slice(-8),
]
const DISCOVERY_ECHO_ELIDED = DISCOVERY_ECHO_REGION.length - 16

const DEMO_PACKAGE_JSON = `{
  "name": "demo-worker",
  "private": true,
  "type": "module",
  "dependencies": {
    "iii-sdk": "^0.12.0"
  },
  "scripts": {
    "start": "node --import tsx src/index.ts"
  }
}
`

/** The one C211 wording (error.rs::C211_SUFFIX) — message carries no code prefix. */
function c211(path: string) {
  return {
    code: 'C211',
    message: `${path}: not found or not accessible. Verify the path with coder::list-folder or coder::tree.`,
  }
}

function base(
  id: string,
  functionId: string,
  input: unknown,
  output?: unknown,
  extra?: Partial<FunctionCallMessage>,
): FunctionCallMessage {
  return {
    id,
    role: 'function-call',
    functionId,
    input,
    output,
    durationMs: 180,
    createdAt: now,
    ...extra,
  }
}

/* ---------------- create-file ---------------- */

/** Hero fixture: new TypeScript worker with JSON Schema metadata — rich TS body. */
export const coderCreateSingle = base(
  'coder-create-1',
  'coder::create-file',
  {
    files: [
      {
        path: 'workers/demo/src/index.ts',
        content: DEMO_WORKER_TS,
        mode: '0644',
        parents: true,
        overwrite: false,
      },
    ],
  },
  {
    results: [
      {
        path: '/work/workers/demo/src/index.ts',
        success: true,
        bytes_written: byteLen(DEMO_WORKER_TS),
      },
    ],
  },
)

/** Large markdown skill doc — exercises markdown + fenced-code highlighting. */
export const coderCreateSkillDoc = base(
  'coder-create-skill',
  'coder::create-file',
  {
    files: [
      {
        path: 'workers/iii/skills/SKILL.md',
        content: III_SKILL_MD,
        mode: '0644',
        parents: true,
      },
    ],
  },
  {
    results: [
      {
        path: '/work/workers/iii/skills/SKILL.md',
        success: true,
        bytes_written: byteLen(III_SKILL_MD),
      },
    ],
  },
)

/** Multi-file scaffold — two bodies stacked in one call. */
export const coderCreateMultiScaffold = base(
  'coder-create-scaffold',
  'coder::create-file',
  {
    files: [
      {
        path: 'workers/demo/src/index.ts',
        content: DEMO_WORKER_TS,
        parents: true,
      },
      {
        path: 'workers/demo/package.json',
        content: DEMO_PACKAGE_JSON,
        parents: true,
      },
    ],
  },
  wrapHarness({
    results: [
      {
        path: '/work/workers/demo/src/index.ts',
        success: true,
        bytes_written: byteLen(DEMO_WORKER_TS),
      },
      {
        path: '/work/workers/demo/package.json',
        success: true,
        bytes_written: byteLen(DEMO_PACKAGE_JSON),
      },
    ],
  }),
)

/** Per-entry WireError: .env matches non_accessible_globs → C211 for that
 *  entry only; the path echoes the caller's input verbatim (resolution failed). */
export const coderCreateMultiPartialFail = base(
  'coder-create-multi',
  'coder::create-file',
  {
    files: [
      {
        path: '.env',
        content: 'III_ENGINE_URL=ws://127.0.0.1:49134\nSECRET=1\n',
        overwrite: true,
      },
      {
        path: 'workers/demo/.gitignore',
        content: 'node_modules/\ndist/\n',
      },
    ],
  },
  {
    results: [
      {
        path: '.env',
        success: false,
        bytes_written: 0,
        error: c211('.env'),
      },
      {
        path: '/work/workers/demo/.gitignore',
        success: true,
        bytes_written: byteLen('node_modules/\ndist/\n'),
      },
    ],
  },
)

export const coderCreatePending = base(
  'coder-create-pending',
  'coder::create-file',
  {
    files: [
      {
        path: 'workers/demo/src/index.ts',
        content: DEMO_WORKER_TS,
        parents: true,
      },
    ],
  },
  undefined,
  { pendingApproval: true },
)

export const coderCreateRunning = base(
  'coder-create-running',
  'coder::create-file',
  {
    files: [
      {
        path: 'workers/iii/skills/SKILL.md',
        content: III_SKILL_MD,
        parents: true,
      },
    ],
  },
  undefined,
  { running: true },
)

/* ---------------- update-file ---------------- */

/** Insert + update_lines + replace in one file; per-op post-apply echoes. */
export const coderUpdateMixedOps = base(
  'coder-update-ops',
  'coder::update-file',
  {
    files: [
      {
        path: 'workers/demo/src/index.ts',
        ops: [
          {
            op: 'insert',
            at_line: 1,
            content: "import type { Logger } from 'iii-sdk'\n",
          },
          {
            op: 'update_lines',
            from_line: 3,
            to_line: 3,
            content:
              "const iii = registerWorker(process.env.III_ENGINE_URL!, {\n  workerName: 'demo',\n  invocationTimeoutMs: 30_000,\n})\n",
          },
          {
            op: 'replace',
            pattern: 'demo::add',
            replacement: 'demo::sum',
            expect_matches: 1,
          },
        ],
      },
    ],
  },
  {
    results: [
      {
        path: '/work/workers/demo/src/index.ts',
        success: true,
        applied: 3,
        new_line_count: 30,
        echoes: [
          {
            op_index: 0,
            from_line: 1,
            lines: [
              "import type { Logger } from 'iii-sdk'",
              "import { registerWorker } from 'iii-sdk'",
              '',
            ],
          },
          {
            op_index: 1,
            from_line: 2,
            lines: [
              "import { registerWorker } from 'iii-sdk'",
              '',
              'const iii = registerWorker(process.env.III_ENGINE_URL!, {',
              "  workerName: 'demo',",
              '  invocationTimeoutMs: 30_000,',
              '})',
              '',
              'iii.registerFunction(',
            ],
          },
          {
            op_index: 2,
            from_line: 10,
            lines: ["  'demo::sum',"],
            total_replacements: 1,
          },
        ],
        echoes_truncated: false,
      },
    ],
  },
)

/** Markdown skill — section rewrite whose line-op echo elides the middle. */
export const coderUpdateSkillDiscovery = base(
  'coder-update-skill',
  'coder::update-file',
  {
    files: [
      {
        path: 'workers/iii/skills/SKILL.md',
        ops: [
          {
            op: 'update_lines',
            from_line: 35,
            to_line: 42,
            content: DISCOVERY_SECTION_V2,
          },
        ],
      },
    ],
  },
  wrapHarness({
    results: [
      {
        path: '/work/workers/iii/skills/SKILL.md',
        success: true,
        applied: 1,
        new_line_count: 34 + DISCOVERY_V2_LINES.length,
        echoes: [
          {
            op_index: 0,
            from_line: 33,
            lines: DISCOVERY_ECHO_LINES,
            elided: DISCOVERY_ECHO_ELIDED,
          },
        ],
        echoes_truncated: false,
      },
    ],
  }),
)

/** Replace-site echoes: a multi-line region (first+last line, inner elided)
 *  and a bulk rename whose 12 matches exceed the 5-site echo cap. */
export const coderUpdateReplaceSites = base(
  'coder-update-sites',
  'coder::update-file',
  {
    files: [
      {
        path: 'workers/demo/src/adapters.ts',
        ops: [
          {
            op: 'replace',
            pattern: '// BEGIN legacy exports.*?// END legacy exports',
            replacement:
              "// BEGIN adapter exports (generated)\nexport { LibkrunAdapter } from './libkrun'\nexport type { AdapterBootArgs } from './types'\n// END adapter exports (generated)",
            dot_matches_newline: true,
            expect_matches: 1,
          },
        ],
      },
      {
        path: 'workers/demo/src/events.ts',
        ops: [
          {
            op: 'replace',
            pattern: 'emitLegacyEvent',
            replacement: 'emitEvent',
          },
        ],
      },
    ],
  },
  {
    results: [
      {
        path: '/work/workers/demo/src/adapters.ts',
        success: true,
        applied: 1,
        new_line_count: 31,
        echoes: [
          {
            op_index: 0,
            from_line: 12,
            lines: [
              '// BEGIN adapter exports (generated)',
              '// END adapter exports (generated)',
            ],
            elided: 2,
            total_replacements: 1,
          },
        ],
        echoes_truncated: false,
      },
      {
        path: '/work/workers/demo/src/events.ts',
        success: true,
        applied: 1,
        new_line_count: 188,
        echoes: [18, 44, 71, 102, 130].map((line) => ({
          op_index: 0,
          from_line: line,
          lines: [`  emitEvent('boot', { vmId })`],
          total_replacements: 12,
        })),
        echoes_truncated: false,
      },
    ],
  },
)

/** Per-entry WireError on one file; the other applies and echoes normally. */
export const coderUpdatePartialFail = base(
  'coder-update-fail',
  'coder::update-file',
  {
    files: [
      {
        path: '.env',
        ops: [{ op: 'replace', pattern: 'SECRET', replacement: 'REDACTED' }],
      },
      {
        path: 'workers/iii/skills/SKILL.md',
        ops: [
          {
            op: 'insert',
            at_line: 1,
            content: '<!-- generated by harness -->\n',
          },
        ],
      },
    ],
  },
  wrapHarness({
    results: [
      {
        path: '.env',
        success: false,
        applied: 0,
        new_line_count: 0,
        echoes: [],
        echoes_truncated: false,
        error: c211('.env'),
      },
      {
        path: '/work/workers/iii/skills/SKILL.md',
        success: true,
        applied: 1,
        new_line_count: 43,
        echoes: [
          {
            op_index: 0,
            from_line: 1,
            lines: ['<!-- generated by harness -->', '---', 'name: iii'],
          },
        ],
        echoes_truncated: false,
      },
    ],
  }),
)

export const coderUpdatePending = base(
  'coder-update-pending',
  'coder::update-file',
  {
    files: [
      {
        path: 'workers/demo/src/index.ts',
        ops: [
          {
            op: 'insert',
            at_line: 1,
            content: '// TODO: wire Logger from iii-sdk\n',
          },
        ],
      },
    ],
  },
  undefined,
  { pendingApproval: true },
)

/* ---------------- delete-file ---------------- */

export const coderDeleteRecursive = base(
  'coder-delete-rec',
  'coder::delete-file',
  { paths: ['workers/demo/dist/', 'workers/demo/.turbo/'], recursive: true },
  {
    results: [
      { path: '/work/workers/demo/dist', success: true, removed: true },
      { path: '/work/workers/demo/.turbo', success: true, removed: true },
    ],
  },
)

/** success + !removed = idempotent "already absent". */
export const coderDeleteIdempotent = base(
  'coder-delete-miss',
  'coder::delete-file',
  { paths: ['workers/demo/node_modules/.cache/foo'] },
  {
    results: [
      {
        path: 'workers/demo/node_modules/.cache/foo',
        success: true,
        removed: false,
      },
    ],
  },
)

export const coderDeleteRunning = base(
  'coder-delete-running',
  'coder::delete-file',
  { paths: ['workers/demo/tmp/scratch.ts'], recursive: false },
  undefined,
  { running: true },
)

/* ---------------- move ---------------- */

/** Rename + cross-root move + no-op self-move (success + !moved = unchanged). */
export const coderMoveBatch = base(
  'coder-move-batch',
  'coder::move',
  {
    files: [
      { from: 'workers/demo/src/index.ts', to: 'workers/demo/src/main.ts' },
      {
        from: 'workers/demo/build/output.tar.gz',
        to: '/tmp/coder-cache/output.tar.gz',
        overwrite: true,
      },
      { from: 'workers/demo/notes.md', to: './workers/demo/notes.md' },
    ],
  },
  wrapHarness({
    results: [
      {
        from: '/work/workers/demo/src/index.ts',
        to: '/work/workers/demo/src/main.ts',
        success: true,
        moved: true,
      },
      {
        from: '/work/workers/demo/build/output.tar.gz',
        to: '/tmp/coder-cache/output.tar.gz',
        success: true,
        moved: true,
      },
      {
        from: '/work/workers/demo/notes.md',
        to: '/work/workers/demo/notes.md',
        success: true,
        moved: false,
      },
    ],
  }),
)

/** Per-entry C217: destination exists and overwrite was not passed. */
export const coderMovePartialFail = base(
  'coder-move-fail',
  'coder::move',
  {
    files: [
      { from: 'workers/demo/src/index.ts', to: 'workers/demo/src/main.ts' },
      { from: 'workers/demo/README.md', to: 'workers/demo/docs/README.md' },
    ],
  },
  {
    results: [
      {
        from: '/work/workers/demo/src/index.ts',
        to: '/work/workers/demo/src/main.ts',
        success: true,
        moved: true,
      },
      {
        from: '/work/workers/demo/README.md',
        to: '/work/workers/demo/docs/README.md',
        success: false,
        moved: false,
        error: {
          code: 'C217',
          message:
            '/work/workers/demo/docs/README.md already exists; pass overwrite=true to replace',
        },
      },
    ],
  },
)

export const coderMovePending = base(
  'coder-move-pending',
  'coder::move',
  {
    files: [
      { from: 'workers/demo/src/index.ts', to: 'workers/demo/src/main.ts' },
    ],
  },
  undefined,
  { pendingApproval: true },
)

/* ---------------- read-file ---------------- */

/** Single-path full read — scalar response fields, fully traversed. */
export const coderReadSingle = base(
  'coder-read-single',
  'coder::read-file',
  { path: 'workers/demo/src/index.ts' },
  {
    path: '/work/workers/demo/src/index.ts',
    content: DEMO_WORKER_TS,
    is_utf8: true,
    lines_returned: 26,
    total_lines: 26,
    more_lines: false,
    size: byteLen(DEMO_WORKER_TS),
    mode: 0o644,
    mtime: MTIME,
  },
)

/** Numbered window — `N→` prefixes from the wire, absent total_lines
 *  (the stream never reached EOF), more_lines feeding the next-window hint. */
export const coderReadWindowNumbered = base(
  'coder-read-window',
  'coder::read-file',
  {
    path: 'workers/iii/skills/SKILL.md',
    line_from: 26,
    line_to: 33,
    numbered: true,
  },
  {
    path: '/work/workers/iii/skills/SKILL.md',
    content: numberWindow(III_SKILL_MD, 26, 33),
    is_utf8: true,
    lines_returned: 8,
    total_lines: null,
    more_lines: true,
    size: byteLen(III_SKILL_MD),
    mode: 0o644,
    mtime: MTIME,
  },
)

/** Stat probe on an over-cap file: size/mode/mtime populate, but
 *  total_lines/is_utf8 stay null (file exceeds max_read_bytes) — a SUCCESS. */
export const coderReadStat = base(
  'coder-read-stat',
  'coder::read-file',
  { path: 'logs/engine.jsonl', stat: true },
  {
    path: '/work/logs/engine.jsonl',
    content: null,
    is_utf8: null,
    lines_returned: 0,
    total_lines: null,
    more_lines: false,
    size: 18_874_368,
    mode: 0o644,
    mtime: MTIME,
  },
)

/** Batch read: bare-string and object targets, plus a per-entry C211. */
export const coderReadBatch = base(
  'coder-read-batch',
  'coder::read-file',
  {
    paths: [
      'workers/demo/package.json',
      {
        path: 'workers/iii/skills/SKILL.md',
        line_from: 1,
        line_to: 6,
        numbered: true,
      },
      '.env',
    ],
  },
  wrapHarness({
    results: [
      {
        path: '/work/workers/demo/package.json',
        success: true,
        content: DEMO_PACKAGE_JSON,
        is_utf8: true,
        lines_returned: 11,
        total_lines: 11,
        more_lines: false,
        size: byteLen(DEMO_PACKAGE_JSON),
        mode: 0o644,
        mtime: MTIME,
      },
      {
        path: '/work/workers/iii/skills/SKILL.md',
        success: true,
        content: numberWindow(III_SKILL_MD, 1, 6),
        is_utf8: true,
        lines_returned: 6,
        total_lines: null,
        more_lines: true,
        size: byteLen(III_SKILL_MD),
        mode: 0o644,
        mtime: MTIME,
      },
      { path: '.env', success: false, error: c211('.env') },
    ],
  }),
)

/* ---------------- search ---------------- */

/** Content matches with before/after context — straight-to-update-file flow. */
export const coderSearchContext = base(
  'coder-search-ctx',
  'coder::search',
  {
    query: 'registerFunction',
    path: 'workers',
    include_globs: ['**/*.ts'],
    context_lines_before: 1,
    context_lines_after: 2,
    search_paths: false,
  },
  {
    content_matches: [
      {
        path: '/work/workers/demo/src/index.ts',
        line: 5,
        column: 5,
        text: 'iii.registerFunction(',
        before: [''],
        after: [
          "  'demo::add',",
          '  async (payload: { a: number; b: number }) => {',
        ],
      },
      {
        path: '/work/workers/todo/src/index.ts',
        line: 12,
        column: 5,
        text: "iii.registerFunction('todo::add', async (payload: AddTodo) => {",
        before: ['// register the public surface'],
        after: ['  const todo = await store.add(payload)', '  return { todo }'],
      },
    ],
    path_matches: [],
    truncated: false,
  },
)

/** Budget-truncated search — `truncated: true` means refine, don't paginate. */
export const coderSearchTruncated = base(
  'coder-search-trunc',
  'coder::search',
  { query: 'TODO', ignore_case: true, use_default_excludes: false },
  wrapHarness({
    content_matches: [
      {
        path: '/work/workers/demo/src/index.ts',
        line: 1,
        column: 4,
        text: '// TODO: wire Logger from iii-sdk',
      },
      {
        path: '/work/workers/todo/src/store.ts',
        line: 7,
        column: 27,
        text: 'export async function add(todo: AddTodo): Promise<Todo> {',
      },
      {
        path: '/work/workers/todo/node_modules/iii-sdk/dist/index.js',
        line: 1402,
        column: 19,
        text: '/* eslint-disable todo-comments */',
      },
    ],
    path_matches: [
      { path: '/work/workers/todo' },
      { path: '/work/workers/todo/TODO.md' },
    ],
    truncated: true,
  }),
)

/* ---------------- tree ---------------- */

/** Snapshot with all three truncation stubs (default_exclude, max_depth,
 *  per_folder_limit + total) and a non_accessible leaf. Hints verbatim from
 *  tree.rs. Every node carries `non_accessible` explicitly — the golden
 *  requires it (tree.rs's omit-when-false serde skip is a worker-side
 *  serializer/golden split; the parser's `.default(false)` covers the live
 *  wire and is unit-tested in parsers.test.ts). */
export const coderTreeSnapshot = base(
  'coder-tree-snap',
  'coder::tree',
  { path: 'workers/demo', max_depth: 2, per_folder_limit: 5 },
  {
    path: '/work/workers/demo',
    root: {
      name: 'demo',
      kind: 'dir',
      size: 288,
      mtime: MTIME,
      non_accessible: false,
      children: [
        {
          name: '.env',
          kind: 'file',
          size: 64,
          mtime: MTIME,
          non_accessible: true,
        },
        {
          name: 'fixtures',
          kind: 'dir',
          size: 4096,
          mtime: MTIME,
          non_accessible: false,
          children: [
            {
              name: 'add.json',
              kind: 'file',
              size: 212,
              mtime: MTIME,
              non_accessible: false,
            },
            {
              name: 'boot.json',
              kind: 'file',
              size: 198,
              mtime: MTIME,
              non_accessible: false,
            },
            {
              name: 'echo.json',
              kind: 'file',
              size: 240,
              mtime: MTIME,
              non_accessible: false,
            },
            {
              name: 'list.json',
              kind: 'file',
              size: 187,
              mtime: MTIME,
              non_accessible: false,
            },
            {
              name: 'sum.json',
              kind: 'file',
              size: 224,
              mtime: MTIME,
              non_accessible: false,
            },
          ],
          truncated: {
            reason: 'per_folder_limit',
            shown: 5,
            total: 48,
            hint: 'use coder::list-folder for paginated access to all entries',
          },
        },
        {
          name: 'node_modules',
          kind: 'dir',
          size: 4096,
          mtime: MTIME,
          non_accessible: false,
          truncated: {
            reason: 'default_exclude',
            shown: 0,
            hint: 'folder matches default_exclude_globs (coder::info lists them); re-call coder::tree with use_default_excludes: false to descend',
          },
        },
        {
          name: 'package.json',
          kind: 'file',
          size: byteLen(DEMO_PACKAGE_JSON),
          mtime: MTIME,
          non_accessible: false,
        },
        {
          name: 'src',
          kind: 'dir',
          size: 160,
          mtime: MTIME,
          non_accessible: false,
          children: [
            {
              name: 'adapters.ts',
              kind: 'file',
              size: 1184,
              mtime: MTIME,
              non_accessible: false,
            },
            {
              name: 'index.ts',
              kind: 'file',
              size: byteLen(DEMO_WORKER_TS),
              mtime: MTIME,
              non_accessible: false,
            },
            {
              name: 'lib',
              kind: 'dir',
              size: 96,
              mtime: MTIME,
              non_accessible: false,
              truncated: {
                reason: 'max_depth',
                shown: 0,
                hint: 'raise max_depth or call coder::tree with this path as the new root',
              },
            },
          ],
        },
      ],
    },
  },
)

/* ---------------- list-folder ---------------- */

/** Page 2 of a paginated listing; one non_accessible entry; has_more on. */
export const coderListFolderPage = base(
  'coder-list-page',
  'coder::list-folder',
  { path: 'workers', page: 2, page_size: 5 },
  {
    path: '/work/workers',
    entries: [
      {
        name: 'echo',
        kind: 'dir',
        size: 192,
        mtime: MTIME,
        non_accessible: false,
      },
      {
        name: 'iii',
        kind: 'dir',
        size: 256,
        mtime: MTIME,
        non_accessible: false,
      },
      {
        name: 'notes.local.md',
        kind: 'file',
        size: 1832,
        mtime: MTIME,
        non_accessible: false,
      },
      {
        name: 'secrets',
        kind: 'dir',
        size: 96,
        mtime: MTIME,
        non_accessible: true,
      },
      {
        name: 'todo',
        kind: 'dir',
        size: 224,
        mtime: MTIME,
        non_accessible: false,
      },
    ],
    page: 2,
    page_size: 5,
    total: 23,
    has_more: true,
  },
)

/* ---------------- info ---------------- */

/** Pure discovery — values mirror config.rs defaults plus a two-root jail. */
export const coderInfo = base(
  'coder-info',
  'coder::info',
  {},
  wrapHarness({
    base_paths: ['/work', '/tmp/coder-cache'],
    primary_root: '/work',
    batch_read_budget_bytes: 1_048_576,
    max_output_bytes: 131_072,
    max_read_bytes: 10_485_760,
    max_write_bytes: 10_485_760,
    default_exclude_globs: [
      '**/.git/**',
      '**/node_modules/**',
      '**/target/**',
      '**/dist/**',
      '**/.venv/**',
      '**/__pycache__/**',
    ],
    non_accessible_globs: ['**/.env', '**/.env.*', '**/secrets/**'],
    list_default_page_size: 100,
    list_max_page_size: 1000,
    search_default_max_line_bytes: 4096,
    search_default_max_matches: 1000,
    search_response_budget_bytes: 262_144,
    tree_default_depth: 4,
    tree_per_folder_limit: 50,
    version: '0.4.1',
  }),
)

/* ---------------- apply-patch ---------------- */

/** V4A patch: rename + edit, new file, deletion — one hunk per file. */
const APPLY_PATCH_V4A = [
  '*** Begin Patch',
  '*** Update File: workers/demo/src/index.ts',
  '*** Move to: workers/demo/src/main.ts',
  '@@ iii.registerFunction(',
  "-  'demo::add',",
  "+  'demo::sum',",
  '   async (payload: { a: number; b: number }) => {',
  '*** Add File: workers/demo/src/lib/log.ts',
  '+export function log(msg: string): void {',
  "+  process.stdout.write(msg + '\\n')",
  '+}',
  '*** Delete File: workers/demo/src/adapters.ts',
  '*** End Patch',
  '',
].join('\n')

/** Mixed-kind patch with per-file results, one echo, and passing checks. */
export const coderApplyPatch = base(
  'coder-apply-patch',
  'coder::apply-patch',
  { patch: APPLY_PATCH_V4A },
  wrapHarness({
    results: [
      {
        path: '/work/workers/demo/src/main.ts',
        kind: 'moved',
        new_line_count: 26,
        echo: { from_line: 6, lines: ["  'demo::sum',"] },
      },
      {
        path: '/work/workers/demo/src/lib/log.ts',
        kind: 'added',
        new_line_count: 3,
      },
      { path: '/work/workers/demo/src/adapters.ts', kind: 'deleted' },
    ],
    checks: [
      {
        command: 'pnpm exec tsc --noEmit',
        exit_code: 0,
        output: '',
        truncated: false,
      },
    ],
  }),
)

export const coderApplyPatchPending = base(
  'coder-apply-patch-pending',
  'coder::apply-patch',
  { patch: APPLY_PATCH_V4A },
  undefined,
  { pendingApproval: true },
)

/* ---------------- checks ---------------- */

/** Post-write checks in all three badge states: green exit 0, red
 *  non-zero with output, amber error (timeout) with truncated output. */
export const coderUpdateWithChecks = base(
  'coder-update-checks',
  'coder::update-file',
  {
    files: [
      {
        path: 'workers/demo/src/index.ts',
        ops: [
          {
            op: 'insert',
            at_line: 1,
            content: "import type { Logger } from 'iii-sdk'\n",
          },
        ],
      },
    ],
  },
  {
    results: [
      {
        path: '/work/workers/demo/src/index.ts',
        success: true,
        applied: 1,
        new_line_count: 27,
        echoes: [
          {
            op_index: 0,
            from_line: 1,
            lines: [
              "import type { Logger } from 'iii-sdk'",
              "import { registerWorker } from 'iii-sdk'",
              '',
            ],
          },
        ],
        echoes_truncated: false,
      },
    ],
    checks: [
      {
        command: 'pnpm exec biome check src/',
        exit_code: 0,
        output: 'Checked 14 files in 62ms. No fixes applied.\n',
        truncated: false,
      },
      {
        command: 'pnpm exec tsc --noEmit',
        exit_code: 2,
        output:
          "src/index.ts(1,15): error TS6133: 'Logger' is declared but its value is never read.\n",
        truncated: false,
      },
      {
        command: 'pnpm test',
        output: ' RUN  v3.1.4 /work/workers/demo\n',
        truncated: true,
        error: 'timed out after 120s',
      },
    ],
  },
)

/* ---------------- context ---------------- */

/** Workspace card: git repo with dirty entries + one instruction file. */
export const coderContext = base(
  'coder-context',
  'coder::context',
  {},
  wrapHarness({
    primary_root: '/work',
    base_paths: ['/work', '/tmp/coder-cache'],
    platform: { os: 'linux', arch: 'aarch64' },
    git: {
      branch: 'feat/worktrees',
      status: [' M workers/demo/src/index.ts', '?? workers/demo/src/lib/'],
      status_truncated: false,
      recent_commits: [
        '936ba3be fix(provider): surface stream-fatal error events',
        '5ddd788d chore(harness): bump to v1.1.5',
      ],
    },
    instruction_files: [
      {
        path: 'CLAUDE.md',
        content:
          '# Project notes\n\n- Use pnpm, never npm.\n- Conventional commits.\n',
        truncated: false,
      },
      {
        path: 'workers/demo/CLAUDE.md',
        content: '# demo worker\n\nRun with III_ENGINE_URL set.\n',
        truncated: true,
      },
    ],
  }),
)

/** Non-repo workspace — git absent on the wire. */
export const coderContextNoGit = base(
  'coder-context-no-git',
  'coder::context',
  {},
  {
    primary_root: '/tmp/scratch',
    base_paths: ['/tmp/scratch'],
    platform: { os: 'darwin', arch: 'arm64' },
    instruction_files: [],
  },
)

/* ---------------- worktree-add / worktree-remove ---------------- */

export const coderWorktreeAdd = base(
  'coder-worktree-add',
  'coder::worktree-add',
  { name: 'fix-timeouts' },
  wrapHarness({
    path: '/work/.worktrees/fix-timeouts',
    branch: 'worktree-fix-timeouts',
  }),
)

export const coderWorktreeRemoveClean = base(
  'coder-worktree-remove',
  'coder::worktree-remove',
  { name: 'fix-timeouts' },
  {
    removed: true,
    dirty: false,
    path: '/work/.worktrees/fix-timeouts',
    branch: 'worktree-fix-timeouts',
    branch_deleted: true,
  },
)

/** Refused: uncommitted work keeps the worktree (and its branch) alive. */
export const coderWorktreeRemoveDirty = base(
  'coder-worktree-remove-dirty',
  'coder::worktree-remove',
  { name: 'spike-quic' },
  wrapHarness({
    removed: false,
    dirty: true,
    path: '/work/.worktrees/spike-quic',
    branch: 'worktree-spike-quic',
    branch_deleted: false,
  }),
)

/* ---------------- top-level error ---------------- */

export const coderGateError = base(
  'coder-gate-err',
  'coder::create-file',
  {
    files: [
      {
        path: 'workers/demo/src/index.ts',
        content: DEMO_WORKER_TS,
      },
    ],
  },
  {
    error: {
      kind: 'function_error',
      message: 'trigger_failed: approval gate unreachable',
      details: {
        status: 'denied',
        denied_by: 'gate_unavailable',
        function_id: 'coder::create-file',
        reason: 'approval gate unreachable',
      },
      content: [{ type: 'text', text: 'denied' }],
    },
  },
)

export const coderFixtures = [
  coderCreateSingle,
  coderCreateSkillDoc,
  coderCreateMultiScaffold,
  coderCreateMultiPartialFail,
  coderCreatePending,
  coderCreateRunning,
  coderUpdateMixedOps,
  coderUpdateSkillDiscovery,
  coderUpdateReplaceSites,
  coderUpdatePartialFail,
  coderUpdatePending,
  coderDeleteRecursive,
  coderDeleteIdempotent,
  coderDeleteRunning,
  coderMoveBatch,
  coderMovePartialFail,
  coderMovePending,
  coderReadSingle,
  coderReadWindowNumbered,
  coderReadStat,
  coderReadBatch,
  coderSearchContext,
  coderSearchTruncated,
  coderTreeSnapshot,
  coderListFolderPage,
  coderInfo,
  coderApplyPatch,
  coderApplyPatchPending,
  coderUpdateWithChecks,
  coderContext,
  coderContextNoGit,
  coderWorktreeAdd,
  coderWorktreeRemoveClean,
  coderWorktreeRemoveDirty,
  coderGateError,
] as const
