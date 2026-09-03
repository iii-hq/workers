import { number, object, select, text, toggle, variant, variantOption } from '../spec-helpers'
import { choice, type WorkerConfigurationSpec } from '../types'

export const nestedWorkerSpecs: readonly WorkerConfigurationSpec[] = [
  {
    id: 'codex',
    title: 'Codex',
    description: 'Headless Codex defaults, event streams and CLI connection settings.',
    sections: [
      {
        title: 'Turn defaults',
        fields: [
          object('defaults', 'Defaults', [
            text('model', 'Model', 'Empty uses the Codex CLI default.'),
            select('sandbox_mode', 'Sandbox mode', [
              choice('read-only', 'Read only'),
              choice('workspace-write', 'Workspace write'),
              choice('danger-full-access', 'Danger full access'),
            ]),
            select('approval_policy', 'Approval policy', [
              choice('never', 'Never'),
              choice('on-request', 'On request'),
              choice('on-failure', 'On failure'),
              choice('untrusted', 'Untrusted'),
            ]),
            select('reasoning_effort', 'Reasoning effort', [
              choice('', 'Codex default'),
              choice('minimal', 'Minimal'),
              choice('low', 'Low'),
              choice('medium', 'Medium'),
              choice('high', 'High'),
              choice('xhigh', 'Extra high'),
            ]),
            text('cwd', 'Working directory', 'Empty uses the worker process directory.'),
            toggle('skip_git_repo_check', 'Run outside Git repositories', undefined, { defaultValue: true }),
          ]),
        ],
      },
      {
        title: 'Runtime',
        fields: [
          text('codex_executable', 'Codex executable', 'Empty resolves codex on PATH.'),
          text('base_url', 'API base URL', 'Empty uses the SDK default.'),
          text('events_stream', 'Events stream'),
          text('raw_events_stream', 'Raw events stream'),
          toggle('iii_context', 'Prepend iii context', 'Help Codex discover engine functions.', { defaultValue: true }),
        ],
      },
    ],
    expectedFields: [
      'defaults.model',
      'defaults.sandbox_mode',
      'defaults.approval_policy',
      'defaults.reasoning_effort',
      'defaults.cwd',
      'defaults.skip_git_repo_check',
      'events_stream',
      'raw_events_stream',
      'codex_executable',
      'base_url',
      'iii_context',
    ],
  },
  {
    id: 'grok',
    title: 'Grok',
    description: 'Headless Grok defaults, event streams and CLI path.',
    sections: [
      {
        title: 'Turn defaults',
        fields: [
          object('defaults', 'Defaults', [
            text('model', 'Model', 'Empty uses the Grok CLI default.'),
            text('cwd', 'Working directory', 'Empty uses the worker process directory.'),
            toggle('always_approve', 'Always approve', 'Automatically approve commands for headless turns.', {
              defaultValue: true,
            }),
          ]),
        ],
      },
      {
        title: 'Runtime',
        fields: [
          text('grok_executable', 'Grok executable', 'Empty resolves grok on PATH.'),
          text('events_stream', 'Events stream'),
          text('raw_events_stream', 'Raw events stream'),
          toggle('iii_context', 'Prepend iii context', undefined, { defaultValue: true }),
        ],
      },
    ],
    expectedFields: [
      'defaults.model',
      'defaults.cwd',
      'defaults.always_approve',
      'events_stream',
      'raw_events_stream',
      'grok_executable',
      'iii_context',
    ],
  },
  {
    id: 'opencode',
    title: 'OpenCode',
    description: 'OpenCode turn defaults, event streams and CLI path.',
    sections: [
      {
        title: 'Turn defaults',
        fields: [
          object('defaults', 'Defaults', [
            text('model', 'Model', 'Empty uses the OpenCode default.'),
            text('cwd', 'Working directory'),
            text('agent', 'Agent', 'Named OpenCode agent; empty uses its default.'),
          ]),
        ],
      },
      {
        title: 'Runtime',
        fields: [
          text('opencode_executable', 'OpenCode executable'),
          text('events_stream', 'Events stream'),
          text('raw_events_stream', 'Raw events stream'),
          toggle('iii_context', 'Prepend iii context', undefined, { defaultValue: true }),
        ],
      },
    ],
    expectedFields: [
      'defaults.model',
      'defaults.cwd',
      'defaults.agent',
      'events_stream',
      'raw_events_stream',
      'iii_context',
      'opencode_executable',
    ],
  },
  {
    id: 'session-manager',
    title: 'Session manager',
    description: 'Storage backend and list pagination limits for durable sessions.',
    sections: [
      {
        title: 'Storage',
        fields: [
          variant('adapter', 'Storage adapter', [
            variantOption(
              'fs',
              'Filesystem',
              { name: 'fs', config: {} },
              [text('config.data_dir', 'Data directory', 'Directory containing one JSONL file per session.')],
              'Store append-only session files locally.',
            ),
            variantOption(
              'bridge',
              'Bridge',
              { name: 'bridge', config: { url: 'ws://127.0.0.1:49134', timeout_ms: 5000 } },
              [
                text('config.url', 'Engine URL', 'WebSocket URL of the main session-manager instance.'),
                number('config.timeout_ms', 'Call timeout (ms)', undefined, { min: 1 }),
              ],
              'Delegate storage and event fan-out to another iii instance.',
            ),
          ]),
        ],
      },
      {
        title: 'Pagination',
        fields: [
          number('default_list_limit', 'Default list limit', undefined, { min: 1 }),
          number('max_list_limit', 'Maximum list limit', undefined, { min: 1 }),
        ],
      },
    ],
    expectedFields: [
      'adapter.name',
      'adapter.config.data_dir',
      'adapter.config.url',
      'adapter.config.timeout_ms',
      'default_list_limit',
      'max_list_limit',
    ],
  },
]
