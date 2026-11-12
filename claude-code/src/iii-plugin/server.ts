/**
 * The plugin's MCP server: agent profiles and skills from the engine's
 * `iii-directory`, served to Claude Code over stdio.
 *
 * Hand-rolled JSON-RPC (newline-delimited, MCP 2024-11-05 / 2025-06-18) rather
 * than the MCP SDK: the surface is five tools and resources, and the
 * bundle stays one file with iii-sdk as its only dependency.
 *
 *   tools      iii_status, iii_list_agents, iii_get_agent, iii_list_skills, iii_get_skill
 *   resources  `iii-skill://<id>` per skill
 */

import { type CallArgs, callFunction, type ObserveArgs, ObserverBuffer } from './bus.js';
import type { Directory } from './directory.js';

const PROTOCOL = '2025-06-18';
const SUPPORTED = new Set(['2024-11-05', '2025-03-26', '2025-06-18']);

type Json = Record<string, unknown>;
type Request = { jsonrpc: '2.0'; id?: string | number; method: string; params?: Json };

const TOOLS = [
  {
    name: 'iii_status',
    description:
      'Which iii engine this plugin is attached to, how it was found, and the connection state.',
    inputSchema: { type: 'object', properties: {} },
  },
  {
    name: 'iii_list_agents',
    description:
      'List the agent profiles installed in the iii engine (id, name, description, model, skill/function counts).',
    inputSchema: {
      type: 'object',
      properties: {
        include_hidden: { type: 'boolean', description: 'Include hidden helper profiles.' },
      },
    },
  },
  {
    name: 'iii_get_agent',
    description:
      'Fetch one agent profile: its full system prompt (resolved through `extends`), preloaded skills and functions. Adopt the prompt to act as that agent, or pass it to a subagent.',
    inputSchema: { type: 'object', properties: { id: { type: 'string' } }, required: ['id'] },
  },
  {
    name: 'iii_list_skills',
    description:
      'List skills installed in the iii engine. Filter with `search` (substring) or `prefix` (worker namespace).',
    inputSchema: {
      type: 'object',
      properties: { search: { type: 'string' }, prefix: { type: 'string' } },
    },
  },
  {
    name: 'iii_call',
    description:
      'Call any function on the iii engine by id and return its result. The way to act on the engine: use it instead of the `iii call` CLI. Find ids with engine::functions::list or directory::search_functions, and read a contract with engine::functions::info before the first call.',
    inputSchema: {
      type: 'object',
      properties: {
        function_id: { type: 'string', description: 'e.g. `engine::workers::list`' },
        payload: {
          type: 'object',
          description: 'The function arguments (a JSON object). Default {}.',
        },
        timeout_ms: { type: 'number' },
        namespace: { type: 'string', description: 'Route to another namespace.' },
        void: {
          type: 'boolean',
          description: 'Fire and forget: return without waiting for a result.',
        },
        max_chars: { type: 'number', description: 'Cap on the returned JSON (default 100000).' },
      },
      required: ['function_id'],
    },
  },
  {
    name: 'iii_observe',
    description:
      'Observe a function on the iii engine: every matching invocation of it is buffered for this session. Returns an observer_id; read what happened with iii_events, remove it with iii_unobserve. Use it instead of polling.',
    inputSchema: {
      type: 'object',
      properties: {
        function_id: { type: 'string', description: 'The function to observe, e.g. `state::set`.' },
        filters: {
          type: 'object',
          description:
            "Which calls match, e.g. { 'input.scope': { $eq: 'jobs' } }. Default: every call.",
        },
        args: {
          type: 'object',
          description: "The observed function's args_format (see engine::functions::info).",
        },
        capture: {
          type: 'array',
          items: { type: 'string', enum: ['input', 'output', 'error', 'caller', 'metadata'] },
          description:
            "What each event carries. Default ['input','output','caller'] (successes only); add 'error' for failures.",
        },
        namespace: { type: 'string' },
        max_events: { type: 'number', description: 'Remove the observer after this many matches.' },
        expires_in_ms: { type: 'number', description: 'Remove the observer after this long.' },
      },
      required: ['function_id'],
    },
  },
  {
    name: 'iii_events',
    description:
      'Read (and remove) the buffered events of one observer, or of all of them, oldest first. Reports how many remain and how many were dropped because the buffer was full.',
    inputSchema: {
      type: 'object',
      properties: {
        observer_id: { type: 'string', description: 'Omit for every observer.' },
        max: { type: 'number', description: 'Default 50.' },
      },
    },
  },
  {
    name: 'iii_observers',
    description:
      "This session's observers: function, filters, state, and buffered / received / dropped counts.",
    inputSchema: { type: 'object', properties: {} },
  },
  {
    name: 'iii_unobserve',
    description: 'Remove an observer created with iii_observe, discarding its unread events.',
    inputSchema: {
      type: 'object',
      properties: { observer_id: { type: 'string' } },
      required: ['observer_id'],
    },
  },
  {
    name: 'iii_get_skill',
    description: 'Fetch one skill by id and return its markdown body.',
    inputSchema: { type: 'object', properties: { id: { type: 'string' } }, required: ['id'] },
  },
];

/** What Claude Code shows the model about this server. */
export const INSTRUCTIONS = [
  'The running iii engine. Everything on it is a function, called by id.',
  'Act on the engine with iii_call (any function id, with a JSON payload) instead of the `iii call` CLI or curl; discover ids with iii_call on engine::functions::list or directory::search_functions, and read a contract with engine::functions::info before its first call.',
  'React to the engine with iii_observe instead of polling: it buffers every matching invocation of a function for this session; read them with iii_events, list them with iii_observers, and remove each one with iii_unobserve when done.',
  'Profiles written for the iii harness say agent_call and harness::observe: here those are iii_call and iii_observe.',
  'Agent profiles: iii_list_agents / iii_get_agent (or /iii:profile). Skills: iii_list_skills / iii_get_skill (or /iii:skills).',
].join(' ');

class RpcError extends Error {
  constructor(
    readonly code: number,
    message: string,
  ) {
    super(message);
  }
}

function text(value: unknown, isError = false): Json {
  const body = typeof value === 'string' ? value : JSON.stringify(value, null, 2);
  return { content: [{ type: 'text', text: body }], ...(isError ? { isError: true } : {}) };
}

function requireId(args: Json): string {
  const id = typeof args.id === 'string' ? args.id.trim() : '';
  if (!id) throw new RpcError(-32602, '`id` is required');
  return id;
}

async function callTool(
  dir: Directory,
  observers: ObserverBuffer,
  name: string,
  args: Json,
): Promise<Json> {
  try {
    switch (name) {
      case 'iii_status': {
        // Attempt the connection so the answer is the engine, not "not yet".
        const error = await dir.connectNow().then(
          () => '',
          (err: unknown) => String(err instanceof Error ? err.message : err),
        );
        return text({ ...dir.status(), ...(error ? { error } : {}) });
      }
      case 'iii_call':
        return text(await callFunction(await dir.bus(), args as unknown as CallArgs));
      case 'iii_observe':
        return text(await observers.observe(args as unknown as ObserveArgs));
      case 'iii_events':
        return text(
          observers.events({
            ...(typeof args.observer_id === 'string' ? { observer_id: args.observer_id } : {}),
            ...(typeof args.max === 'number' ? { max: args.max } : {}),
          }),
        );
      case 'iii_observers':
        return text(observers.list());
      case 'iii_unobserve': {
        const id = typeof args.observer_id === 'string' ? args.observer_id.trim() : '';
        if (!id) throw new RpcError(-32602, '`observer_id` is required');
        return text(await observers.unobserve(id));
      }
      case 'iii_list_agents':
        return text(await dir.listAgents(args.include_hidden === true));
      case 'iii_get_agent':
        return text(await dir.getAgent(requireId(args)));
      case 'iii_list_skills':
        return text(
          await dir.listSkills({
            ...(typeof args.search === 'string' ? { search: args.search } : {}),
            ...(typeof args.prefix === 'string' ? { prefix: args.prefix } : {}),
          }),
        );
      case 'iii_get_skill': {
        const skill = await dir.getSkill(requireId(args));
        return text(skill.body ?? '');
      }
      default:
        throw new RpcError(-32602, `unknown tool: ${name}`);
    }
  } catch (err) {
    if (err instanceof RpcError) throw err;
    // Engine-side failures are tool results, so the model reads them.
    return text(String(err instanceof Error ? err.message : err), true);
  }
}

export function createHandler(
  dir: Directory,
  observers: ObserverBuffer = new ObserverBuffer(() => dir.bus()),
) {
  return async function handle(req: Request): Promise<unknown> {
    const params = req.params ?? {};
    switch (req.method) {
      case 'initialize': {
        const asked = String(params.protocolVersion ?? '');
        return {
          protocolVersion: SUPPORTED.has(asked) ? asked : PROTOCOL,
          capabilities: { tools: {}, resources: {} },
          serverInfo: { name: 'iii-plugin', version: '0.1.0' },
          instructions: INSTRUCTIONS,
        };
      }
      case 'ping':
        return {};
      case 'tools/list':
        return { tools: TOOLS };
      case 'tools/call':
        return callTool(
          dir,
          observers,
          String(params.name ?? ''),
          (params.arguments as Json) ?? {},
        );
      case 'resources/list': {
        const skills = await dir.listSkills().catch(() => []);
        return {
          resources: skills.map((s) => ({
            uri: `iii-skill://${s.id}`,
            name: s.id,
            title: s.title ?? s.id,
            description: s.description ?? '',
            mimeType: 'text/markdown',
          })),
        };
      }
      case 'resources/read': {
        const uri = String(params.uri ?? '');
        if (!uri.startsWith('iii-skill://')) throw new RpcError(-32602, `unknown resource: ${uri}`);
        const skill = await dir.getSkill(uri.slice('iii-skill://'.length));
        return { contents: [{ uri, mimeType: 'text/markdown', text: skill.body ?? '' }] };
      }
      default:
        throw new RpcError(-32601, `method not found: ${req.method}`);
    }
  };
}

/** Serve MCP on stdin/stdout until stdin closes. */
export async function serve(dir: Directory): Promise<void> {
  const observers = new ObserverBuffer(() => dir.bus());
  // stdout is the protocol channel: anything else printed there corrupts it,
  // and iii-sdk logs its connection banners to stdout. Keep the real writer for
  // JSON-RPC and send every other stdout write to stderr.
  const out = process.stdout.write.bind(process.stdout);
  process.stdout.write = process.stderr.write.bind(process.stderr) as typeof process.stdout.write;
  const handle = createHandler(dir, observers);
  const write = (msg: unknown) => out(`${JSON.stringify(msg)}\n`);
  let buffer = '';
  process.stdin.setEncoding('utf8');
  process.stdin.on('data', (chunk: string) => {
    buffer += chunk;
    let nl = buffer.indexOf('\n');
    while (nl >= 0) {
      const line = buffer.slice(0, nl).trim();
      buffer = buffer.slice(nl + 1);
      nl = buffer.indexOf('\n');
      if (!line) continue;
      let req: Request;
      try {
        req = JSON.parse(line);
      } catch {
        write({ jsonrpc: '2.0', id: null, error: { code: -32700, message: 'parse error' } });
        continue;
      }
      if (req.id === undefined) continue; // notifications need no answer
      handle(req).then(
        (result) => write({ jsonrpc: '2.0', id: req.id, result }),
        (err) =>
          write({
            jsonrpc: '2.0',
            id: req.id,
            error: {
              code: err instanceof RpcError ? err.code : -32603,
              message: String(err instanceof Error ? err.message : err),
            },
          }),
      );
    }
  });
  await new Promise<void>((resolve) => process.stdin.once('end', resolve));
  // Observers are not replayed on reconnect and die with the connection, but
  // dispose them explicitly so the engine forgets them at once.
  await observers.close();
  await dir.close();
}
