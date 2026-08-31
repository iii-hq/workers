import {
  number,
  object,
  password,
  permissionRules,
  select,
  stringList,
  text,
  toggle,
  variant,
  variantOption,
} from '../spec-helpers'
import { choice, type WorkerConfigurationSpec } from '../types'

const thinkingChoices = [
  choice('minimal', 'Minimal'),
  choice('low', 'Low'),
  choice('medium', 'Medium'),
  choice('high', 'High'),
  choice('xhigh', 'Extra high'),
]

export const agentCollectionWorkerSpecs: readonly WorkerConfigurationSpec[] = [
  {
    id: 'approval-gate',
    title: 'Approval gate',
    description: 'Default permission mode, ordered rules and filesystem re-ask safety.',
    sections: [
      {
        title: 'Default policy',
        fields: [
          select(
            'default_mode',
            'Permission mode',
            [choice('manual', 'Manual'), choice('auto', 'Automatic'), choice('full', 'Full access')],
            'Applied when a session has no stored approval settings.',
          ),
          number(
            'grant_reask_limit',
            'Filesystem re-ask limit',
            'Maximum repeated jail-scope grant prompts for one held call.',
            { min: 0 },
          ),
        ],
      },
      {
        title: 'Ordered rules',
        description: 'First match wins. Prefix a function glob with ! to deny it.',
        fields: [
          permissionRules(
            'rules',
            'Permission rules',
            'Use a shorthand glob or an advanced rule with mode and argument constraints.',
            {
              addLabel: 'Add rule',
            },
          ),
        ],
      },
    ],
    expectedFields: [
      'default_mode',
      'rules[].shorthand',
      'rules[].function',
      'rules[].action',
      'rules[].rule_id',
      'rules[].modes[]',
      'rules[].args.*',
      'grant_reask_limit',
    ],
  },
  {
    id: 'claude-code',
    title: 'Claude Code',
    description: 'Headless turn defaults, approval routing, streams and terminal experience.',
    sections: [
      {
        title: 'Turn defaults',
        fields: [
          object('defaults', 'Defaults', [
            text('model', 'Model', 'Empty uses the Claude Code default.'),
            select('permission_mode', 'Permission mode', [
              choice('default', 'Default'),
              choice('acceptEdits', 'Accept edits'),
              choice('plan', 'Plan'),
              choice('bypassPermissions', 'Bypass permissions'),
            ]),
            number('max_turns', 'Maximum turns', undefined, { min: 1 }),
            text('cwd', 'Working directory'),
            text('append_system_prompt', 'Additional system prompt'),
            stringList('allowed_tools', 'Allowed tools', 'Empty uses Claude Code’s default set.', {
              itemLabel: 'Tool',
            }),
            stringList('disallowed_tools', 'Disallowed tools', undefined, { itemLabel: 'Tool' }),
          ]),
        ],
      },
      {
        title: 'Worker runtime',
        fields: [
          toggle(
            'approval_gate',
            'Route through approval gate',
            'Send Claude Code tool calls through the approval-gate worker.',
          ),
          text('claude_executable', 'Claude executable', 'Empty resolves claude on PATH.'),
          text('events_stream', 'Events stream'),
          text('raw_events_stream', 'Raw events stream'),
          toggle('iii_context', 'Prepend iii context', undefined, { defaultValue: true }),
        ],
      },
      {
        title: 'Console terminal',
        fields: [
          object('terminal', 'Terminal', [
            text('executable', 'Terminal executable', 'Claude binary on the terminal host.'),
            stringList('args', 'Extra arguments', undefined, { itemLabel: 'Argument' }),
            text('workspace_dir', 'Workspace directory'),
            toggle('auto_install', 'Install when missing', undefined, { defaultValue: true }),
            toggle('setup_workspace', 'Keep workspace equipped', undefined, { defaultValue: true }),
          ]),
        ],
      },
    ],
    expectedFields: [
      'defaults.model',
      'defaults.permission_mode',
      'defaults.max_turns',
      'defaults.cwd',
      'defaults.append_system_prompt',
      'defaults.allowed_tools[]',
      'defaults.disallowed_tools[]',
      'approval_gate',
      'events_stream',
      'raw_events_stream',
      'iii_context',
      'claude_executable',
      'terminal.executable',
      'terminal.args[]',
      'terminal.workspace_dir',
      'terminal.auto_install',
      'terminal.setup_workspace',
    ],
  },
  {
    id: 'devin',
    title: 'Devin',
    description: 'Cloud API credentials and CLI agent runtime.',
    sections: [
      {
        title: 'Cloud API',
        fields: [
          password('api_key', 'API key', 'Personal or organization service token.'),
          text('org_id', 'Organization ID', 'Leave empty for personal v1 tokens.'),
          text('base_url', 'API base URL'),
          number('request_timeout_secs', 'Request timeout (seconds)', undefined, { min: 1 }),
        ],
      },
      {
        title: 'CLI agent',
        fields: [
          text('devin_executable', 'Devin executable', 'Empty resolves devin on PATH.'),
          stringList('cli_extra_args', 'Extra CLI arguments', 'Inserted before --print --.', { itemLabel: 'Argument' }),
          text('events_stream', 'Events stream'),
          text('raw_events_stream', 'Raw events stream'),
          toggle('iii_context', 'Prepend iii context', undefined, { defaultValue: true }),
        ],
      },
    ],
    expectedFields: [
      'api_key',
      'org_id',
      'base_url',
      'request_timeout_secs',
      'devin_executable',
      'cli_extra_args[]',
      'events_stream',
      'raw_events_stream',
      'iii_context',
    ],
  },
  {
    id: 'harness',
    title: 'Harness',
    description: 'Turn budgets, RPC limits, dispatch policy and session filesystem defaults.',
    sections: [
      {
        title: 'Turn budgets',
        fields: [
          number('default_max_turns', 'Default maximum turns', undefined, { min: 1 }),
          number('default_pending_timeout_ms', 'Pending timeout (ms)', undefined, { min: 1 }),
          number('max_depth', 'Maximum sub-agent depth', undefined, { min: 0 }),
          number('max_children', 'Maximum children per turn', undefined, { min: 0 }),
          number('max_validation_retries', 'Validation retries', undefined, { min: 0 }),
          number('max_transient_resumes', 'Transient resumes', undefined, { min: 0 }),
          number('idem_ttl_secs', 'Idempotency TTL (seconds)', undefined, { min: 1 }),
          number('stream_coalesce_ms', 'Stream coalescing (ms)', 'Zero writes every delta.', { min: 0 }),
        ],
      },
      {
        title: 'RPC and sweep',
        fields: [
          number('session_timeout_ms', 'Session RPC timeout (ms)', undefined, { min: 1 }),
          number('context_timeout_ms', 'Context RPC timeout (ms)', undefined, { min: 1 }),
          number('router_timeout_ms', 'Router timeout (ms)', undefined, { min: 1 }),
          number('dispatch_timeout_ms', 'Dispatch timeout (ms)', undefined, { min: 1 }),
          text('sweep_expression', 'Pending-call sweep schedule', 'Six-field cron expression.'),
        ],
      },
      {
        title: 'Parentless sessions',
        fields: [
          object(
            'default_functions',
            'Default function policy',
            [
              stringList('allow', 'Allow', 'Function IDs or globs available to the turn.', {
                itemLabel: 'Function glob',
              }),
              stringList('deny', 'Deny', 'Function IDs or globs removed from the allow set.', {
                itemLabel: 'Function glob',
              }),
              select('expose', 'Exposure mode', [choice('agent_trigger', 'Agent trigger'), choice('native', 'Native')]),
            ],
            'Policy used by a parentless spawn with no explicit function list. Turning it off stores null (deny all).',
            {
              optional: true,
              defaultValue: { allow: ['*'], deny: [], expose: 'agent_trigger' },
              disabledValue: null,
            },
          ),
          text(
            'default_filesystem_root',
            'Default filesystem root',
            'Unset uses the worker boot directory; “off” disables default scoping.',
            { optional: true },
          ),
        ],
      },
    ],
    expectedFields: [
      'default_max_turns',
      'default_pending_timeout_ms',
      'max_depth',
      'max_children',
      'max_validation_retries',
      'max_transient_resumes',
      'idem_ttl_secs',
      'session_timeout_ms',
      'context_timeout_ms',
      'router_timeout_ms',
      'dispatch_timeout_ms',
      'stream_coalesce_ms',
      'sweep_expression',
      'default_functions.allow[]',
      'default_functions.deny[]',
      'default_functions.expose',
      'default_filesystem_root',
    ],
  },
  {
    id: 'memory-consolidate',
    title: 'Memory consolidation',
    description: 'Scheduled consolidation, safety caps and optional model-assisted promotion.',
    sections: [
      {
        title: 'Schedule',
        fields: [
          toggle('enabled', 'Run scheduled passes', undefined, { defaultValue: true }),
          number('interval_hours', 'Interval (hours)', undefined, { min: 1 }),
          toggle('dry_run', 'Dry run', 'Plan supersedes without writing changes.'),
          stringList('banks', 'Banks', 'Empty processes every bank.', { itemLabel: 'Bank' }),
          number('max_supersedes_per_run', 'Supersedes per run', undefined, { min: 1 }),
        ],
      },
      {
        title: 'Model assistance',
        fields: [
          toggle('llm_assist_enabled', 'Enable model assistance'),
          text('llm_model', 'Judge model', 'Empty uses the first router model.'),
          number('promote_corroboration_threshold', 'Promotion threshold', 'Zero disables learned-rule promotion.', {
            min: 0,
          }),
        ],
      },
    ],
    expectedFields: [
      'enabled',
      'interval_hours',
      'dry_run',
      'banks[]',
      'max_supersedes_per_run',
      'llm_assist_enabled',
      'llm_model',
      'promote_corroboration_threshold',
    ],
  },
  {
    id: 'pi',
    title: 'Pi',
    description: 'Pi turn defaults, event streams and console terminal setup.',
    sections: [
      {
        title: 'Turn defaults',
        fields: [
          object('defaults', 'Defaults', [
            text('model', 'Model', 'Empty uses Pi’s default.'),
            select('thinking_level', 'Thinking level', [choice('off', 'Off'), ...thinkingChoices]),
            text('cwd', 'Working directory'),
            stringList('tools', 'Tools', 'Empty uses Pi’s default set.', { itemLabel: 'Tool' }),
            text('agent_dir', 'Agent definitions directory'),
          ]),
        ],
      },
      {
        title: 'Runtime',
        fields: [
          text('events_stream', 'Events stream'),
          text('raw_events_stream', 'Raw events stream'),
          toggle('iii_context', 'Prepend iii context', undefined, { defaultValue: true }),
        ],
      },
      {
        title: 'Console terminal',
        fields: [
          object('terminal', 'Terminal', [
            text('executable', 'Terminal executable'),
            stringList('args', 'Extra arguments', undefined, { itemLabel: 'Argument' }),
            text('workspace_dir', 'Workspace directory'),
            toggle('auto_install', 'Install when missing', undefined, { defaultValue: true }),
            toggle('setup_workspace', 'Keep workspace equipped', undefined, { defaultValue: true }),
            text('auth_provider', 'Fallback auth provider'),
          ]),
        ],
      },
    ],
    expectedFields: [
      'defaults.model',
      'defaults.thinking_level',
      'defaults.cwd',
      'defaults.tools[]',
      'defaults.agent_dir',
      'events_stream',
      'raw_events_stream',
      'iii_context',
      'terminal.executable',
      'terminal.args[]',
      'terminal.workspace_dir',
      'terminal.auto_install',
      'terminal.setup_workspace',
      'terminal.auth_provider',
    ],
  },
  {
    id: 'provider-xai',
    title: 'xAI provider',
    description: 'Server-side xAI Agent Tools offered by the provider worker.',
    sections: [
      {
        title: 'Agent tools',
        fields: [
          toggle('tools_enabled', 'Enable Agent Tools', 'Use the xAI Responses API for server-side tools.'),
          stringList('tool_sources', 'Tool sources', undefined, {
            itemLabel: 'Tool',
            options: [
              choice('x_search', 'X search'),
              choice('web_search', 'Web search'),
              choice('code_interpreter', 'Code interpreter'),
              choice('collections_search', 'Collections search'),
            ],
          }),
        ],
      },
    ],
    expectedFields: ['tools_enabled', 'tool_sources[]'],
  },
  {
    id: 'slack',
    title: 'Slack',
    description: 'Slack API credentials, inbound bridge behavior and Harness defaults.',
    sections: [
      {
        title: 'Credentials and ingress',
        fields: [
          password('bot_token', 'Bot token', 'Required Slack xoxb token.'),
          password('user_token', 'User token', 'Optional xoxp token for message search.', { optional: true }),
          password('app_token', 'App token', 'Optional xapp token; enables Socket Mode.', { optional: true }),
          password('signing_secret', 'Signing secret', 'Required for HTTP event ingress.', { optional: true }),
          text('public_base_url', 'Public engine URL', 'Root used for Slack event and interaction callbacks.', {
            optional: true,
          }),
        ],
      },
      {
        title: 'Access and conversations',
        fields: [
          text('default_channel', 'Default channel', 'Fallback target for proactive sends.', { optional: true }),
          stringList('allowed_channels', 'Allowed channels', 'Empty allows every channel.', {
            itemLabel: 'Channel ID',
          }),
          stringList('allowed_teams', 'Allowed teams', 'Empty allows every team.', { itemLabel: 'Team ID' }),
          toggle('require_mention', 'Require mention in channels', 'Direct messages always trigger.', {
            defaultValue: true,
          }),
          toggle('backfill_thread', 'Backfill thread context', undefined, { defaultValue: true }),
          number('backfill_max_messages', 'Maximum backfill messages', undefined, { min: 0 }),
        ],
      },
      {
        title: 'Harness defaults',
        fields: [
          object(
            'default_model',
            'Default model',
            [text('provider', 'Provider'), text('id', 'Model ID')],
            'Unset uses the first model in the router catalog.',
            { optional: true, defaultValue: { provider: '', id: '' } },
          ),
          text('system_prompt', 'System prompt', undefined, { optional: true }),
          stringList('functions_allow', 'Allowed functions', undefined, { itemLabel: 'Function glob' }),
          number('timeout_ms', 'RPC timeout (ms)', undefined, { min: 1 }),
        ],
      },
    ],
    expectedFields: [
      'bot_token',
      'user_token',
      'app_token',
      'signing_secret',
      'public_base_url',
      'default_channel',
      'allowed_channels[]',
      'allowed_teams[]',
      'require_mention',
      'backfill_thread',
      'backfill_max_messages',
      'default_model.provider',
      'default_model.id',
      'system_prompt',
      'functions_allow[]',
      'timeout_ms',
    ],
  },
  {
    id: 'telegram-bot',
    title: 'Telegram bot',
    description: 'Telegram ingress, model defaults, streaming and conversation behavior.',
    sections: [
      {
        title: 'Bot and ingress',
        fields: [
          password('bot_token', 'Bot token', 'Required Telegram Bot API token.'),
          variant('updates', 'Update adapter', [
            variantOption('polling', 'Long polling', { name: 'polling', config: { timeout_seconds: 50 } }, [
              number('config.timeout_seconds', 'Polling timeout (seconds)', undefined, { min: 1, max: 50 }),
            ]),
            variantOption('webhook', 'Webhook', { name: 'webhook', config: { base_url: '', secret: '' } }, [
              text('config.base_url', 'Public engine URL'),
              password('config.secret', 'Webhook secret', undefined, { optional: true }),
            ]),
          ]),
        ],
      },
      {
        title: 'Model and prompt',
        fields: [
          object(
            'default_model',
            'Default model',
            [text('provider', 'Provider'), text('id', 'Model ID')],
            'Unset shows the model picker on /start.',
            { optional: true, defaultValue: { provider: '', id: '' } },
          ),
          select('default_thinking_level', 'Thinking level', thinkingChoices, 'Unset uses the provider default.', {
            optional: true,
          }),
          text('system_prompt', 'System prompt', undefined, { optional: true }),
          select('system_prompt_mode', 'System prompt mode', [
            choice('override', 'Override'),
            choice('enrich', 'Enrich'),
          ]),
          select('channel_context', 'Telegram channel context', [choice('auto', 'Automatic'), choice('off', 'Off')]),
          stringList('functions_allow', 'Allowed functions', undefined, { itemLabel: 'Function glob' }),
        ],
      },
      {
        title: 'Conversation and streaming',
        fields: [
          select('verbosity', 'Transcript verbosity', [
            choice('none', 'None'),
            choice('minimal', 'Minimal'),
            choice('high', 'High'),
            choice('debug', 'Debug'),
          ]),
          select('steering_mode', 'Mid-turn messages', [
            choice('steering', 'Steer current turn'),
            choice('fifo', 'Queue in order'),
          ]),
          object('streaming', 'Streaming', [
            select('transport', 'Transport', [
              choice('auto', 'Automatic'),
              choice('draft', 'Draft'),
              choice('edit', 'Edit'),
            ]),
            number('draft_id_seed', 'Draft ID seed', undefined, { step: 1 }),
            number('draft_throttle_ms', 'Draft throttle (ms)', undefined, { min: 0 }),
            number('create_settle_ms', 'Create settle window (ms)', undefined, { min: 0 }),
          ]),
          number('timeout_ms', 'RPC timeout (ms)', undefined, { min: 1 }),
        ],
      },
    ],
    expectedFields: [
      'bot_token',
      'updates.name',
      'updates.config.timeout_seconds',
      'updates.config.base_url',
      'updates.config.secret',
      'default_model.provider',
      'default_model.id',
      'verbosity',
      'default_thinking_level',
      'streaming.transport',
      'streaming.draft_id_seed',
      'streaming.draft_throttle_ms',
      'streaming.create_settle_ms',
      'steering_mode',
      'functions_allow[]',
      'system_prompt',
      'system_prompt_mode',
      'channel_context',
      'timeout_ms',
    ],
  },
]
