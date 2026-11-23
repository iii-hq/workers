import type { Server, ServersList } from '../../src/types'

export const NOW = 1_700_000_100_000
export function server(over: Partial<Server> = {}): Server {
  return {
    name: 'linear',
    transport: 'http',
    target: 'https://mcp.linear.app/mcp',
    auth: 'bearer',
    state: 'ready',
    stage: null,
    since: NOW - 60_000,
    error: null,
    approved: true,
    fingerprint: 'f'.repeat(64),
    protocol_version: '2025-11-25',
    server_info: { name: 'linear', title: null, version: '1.0.0' },
    tools_listed: true,
    tools: [
      {
        name: 'list_issues',
        function_id: 'mcp::linear::list_issues',
        registered: true,
        annotations: { readOnlyHint: true },
      },
      {
        name: 'save_issue',
        function_id: 'mcp::linear::save_issue',
        registered: true,
        annotations: { readOnlyHint: false, destructiveHint: false },
      },
    ],
    skipped: [],
    has_instructions: false,
    capabilities: { resources: false, prompts: false },
    resource_functions: [],
    prompts: [],
    list_errors: { prompts: null, templates: null },
    login: null,
    process: null,
    ...over,
  }
}
export const list = (servers: Server[]): ServersList => ({
  stdio_enabled: true,
  callback_url: 'http://127.0.0.1:3111/mcp-client/oauth/callback',
  servers,
})
export const stdio = (over: Partial<Server> = {}): Server =>
  server({
    name: 'playwright',
    transport: 'stdio',
    target: 'npx -y @playwright/mcp@0.0.41',
    auth: 'none',
    process: {
      pid: 48213,
      started_at: NOW - 8_040_000,
      restarts: 0,
      restart_budget: 5,
      next_restart_at: null,
      last_exit: null,
    },
    ...over,
  })
export const docs = (): Server =>
  server({
    name: 'docs',
    has_instructions: true,
    capabilities: { resources: true, prompts: true },
    resource_functions: (['list', 'read', 'templates'] as const).map((name) => ({
      name,
      function_id: `mcp::docs::resources::${name}`,
      registered: true,
    })),
    prompts: [
      {
        name: 'code_review',
        function_id: 'mcp::docs::prompts::code_review',
        registered: true,
        title: null,
        arguments: 2,
        required: 1,
      },
    ],
    tools: [{ name: 'search', function_id: 'mcp::docs::search', registered: true, annotations: null }],
  })
