import {
  filterList,
  number,
  object,
  objectList,
  objectMap,
  password,
  select,
  stringList,
  structuredValue,
  text,
  toggle,
  variant,
  variantOption,
} from '../spec-helpers'
import { choice, type WorkerConfigurationSpec } from '../types'

export const serviceCollectionWorkerSpecs: readonly WorkerConfigurationSpec[] = [
  {
    id: 'bridge',
    title: 'Bridge',
    description: 'Remote engine connection and the functions exposed or forwarded across it.',
    sections: [
      {
        title: 'Connection',
        fields: [
          text('url', 'Remote engine URL', 'Unset uses the III_URL fallback and then the built-in address.', {
            optional: true,
          }),
        ],
      },
      {
        title: 'Remote to local',
        fields: [
          objectList(
            'expose',
            'Exposed functions',
            'Function',
            { local_function: '', remote_function: '' },
            [
              text('local_function', 'Local function'),
              text('remote_function', 'Remote name', 'Unset uses the local function name.', { optional: true }),
            ],
            'Functions registered on the remote engine and handled locally.',
            { summaryPaths: [['local_function']], addLabel: 'Expose function' },
          ),
        ],
      },
      {
        title: 'Local to remote',
        fields: [
          objectList(
            'forward',
            'Forwarded functions',
            'Function',
            { local_function: '', remote_function: '' },
            [
              text('local_function', 'Local function'),
              text('remote_function', 'Remote function'),
              number('timeout_ms', 'Timeout (ms)', undefined, { min: 1, optional: true }),
            ],
            'Local function names that proxy calls to the remote engine.',
            { summaryPaths: [['local_function'], ['remote_function']], addLabel: 'Forward function' },
          ),
        ],
      },
    ],
    expectedFields: [
      'url',
      'expose[].local_function',
      'expose[].remote_function',
      'forward[].local_function',
      'forward[].remote_function',
      'forward[].timeout_ms',
    ],
  },
  {
    id: 'email',
    title: 'Email',
    description: 'Named mail accounts and global message limits.',
    sections: [
      {
        title: 'Accounts',
        fields: [
          objectMap(
            'accounts',
            'Mail accounts',
            'Account',
            { provider: 'smtp', from: '', smtp: { host: '', port: 587, starttls: true } },
            [
              select('provider', 'Provider', [choice('smtp', 'SMTP'), choice('imap', 'IMAP')]),
              text('from', 'From address'),
              object(
                'smtp',
                'SMTP',
                [
                  text('host', 'Host'),
                  number('port', 'Port', undefined, { min: 1, max: 65535 }),
                  toggle('starttls', 'STARTTLS', undefined, { defaultValue: true }),
                  text('username', 'Username', undefined, { optional: true }),
                  password('password', 'Password', undefined, { optional: true }),
                ],
                'Outgoing server settings.',
                { optional: true, defaultValue: { host: '', port: 587, starttls: true } },
              ),
              object(
                'imap',
                'IMAP',
                [
                  text('host', 'Host'),
                  number('port', 'Port', undefined, { min: 1, max: 65535 }),
                  toggle('tls', 'TLS', undefined, { defaultValue: true }),
                  stringList('folders', 'Folders', undefined, { itemLabel: 'Folder' }),
                  text('username', 'Username', undefined, { optional: true }),
                  password('password', 'Password', undefined, { optional: true }),
                ],
                'Incoming server settings.',
                { optional: true, defaultValue: { host: '', port: 993, tls: true, folders: ['INBOX'] } },
              ),
            ],
            'Each account owns its provider, sender identity and server credentials.',
            { keyLabel: 'Account name', summaryPaths: [['from'], ['provider']] },
          ),
        ],
      },
      {
        title: 'Limits',
        fields: [
          object('limits', 'Message limits', [
            number('max_attachment_bytes', 'Maximum attachment bytes', undefined, { min: 1 }),
            number('max_recipients', 'Maximum recipients', undefined, { min: 1 }),
            number('send_timeout_ms', 'Send timeout (ms)', undefined, { min: 1 }),
            number('imap_connect_timeout_ms', 'IMAP connection timeout (ms)', undefined, { min: 1 }),
          ]),
        ],
      },
    ],
    expectedFields: [
      'accounts.*.provider',
      'accounts.*.from',
      'accounts.*.smtp.host',
      'accounts.*.smtp.port',
      'accounts.*.smtp.starttls',
      'accounts.*.smtp.username',
      'accounts.*.smtp.password',
      'accounts.*.imap.host',
      'accounts.*.imap.port',
      'accounts.*.imap.tls',
      'accounts.*.imap.folders[]',
      'accounts.*.imap.username',
      'accounts.*.imap.password',
      'limits.max_attachment_bytes',
      'limits.max_recipients',
      'limits.send_timeout_ms',
      'limits.imap_connect_timeout_ms',
    ],
  },
  {
    id: 'http',
    title: 'HTTP',
    description: 'Listener, request limits, CORS policy and global middleware.',
    sections: [
      {
        title: 'Server',
        fields: [
          text('host', 'Bind host'),
          number('port', 'Port', 'Zero asks the operating system for an available port.', { min: 0, max: 65535 }),
          number('default_timeout', 'Request timeout (ms)', undefined, { min: 1 }),
          number('concurrency_request_limit', 'Concurrent requests', undefined, { min: 1 }),
        ],
      },
      {
        title: 'Cross-origin requests',
        fields: [
          object(
            'cors',
            'CORS policy',
            [
              stringList('allowed_origins', 'Allowed origins', 'Empty allows any origin.', { itemLabel: 'Origin' }),
              stringList(
                'allowed_methods',
                'Allowed methods',
                'Empty allows any method, including extension and WebDAV methods.',
                {
                  itemLabel: 'Method',
                  placeholder: 'GET, CONNECT or PROPFIND',
                },
              ),
            ],
            'Configure an explicit policy or leave it off for the permissive fallback.',
            {
              optional: true,
              defaultValue: { allowed_origins: [], allowed_methods: [] },
            },
          ),
        ],
      },
      {
        title: 'Global middleware',
        fields: [
          objectList(
            'middleware',
            'Middleware',
            'Middleware',
            { function_id: '', phase: 'preHandler', priority: 0 },
            [
              text('function_id', 'Function ID'),
              select('phase', 'Phase', [choice('preHandler', 'Before handler')]),
              number('priority', 'Priority', 'Lower values run first.', { step: 1 }),
            ],
            'Functions executed for every route before its handler.',
            { summaryPaths: [['function_id']], addLabel: 'Add middleware' },
          ),
        ],
      },
    ],
    expectedFields: [
      'port',
      'host',
      'default_timeout',
      'cors.allowed_origins[]',
      'cors.allowed_methods[]',
      'concurrency_request_limit',
      'middleware[].function_id',
      'middleware[].phase',
      'middleware[].priority',
    ],
  },
  {
    id: 'pubsub',
    title: 'Pub/sub',
    description: 'Hot-swappable local or Redis broadcast backend.',
    sections: [
      {
        title: 'Adapter',
        fields: [
          object(
            'adapter',
            'Backend',
            [
              select('name', 'Adapter', [choice('local', 'Local'), choice('redis', 'Redis')]),
              structuredValue(
                'config',
                'Adapter settings',
                'Redis accepts redis_url. Nested objects, lists, and scalar adapter values are supported.',
                {
                  optional: true,
                  secretKeys: ['password'],
                },
              ),
            ],
            'Unset uses the local in-process adapter.',
            { optional: true, defaultValue: { name: 'local', config: {} } },
          ),
        ],
      },
    ],
    expectedFields: ['adapter.name', 'adapter.config.*'],
  },
  {
    id: 'queue',
    title: 'Queue',
    description: 'Queue transport and durable named function queues.',
    sections: [
      {
        title: 'Adapter',
        fields: [
          variant(
            'adapter',
            'Transport',
            [
              variantOption(
                'builtin',
                'Built-in',
                {
                  name: 'builtin',
                  config: {
                    store_method: 'in_memory',
                  },
                },
                [
                  select('config.store_method', 'Storage', [
                    choice('file_based', 'Durable files'),
                    choice('in_memory', 'Memory only'),
                  ]),
                  text('config.file_path', 'Data directory', 'Used by durable file storage; defaults to data/queue.', {
                    optional: true,
                  }),
                  number(
                    'config.save_interval_ms',
                    'Save interval (ms)',
                    'Accepted for compatibility and defaults to 5000; mutations are persisted immediately.',
                    { min: 0, optional: true },
                  ),
                ],
                'Runs in process. Memory is the runtime default; durable file storage is optional.',
              ),
              variantOption(
                'redis',
                'Redis',
                { name: 'redis', config: { redis_url: 'redis://localhost:6379' } },
                [text('config.redis_url', 'Redis URL', 'Redis provides pub/sub without retries or durability.')],
                'Connect to a Redis pub/sub transport.',
              ),
              variantOption(
                'rabbitmq',
                'RabbitMQ',
                {
                  name: 'rabbitmq',
                  config: {
                    amqp_url: 'amqp://localhost:5672',
                    max_attempts: 3,
                    prefetch_count: 10,
                    queue_mode: 'standard',
                  },
                },
                [
                  text('config.amqp_url', 'AMQP URL'),
                  number('config.max_attempts', 'Delivery attempts', undefined, { min: 0 }),
                  number('config.prefetch_count', 'Consumer prefetch', undefined, { min: 0 }),
                  select('config.queue_mode', 'Queue mode', [choice('standard', 'Standard'), choice('fifo', 'FIFO')]),
                  text(
                    'config.priority_field',
                    'Priority payload field',
                    'Optional payload field used to stamp message priority.',
                    { optional: true },
                  ),
                ],
                'Durable queues with retries, dead letters, FIFO and priority support.',
              ),
              variantOption(
                'in_memory',
                'Memory only (legacy)',
                { name: 'in_memory', config: {} },
                [],
                'Legacy alias for the built-in in-memory transport.',
              ),
              variantOption(
                'file_based',
                'Durable files (legacy)',
                { name: 'file_based', config: { file_path: 'data/queue', save_interval_ms: 5000 } },
                [
                  text('config.file_path', 'Data directory'),
                  number('config.save_interval_ms', 'Save interval (ms)', undefined, { min: 0 }),
                ],
                'Legacy alias for the built-in file-backed transport.',
              ),
            ],
            'Changing transport hot-swaps the adapter and restarts active consumers.',
          ),
        ],
      },
      {
        title: 'Function queues',
        fields: [
          objectMap(
            'queue_configs',
            'Named queues',
            'Queue',
            {
              max_retries: 3,
              concurrency: 10,
              timeout_ms: 1800000,
              type: 'standard',
              backoff_ms: 1000,
              poll_interval_ms: 100,
              redeliver_on_engine_restart: false,
            },
            [
              select('type', 'Scheduling mode', [choice('standard', 'Standard'), choice('fifo', 'FIFO')]),
              number('max_retries', 'Maximum retries', undefined, { min: 0 }),
              number('concurrency', 'Concurrency', undefined, { min: 1 }),
              number('timeout_ms', 'Invocation timeout (ms)', undefined, { min: 1 }),
              text('message_group_field', 'FIFO message group field', undefined, { optional: true }),
              number('backoff_ms', 'Retry backoff (ms)', undefined, { min: 0 }),
              number('poll_interval_ms', 'Poll interval (ms)', undefined, { min: 1 }),
              toggle('redeliver_on_engine_restart', 'Redeliver after engine restart'),
              number('max_priority', 'Maximum priority levels', undefined, { min: 1, optional: true }),
              text('priority_field', 'Priority payload field', undefined, { optional: true }),
            ],
            'Definitions are restored when the queue worker restarts.',
            { keyLabel: 'Queue name', summaryPaths: [['type']] },
          ),
        ],
      },
    ],
    expectedFields: [
      'adapter.name',
      'adapter.config.store_method',
      'adapter.config.file_path',
      'adapter.config.save_interval_ms',
      'adapter.config.redis_url',
      'adapter.config.amqp_url',
      'adapter.config.max_attempts',
      'adapter.config.prefetch_count',
      'adapter.config.queue_mode',
      'adapter.config.priority_field',
      'queue_configs.*.max_retries',
      'queue_configs.*.concurrency',
      'queue_configs.*.timeout_ms',
      'queue_configs.*.type',
      'queue_configs.*.message_group_field',
      'queue_configs.*.backoff_ms',
      'queue_configs.*.poll_interval_ms',
      'queue_configs.*.redeliver_on_engine_restart',
      'queue_configs.*.max_priority',
      'queue_configs.*.priority_field',
    ],
  },
  {
    id: 'rbac-proxy',
    title: 'RBAC proxy',
    description: 'Public listener, upstream engine and function exposure policy.',
    sections: [
      {
        title: 'Listener and routing',
        fields: [
          text('host', 'Bind host'),
          number('port', 'Public port', undefined, { min: 1, max: 65535 }),
          text('engine_url', 'Upstream engine URL'),
          text(
            'middleware_function_id',
            'Middleware function',
            'Route allowed non-engine calls through this function.',
            { optional: true },
          ),
          toggle(
            'expose_worker_internals',
            'Expose worker internals',
            'Include operational identity in engine worker results.',
          ),
        ],
      },
      {
        title: 'RBAC contract',
        fields: [
          object('rbac', 'Access policy', [
            text('auth_function_id', 'Authentication function', 'Unset creates a permissive default session.', {
              optional: true,
            }),
            filterList(
              'expose_functions',
              'Exposed functions',
              'A function is exposed when any wildcard or metadata filter matches.',
            ),
            text('on_function_registration_function_id', 'Function registration hook', undefined, { optional: true }),
            text('on_trigger_registration_function_id', 'Trigger registration hook', undefined, { optional: true }),
            text('on_trigger_type_registration_function_id', 'Trigger type registration hook', undefined, {
              optional: true,
            }),
          ]),
        ],
      },
    ],
    expectedFields: [
      'host',
      'port',
      'engine_url',
      'middleware_function_id',
      'expose_worker_internals',
      'rbac.auth_function_id',
      'rbac.expose_functions[].match',
      'rbac.expose_functions[].metadata.*',
      'rbac.on_function_registration_function_id',
      'rbac.on_trigger_registration_function_id',
      'rbac.on_trigger_type_registration_function_id',
    ],
  },
  {
    id: 'security-scan',
    title: 'Security scan',
    description: 'Repository catalog, scheduled scans, analysis budgets and optional archives.',
    sections: [
      {
        title: 'Repositories',
        fields: [
          objectList(
            'repositories',
            'Repositories',
            'Repository',
            { id: '', path: '' },
            [
              text('id', 'Repository ID'),
              text('path', 'Absolute local path'),
              object(
                'github',
                'GitHub mapping',
                [text('full_name', 'Repository', 'Exact owner/name.')],
                'Optional operator-verified GitHub identity.',
                { optional: true, defaultValue: { full_name: '' } },
              ),
              object(
                'schedule',
                'Scheduled scan',
                [
                  text('expression', 'UTC cron expression', 'Six or seven fields.'),
                  text('target_ref', 'Target Git ref'),
                  select('mode', 'Mode', [choice('scan', 'Scan'), choice('suggest', 'Suggest')]),
                ],
                'Leave disabled to scan only on demand.',
                { optional: true, defaultValue: { expression: '0 0 0 * * *', target_ref: 'HEAD', mode: 'scan' } },
              ),
            ],
            undefined,
            { summaryPaths: [['id'], ['path']], addLabel: 'Add repository' },
          ),
        ],
      },
      {
        title: 'Analysis',
        fields: [
          object('analysis', 'Model budget', [
            text('model', 'Model'),
            text('provider', 'Provider', undefined, { optional: true }),
            number('max_turns', 'Maximum turns', undefined, { min: 1, max: 10 }),
            number('max_output_tokens', 'Maximum output tokens', undefined, { min: 1 }),
            number('max_total_tokens', 'Maximum total tokens', undefined, { min: 1 }),
            number('max_cost_usd', 'Maximum cost (USD)', undefined, { min: 0.01, step: 0.01, optional: true }),
          ]),
        ],
      },
      {
        title: 'Archive',
        fields: [
          object(
            'archive',
            'Durable run archive',
            [
              text('bucket', 'Storage bucket'),
              text('prefix', 'Object prefix', 'Defaults to runs/.', { optional: true }),
            ],
            'Optional storage-worker copy of each JSON run record.',
            { optional: true, defaultValue: { bucket: '', prefix: 'runs/' } },
          ),
        ],
      },
    ],
    expectedFields: [
      'repositories[].id',
      'repositories[].path',
      'repositories[].github.full_name',
      'repositories[].schedule.expression',
      'repositories[].schedule.target_ref',
      'repositories[].schedule.mode',
      'analysis.model',
      'analysis.provider',
      'analysis.max_turns',
      'analysis.max_output_tokens',
      'analysis.max_total_tokens',
      'analysis.max_cost_usd',
      'archive.bucket',
      'archive.prefix',
    ],
  },
  {
    id: 'shell',
    title: 'Shell',
    description: 'Execution policy, environment, filesystem jail, code surface and turn history.',
    sections: [
      {
        title: 'Execution',
        fields: [
          number('max_timeout_ms', 'Maximum foreground timeout (ms)', undefined, { min: 1 }),
          number('max_bg_timeout_ms', 'Maximum background timeout (ms)', 'Zero leaves background jobs unbounded.', {
            min: 0,
          }),
          number('default_timeout_ms', 'Default foreground timeout (ms)', undefined, { min: 1 }),
          number('max_output_bytes', 'Captured bytes per stream', undefined, { min: 1 }),
          text('working_dir', 'Default working directory', undefined, { optional: true }),
          stringList('denylist_patterns', 'Command denylist patterns', 'Advisory regular-expression tripwires.', {
            itemLabel: 'Pattern',
          }),
          number('max_concurrent_jobs', 'Maximum background jobs', undefined, { min: 1 }),
          number('job_retention_secs', 'Finished job retention (seconds)', undefined, { min: 0 }),
        ],
      },
      {
        title: 'Environment',
        fields: [
          object('env', 'Child process environment', [
            toggle('inherit', 'Inherit the full worker environment', 'May expose worker secrets to child processes.'),
            stringList('allow', 'Forwarded variables', 'Used when full inheritance is off.', { itemLabel: 'Variable' }),
          ]),
        ],
      },
      {
        title: 'Filesystem jail',
        fields: [
          object('fs', 'Filesystem access', [
            stringList('host_roots', 'Allowed roots', 'The first entry is the primary root.', { itemLabel: 'Path' }),
            toggle(
              'allow_unjailed',
              'Allow unjailed operation',
              'Acknowledge access to the entire host when roots are empty.',
            ),
            number('max_read_bytes', 'Maximum read bytes', 'Zero is unlimited.', { min: 0 }),
            number('max_write_bytes', 'Maximum write bytes', 'Zero is unlimited.', { min: 0 }),
            stringList('denylist_paths', 'Denied path prefixes', undefined, { itemLabel: 'Path' }),
            toggle('allow_special_bits', 'Allow special mode bits', 'Permit setuid, setgid and sticky bits.'),
          ]),
        ],
      },
      {
        title: 'Sandbox target',
        fields: [
          object('sandbox', 'MicroVM backend', [
            toggle('enabled', 'Accept sandbox-targeted calls', undefined, { defaultValue: true }),
          ]),
        ],
      },
      {
        title: 'Code surface',
        fields: [
          object('code', 'coder::* settings', [
            stringList(
              'non_accessible_globs',
              'Protected globs',
              'Matching paths can be listed but not changed or read.',
              { itemLabel: 'Glob' },
            ),
            stringList('default_exclude_globs', 'Default noise excludes', 'Hidden from tree and search by default.', {
              itemLabel: 'Glob',
            }),
            number('max_read_bytes', 'Maximum file read bytes', undefined, { min: 1 }),
            number('max_write_bytes', 'Maximum file write bytes', undefined, { min: 1 }),
            number('tree_default_depth', 'Default tree depth', undefined, { min: 0 }),
            number('tree_per_folder_limit', 'Tree entries per folder', undefined, { min: 1 }),
            number('list_default_page_size', 'Default folder page size', undefined, { min: 1 }),
            number('list_max_page_size', 'Maximum folder page size', undefined, { min: 1 }),
            number('search_default_max_matches', 'Default search matches', undefined, { min: 1 }),
            number('search_default_max_line_bytes', 'Search line bytes', undefined, { min: 1 }),
            number('batch_read_budget_bytes', 'Batch read response bytes', undefined, { min: 1 }),
            number('max_output_bytes', 'Full-read output bytes', undefined, { min: 1 }),
            number('search_response_budget_bytes', 'Search response bytes', undefined, { min: 1 }),
          ]),
        ],
      },
      {
        title: 'Turn history',
        fields: [
          object('turns', 'Durable change history', [
            text('data_dir', 'Data directory'),
            number('max_blob_bytes', 'Maximum blob-store bytes', undefined, { min: 1 }),
          ]),
        ],
      },
    ],
    expectedFields: [
      'max_timeout_ms',
      'max_bg_timeout_ms',
      'default_timeout_ms',
      'max_output_bytes',
      'working_dir',
      'env.inherit',
      'env.allow[]',
      'denylist_patterns[]',
      'max_concurrent_jobs',
      'job_retention_secs',
      'fs.host_roots[]',
      'fs.allow_unjailed',
      'fs.max_read_bytes',
      'fs.max_write_bytes',
      'fs.denylist_paths[]',
      'fs.allow_special_bits',
      'sandbox.enabled',
      'code.non_accessible_globs[]',
      'code.default_exclude_globs[]',
      'code.max_read_bytes',
      'code.max_write_bytes',
      'code.tree_default_depth',
      'code.tree_per_folder_limit',
      'code.list_default_page_size',
      'code.list_max_page_size',
      'code.search_default_max_matches',
      'code.search_default_max_line_bytes',
      'code.batch_read_budget_bytes',
      'code.max_output_bytes',
      'code.search_response_budget_bytes',
      'turns.data_dir',
      'turns.max_blob_bytes',
    ],
  },
  {
    id: 'worktree',
    title: 'Worktree',
    description: 'Worktree placement, landing, pruning, operation gates and ignored-file provisioning.',
    sections: [
      {
        title: 'Creation and landing',
        fields: [
          text('worktree_root', 'Worktree root'),
          text('branch_prefix', 'Branch prefix'),
          select('branch_naming', 'Branch naming', [
            choice('id', 'Worktree ID'),
            choice('codename', 'Friendly codename'),
          ]),
          text('land_queue', 'Land queue'),
          number('max_land_retries', 'Maximum land retries', undefined, { min: 0 }),
          number('git_timeout_ms', 'Git timeout (ms)', undefined, { min: 1 }),
          number('test_timeout_ms', 'Test gate timeout (ms)', undefined, { min: 1 }),
        ],
      },
      {
        title: 'Automatic pruning',
        fields: [
          text('prune_schedule', 'Prune schedule', 'Six-field cron expression.'),
          number('prune_expire_hours', 'Idle expiry (hours)', undefined, { min: 0 }),
        ],
      },
      {
        title: 'Operation gates',
        fields: [
          object('gates', 'Allowed operations', [
            toggle('allow_remove', 'Allow remove', undefined, { defaultValue: true }),
            toggle('allow_force', 'Allow force operations'),
            toggle('allow_branch_delete', 'Allow branch deletion', undefined, { defaultValue: true }),
            toggle('allow_land', 'Allow land', undefined, { defaultValue: true }),
            toggle('allow_prune', 'Allow prune', undefined, { defaultValue: true }),
            stringList('land_targets', 'Allowed land targets', 'Glob list.', { itemLabel: 'Branch glob' }),
            stringList('repos', 'Allowed repositories', 'Globs over canonical repository paths.', {
              itemLabel: 'Path glob',
            }),
            number('max_worktrees_per_repo', 'Worktrees per repository', 'Zero is unlimited.', { min: 0 }),
          ]),
        ],
      },
      {
        title: 'Ignored-file provisioning',
        fields: [
          object('provision', 'Copy ignored files', [
            toggle('copy_ignored', 'Copy ignored files in the background'),
            stringList('include', 'Include globs', 'Empty includes every ignored path.', { itemLabel: 'Glob' }),
            stringList('exclude', 'Exclude globs', undefined, { itemLabel: 'Glob' }),
            number('max_copy_bytes', 'Maximum copied bytes', undefined, { min: 1 }),
          ]),
        ],
      },
    ],
    expectedFields: [
      'worktree_root',
      'branch_prefix',
      'branch_naming',
      'prune_schedule',
      'prune_expire_hours',
      'land_queue',
      'max_land_retries',
      'git_timeout_ms',
      'test_timeout_ms',
      'gates.allow_remove',
      'gates.allow_force',
      'gates.allow_branch_delete',
      'gates.allow_land',
      'gates.allow_prune',
      'gates.land_targets[]',
      'gates.repos[]',
      'gates.max_worktrees_per_repo',
      'provision.copy_ignored',
      'provision.include[]',
      'provision.exclude[]',
      'provision.max_copy_bytes',
    ],
  },
]
