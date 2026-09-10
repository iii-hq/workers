import { number, password, select, text, toggle } from '../spec-helpers'
import { choice, type WorkerConfigurationSpec } from '../types'

const guidanceDescription = 'Add this worker’s usage guidance to agent system prompts. Changes apply live.'

export const scalarWorkerSpecs: readonly WorkerConfigurationSpec[] = [
  {
    id: 'a2ui',
    title: 'A2UI',
    description: 'Composition defaults and safety budgets for generated interfaces.',
    sections: [
      {
        title: 'Composer',
        fields: [
          text('composer_model', 'Model', 'Optional model override for UI composition.', { optional: true }),
          text('composer_provider', 'Provider', 'Optional provider paired with the model override.', {
            optional: true,
          }),
          number('max_output_tokens', 'Maximum output tokens', 'Output budget for one composition or repair call.', {
            min: 1,
          }),
          number(
            'max_composer_input_bytes',
            'Maximum composer input bytes',
            'Largest UTF-8 prompt sent to the composer.',
            { min: 1 },
          ),
          number('repair_attempts', 'Repair attempts', 'Additional correction calls after an invalid response.', {
            min: 0,
            max: 3,
          }),
        ],
      },
      {
        title: 'Surface limits',
        fields: [
          number('max_surfaces_per_session', 'Surfaces per session', undefined, { min: 1 }),
          number('max_history_per_surface', 'History per surface', undefined, { min: 1 }),
          number('max_templates_per_session', 'Templates per session', undefined, { min: 1 }),
          number('max_components_per_surface', 'Components per surface', undefined, { min: 1 }),
          number('max_description_bytes', 'Description bytes', undefined, { min: 1 }),
          number('max_data_bytes', 'Data model bytes', undefined, { min: 1 }),
          number('max_surface_bytes', 'Surface bytes', undefined, { min: 1 }),
          number('max_session_bytes', 'Session bytes', undefined, { min: 1 }),
        ],
      },
      {
        title: 'Actions',
        fields: [
          toggle(
            'forward_actions',
            'Forward console actions',
            'Send actions back to the originating Harness session.',
            { defaultValue: true },
          ),
        ],
      },
    ],
    expectedFields: [
      'composer_model',
      'composer_provider',
      'max_output_tokens',
      'max_composer_input_bytes',
      'repair_attempts',
      'max_surfaces_per_session',
      'max_history_per_surface',
      'max_templates_per_session',
      'max_components_per_surface',
      'max_description_bytes',
      'max_data_bytes',
      'max_surface_bytes',
      'max_session_bytes',
      'forward_actions',
    ],
  },
  {
    id: 'canvas',
    title: 'Canvas',
    description: 'Limits for stored canvas sources and catalog responses.',
    sections: [
      {
        title: 'Limits',
        fields: [
          number('max_source_bytes', 'Maximum source bytes', 'Largest Excalidraw or Mermaid source accepted.', {
            min: 1,
          }),
          number('max_list', 'Maximum list size', 'Most canvas records returned by one list call.', { min: 1 }),
        ],
      },
    ],
    expectedFields: ['max_source_bytes', 'max_list'],
  },
  {
    id: 'computer',
    title: 'Computer',
    description: 'Session, capture, connection and sandbox desktop defaults.',
    legacyWrapper: 'computer',
    sections: [
      {
        title: 'Sessions',
        fields: [
          text('default_endpoint', 'Default endpoint', 'Guest executor endpoint used when a session omits one.'),
          select('os', 'Operating system label', [
            choice('linux', 'Linux'),
            choice('macos', 'macOS'),
            choice('windows', 'Windows'),
            choice('android', 'Android'),
          ]),
          number('max_sessions', 'Maximum sessions', undefined, { min: 1 }),
          number('idle_stop_ms', 'Idle stop (ms)', 'Zero disables the idle sweep.', { min: 0 }),
          number('command_timeout_ms', 'Command timeout (ms)', undefined, { min: 1 }),
          number('connect_timeout_ms', 'Connection timeout (ms)', undefined, { min: 1 }),
        ],
      },
      {
        title: 'Capture',
        fields: [
          number('screencast_fps', 'Screencast FPS', undefined, { min: 1 }),
          number('max_screenshot_dimension', 'Maximum screenshot dimension', 'Longest image edge in pixels.', {
            min: 320,
            max: 4096,
          }),
          number('screenshot_quality', 'JPEG quality', undefined, { min: 1, max: 100 }),
          toggle(
            'screen_capture_preflight',
            'Screen capture preflight',
            'Ask macOS for Screen Recording before starting a native session.',
            { defaultValue: true },
          ),
        ],
      },
      {
        title: 'Sandbox desktop',
        fields: [
          text('sandbox_image', 'Image', 'Default iii-sandbox image or preset.'),
          number('sandbox_width', 'Display width', undefined, { min: 1 }),
          number('sandbox_height', 'Display height', undefined, { min: 1 }),
          toggle('sandbox_network', 'Network access', 'Allow sandbox-backed desktops to reach the network.', {
            defaultValue: true,
          }),
          number('sandbox_idle_timeout_secs', 'Sandbox idle timeout (seconds)', undefined, { min: 1 }),
        ],
      },
    ],
    expectedFields: [
      'default_endpoint',
      'os',
      'max_sessions',
      'idle_stop_ms',
      'screencast_fps',
      'max_screenshot_dimension',
      'screenshot_quality',
      'command_timeout_ms',
      'connect_timeout_ms',
      'sandbox_image',
      'sandbox_width',
      'sandbox_height',
      'sandbox_network',
      'sandbox_idle_timeout_secs',
      'screen_capture_preflight',
    ],
  },
  {
    id: 'cursor',
    title: 'Cursor',
    description: 'Backend, credentials, process paths and RPC budgets for Cursor agents.',
    sections: [
      {
        title: 'Backend',
        fields: [
          select('local_backend', 'Local backend', [
            choice('cli-acp', 'Cursor Agent CLI'),
            choice('sdk-bridge', 'SDK bridge'),
          ]),
          text('agent_binary', 'Agent binary', 'Optional path to the official Cursor Agent CLI.'),
          text('bridge_binary', 'Bridge binary', 'Path to the separately installed SDK bridge.'),
          password('api_key', 'API key', 'Environment reference or key used by the SDK bridge.'),
          text('workspace', 'Workspace', 'Default workspace for local and cloud processes.'),
        ],
      },
      {
        title: 'Runtime',
        fields: [
          number('startup_timeout_ms', 'Startup timeout (ms)', undefined, { min: 1 }),
          number('shutdown_timeout_ms', 'Shutdown timeout (ms)', undefined, { min: 1 }),
          number('rpc_timeout_ms', 'RPC timeout (ms)', undefined, { min: 1 }),
          number('max_frame_bytes', 'Maximum frame bytes', undefined, { min: 1 }),
          text('events_stream', 'Events stream'),
          text('raw_events_stream', 'Raw events stream'),
        ],
      },
    ],
    expectedFields: [
      'local_backend',
      'agent_binary',
      'api_key',
      'bridge_binary',
      'workspace',
      'startup_timeout_ms',
      'shutdown_timeout_ms',
      'rpc_timeout_ms',
      'max_frame_bytes',
      'events_stream',
      'raw_events_stream',
    ],
  },
  {
    id: 'document',
    title: 'Document',
    description: 'Extraction, embedded-asset and OCR budgets.',
    sections: [
      {
        title: 'Extraction',
        fields: [
          number('max_input_bytes', 'Maximum input bytes', undefined, { min: 1 }),
          number('max_chars', 'Maximum characters', 'Zero disables the returned-text cap.', { min: 0 }),
          number('preview_chars', 'Preview characters', undefined, { min: 0 }),
          number('max_assets', 'Maximum assets', undefined, { min: 0 }),
          number('max_asset_bytes', 'Maximum bytes per asset', undefined, { min: 1 }),
          number('max_assets_total_bytes', 'Maximum total asset bytes', undefined, { min: 1 }),
        ],
      },
      {
        title: 'OCR',
        fields: [
          text('ocr_model', 'OCR model', 'Optional vision model used when a call names none.', { optional: true }),
          number('max_ocr_pages', 'Maximum OCR pages', undefined, { min: 1 }),
          number('ocr_timeout_ms', 'OCR timeout (ms)', undefined, { min: 1 }),
          number('ocr_render_settle_ms', 'Render settle time (ms)', undefined, { min: 0 }),
          toggle('ocr_cache', 'Cache OCR results', undefined, { defaultValue: true }),
        ],
      },
    ],
    expectedFields: [
      'max_input_bytes',
      'max_chars',
      'preview_chars',
      'max_assets',
      'max_asset_bytes',
      'max_assets_total_bytes',
      'ocr_model',
      'max_ocr_pages',
      'ocr_timeout_ms',
      'ocr_render_settle_ms',
      'ocr_cache',
    ],
  },
  {
    id: 'editor',
    title: 'Editor',
    description: 'Diff, file, search and Git operation limits.',
    sections: [
      {
        title: 'Limits',
        fields: [
          number('max_diff_bytes', 'Maximum diff bytes', undefined, { min: 1 }),
          number('diff_context_lines', 'Diff context lines', undefined, { min: 0 }),
          number('find_limit', 'Find result limit', undefined, { min: 1 }),
          number('max_find_candidates', 'Maximum find candidates', undefined, { min: 1 }),
          number('max_file_bytes', 'Maximum file bytes', undefined, { min: 1 }),
          number('search_max_matches', 'Maximum search matches', undefined, { min: 1 }),
          number('git_timeout_ms', 'Git timeout (ms)', undefined, { min: 1 }),
        ],
      },
    ],
    expectedFields: [
      'max_diff_bytes',
      'diff_context_lines',
      'find_limit',
      'max_find_candidates',
      'max_file_bytes',
      'search_max_matches',
      'git_timeout_ms',
    ],
  },
  {
    id: 'fp',
    title: 'Functional pipeline',
    description: 'Prompt guidance for the functional pipeline worker.',
    sections: [
      {
        title: 'Agent guidance',
        fields: [toggle('inject_guidance', 'Inject guidance', guidanceDescription, { defaultValue: true })],
      },
    ],
    expectedFields: ['inject_guidance'],
  },
  {
    id: 'github',
    title: 'GitHub',
    description: 'GitHub CLI authentication and execution limits.',
    sections: [
      {
        title: 'CLI',
        fields: [
          text('gh_executable', 'GitHub CLI executable', 'Binary name or absolute path.'),
          password('token', 'Token', 'Optional token or environment reference.', { optional: true }),
        ],
      },
      {
        title: 'Limits',
        fields: [
          number('default_timeout_ms', 'Default timeout (ms)', undefined, { min: 1 }),
          number('max_timeout_ms', 'Maximum timeout (ms)', undefined, { min: 1 }),
          number('max_output_bytes', 'Maximum output bytes', undefined, { min: 1 }),
        ],
      },
    ],
    expectedFields: ['gh_executable', 'token', 'default_timeout_ms', 'max_timeout_ms', 'max_output_bytes'],
  },
  {
    id: 'memory',
    title: 'Memory',
    description: 'Storage, recall, extraction, learned-rule and embedding behavior.',
    sections: [
      {
        title: 'Storage and recall',
        fields: [
          text('data_dir', 'Data directory'),
          text('default_bank', 'Default bank'),
          toggle('inject_rules', 'Inject rules', undefined, { defaultValue: true }),
          toggle('inject_memories', 'Inject memories', undefined, { defaultValue: true }),
          number('recall_limit', 'Recall limit', undefined, { min: 1 }),
          number('recall_budget_tokens', 'Recall token budget', undefined, { min: 1 }),
          number('decay_half_life_days', 'Recency half-life (days)', undefined, { min: 1 }),
        ],
      },
      {
        title: 'Extraction',
        fields: [
          toggle('extraction_enabled', 'Extract memories', undefined, { defaultValue: true }),
          text('extraction_model', 'Extraction model', 'Empty uses the first router model.'),
          number('extraction_window', 'Message window', undefined, { min: 1 }),
          number('extraction_timeout_ms', 'Extraction timeout (ms)', undefined, { min: 1 }),
          number('max_memories_per_turn', 'Memories per turn', undefined, { min: 1 }),
        ],
      },
      {
        title: 'Learning and embeddings',
        fields: [
          toggle('rule_learning_enabled', 'Learn standing rules', undefined, { defaultValue: true }),
          number('max_rule_chars', 'Rule character budget', undefined, { min: 1 }),
          toggle('embeddings_enabled', 'Use embeddings', undefined, { defaultValue: true }),
          text('embedding_model', 'Embedding model', 'Empty uses the provider default.'),
        ],
      },
    ],
    expectedFields: [
      'data_dir',
      'default_bank',
      'inject_rules',
      'inject_memories',
      'recall_limit',
      'recall_budget_tokens',
      'extraction_enabled',
      'extraction_model',
      'extraction_window',
      'extraction_timeout_ms',
      'max_memories_per_turn',
      'rule_learning_enabled',
      'max_rule_chars',
      'decay_half_life_days',
      'embeddings_enabled',
      'embedding_model',
    ],
  },
  {
    id: 'openwiki',
    title: 'OpenWiki',
    description: 'Generation model, writer concurrency and refresh defaults.',
    sections: [
      {
        title: 'Generation',
        fields: [
          text('model', 'Model'),
          number('max_parallel', 'Parallel writers', undefined, { min: 1, max: 16 }),
          select('refresh_default', 'Refresh cadence', [
            choice('off', 'Off'),
            choice('3h', 'Every 3 hours'),
            choice('6h', 'Every 6 hours'),
            choice('12h', 'Every 12 hours'),
            choice('daily', 'Daily'),
            choice('weekly', 'Weekly'),
          ]),
        ],
      },
    ],
    expectedFields: ['model', 'max_parallel', 'refresh_default'],
  },
  {
    id: 'pdf',
    title: 'PDF',
    description: 'Parsing limits and text-versus-scan classification thresholds.',
    sections: [
      {
        title: 'Extraction',
        fields: [
          number('max_input_bytes', 'Maximum input bytes', undefined, { min: 1 }),
          number('max_chars', 'Maximum characters', 'Zero disables the returned-text cap.', { min: 0 }),
          number('preview_chars', 'Preview characters', undefined, { min: 0 }),
          number('max_items', 'Maximum items', undefined, { min: 1 }),
        ],
      },
      {
        title: 'Classification',
        fields: [
          number('classify_sample_pages', 'Sample pages', undefined, { min: 1 }),
          number('min_text_ops_per_page', 'Minimum text operations per page', undefined, { min: 0 }),
          number('text_page_ratio_threshold', 'Text page ratio threshold', undefined, { min: 0, max: 1, step: 0.01 }),
        ],
      },
    ],
    expectedFields: [
      'max_input_bytes',
      'max_chars',
      'preview_chars',
      'max_items',
      'classify_sample_pages',
      'min_text_ops_per_page',
      'text_page_ratio_threshold',
    ],
  },
  {
    id: 'sandbox-code-runner',
    title: 'Sandbox code runner',
    description: 'Live configuration shared with the Harness prompt.',
    sections: [
      {
        title: 'Agent guidance',
        fields: [toggle('inject_guidance', 'Inject guidance', guidanceDescription, { defaultValue: true })],
      },
    ],
    expectedFields: ['inject_guidance'],
  },
  {
    id: 'scrapling',
    title: 'Scrapling',
    description: 'Prompt guidance for web scraping functions.',
    sections: [
      {
        title: 'Agent guidance',
        fields: [toggle('inject_guidance', 'Inject guidance', guidanceDescription, { defaultValue: true })],
      },
    ],
    expectedFields: ['inject_guidance'],
  },
  {
    id: 'tailscale',
    title: 'Tailscale',
    description: 'Console sharing through Tailscale Serve and Funnel.',
    legacyWrapper: 'tailscale',
    sections: [
      {
        title: 'Connection',
        fields: [
          text('tailscale_binary', 'Tailscale executable'),
          text('console_url', 'Console URL', 'Must point to a loopback HTTP(S) Console root.'),
          number('default_https_port', 'Default HTTPS port', undefined, { min: 1, max: 65535 }),
          number('command_timeout_ms', 'Command timeout (ms)', undefined, { min: 1 }),
        ],
      },
      {
        title: 'Public sharing',
        fields: [toggle('allow_funnel', 'Allow Funnel', 'Permit public Tailscale Funnel shares.')],
      },
    ],
    expectedFields: ['tailscale_binary', 'console_url', 'default_https_port', 'allow_funnel', 'command_timeout_ms'],
  },
  {
    id: 'vscode',
    title: 'VS Code',
    description: 'VS Code Server process paths, listener range and lifecycle timeouts.',
    sections: [
      {
        title: 'Runtime',
        fields: [
          text('code_executable', 'VS Code executable'),
          text('data_dir', 'Data directory'),
          text('bind_host', 'Bind host'),
        ],
      },
      {
        title: 'Ports and lifecycle',
        fields: [
          number('port_min', 'First port', undefined, { min: 1, max: 65535 }),
          number('port_max', 'Last port', undefined, { min: 1, max: 65535 }),
          number('start_timeout_ms', 'Start timeout (ms)', undefined, { min: 1 }),
          number('stop_grace_ms', 'Stop grace period (ms)', undefined, { min: 1 }),
        ],
      },
    ],
    expectedFields: [
      'code_executable',
      'data_dir',
      'bind_host',
      'port_min',
      'port_max',
      'start_timeout_ms',
      'stop_grace_ms',
    ],
  },
  {
    id: 'web',
    title: 'Web fetch',
    description: 'Network, response and transformation limits for web::fetch.',
    sections: [
      {
        title: 'Timeouts and responses',
        fields: [
          number('default_timeout_ms', 'Default timeout (ms)', undefined, { min: 1 }),
          number('max_timeout_ms', 'Maximum timeout (ms)', undefined, { min: 1 }),
          number('default_response_bytes', 'Default response bytes', undefined, { min: 1 }),
          number('max_response_bytes', 'Maximum response bytes', undefined, { min: 1 }),
          number('max_transform_bytes', 'Maximum transform bytes', undefined, { min: 1 }),
          number('max_redirects', 'Maximum redirects', undefined, { min: 0 }),
        ],
      },
      {
        title: 'Network behavior',
        fields: [
          text('user_agent', 'User agent'),
          toggle('allow_loopback', 'Allow loopback', 'Other private address ranges remain blocked.', {
            defaultValue: true,
          }),
          toggle('inject_guidance', 'Inject guidance', guidanceDescription, { defaultValue: true }),
        ],
      },
    ],
    expectedFields: [
      'default_timeout_ms',
      'max_timeout_ms',
      'default_response_bytes',
      'max_response_bytes',
      'max_transform_bytes',
      'max_redirects',
      'user_agent',
      'allow_loopback',
      'inject_guidance',
    ],
  },
  {
    id: 'workflow',
    title: 'Workflow',
    description: 'Pending-call sweep, dispatch and retry behavior.',
    sections: [
      {
        title: 'Execution',
        fields: [
          number('default_pending_timeout_ms', 'Pending timeout (ms)', undefined, { min: 1 }),
          number('dispatch_timeout_ms', 'Dispatch timeout (ms)', undefined, { min: 1 }),
          number('max_node_retries', 'Node retries', undefined, { min: 0 }),
        ],
      },
      {
        title: 'Sweep and guidance',
        fields: [
          text('sweep_expression', 'Sweep schedule', 'Six-field cron expression.'),
          toggle('inject_guidance', 'Inject guidance', guidanceDescription, { defaultValue: true }),
        ],
      },
    ],
    expectedFields: [
      'default_pending_timeout_ms',
      'sweep_expression',
      'dispatch_timeout_ms',
      'max_node_retries',
      'inject_guidance',
    ],
  },
]
