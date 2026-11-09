/**
 * The plugin's MCP server: agent profiles and skills from the engine's
 * `iii-directory`, served to Claude Code over stdio.
 *
 * Hand-rolled JSON-RPC (newline-delimited, MCP 2024-11-05 / 2025-06-18) rather
 * than the MCP SDK: the surface is five tools, prompts and resources, and the
 * bundle stays one file with iii-sdk as its only dependency.
 *
 *   tools      iii_status, iii_list_agents, iii_get_agent, iii_list_skills, iii_get_skill
 *   prompts    one per visible agent profile (`/mcp__iii-directory__<id>`)
 *   resources  `iii-skill://<id>` per skill
 */

import { Directory, renderSubagent } from './directory.js';

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
    name: 'iii_get_skill',
    description: 'Fetch one skill by id and return its markdown body.',
    inputSchema: { type: 'object', properties: { id: { type: 'string' } }, required: ['id'] },
  },
];

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

async function callTool(dir: Directory, name: string, args: Json): Promise<Json> {
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

export function createHandler(dir: Directory) {
  return async function handle(req: Request): Promise<unknown> {
    const params = req.params ?? {};
    switch (req.method) {
      case 'initialize': {
        const asked = String(params.protocolVersion ?? '');
        return {
          protocolVersion: SUPPORTED.has(asked) ? asked : PROTOCOL,
          capabilities: { tools: {}, prompts: {}, resources: {} },
          serverInfo: { name: 'claude-cli-iii-integration', version: '0.1.0' },
          instructions:
            'Agent profiles and skills from the running iii engine. Use iii_list_agents / iii_get_agent to adopt an iii agent profile, and iii_list_skills / iii_get_skill to read iii skills.',
        };
      }
      case 'ping':
        return {};
      case 'tools/list':
        return { tools: TOOLS };
      case 'tools/call':
        return callTool(dir, String(params.name ?? ''), (params.arguments as Json) ?? {});
      case 'prompts/list': {
        // A missing engine is an empty list, not an error: Claude Code lists
        // prompts at startup and should not show the server as broken.
        const agents = await dir.listAgents().catch(() => []);
        return {
          prompts: agents.map((a) => ({
            name: a.id,
            title: a.name ?? a.id,
            description: a.description ?? '',
            arguments: [
              { name: 'task', description: 'What the agent should do.', required: false },
            ],
          })),
        };
      }
      case 'prompts/get': {
        const profile = await dir.getAgent(String(params.name ?? ''));
        const task = String((params.arguments as Json | undefined)?.task ?? '').trim();
        return {
          description: profile.description ?? '',
          messages: [
            {
              role: 'user',
              content: {
                type: 'text',
                text: `Act as the iii agent profile \`${profile.id}\` for the rest of this conversation. Its instructions follow.\n\n<agent-profile id="${profile.id}">\n${renderSubagent(profile)}</agent-profile>${task ? `\n\nTask: ${task}` : ''}`,
              },
            },
          ],
        };
      }
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
  // stdout is the protocol channel: anything else printed there corrupts it,
  // and iii-sdk logs its connection banners to stdout. Keep the real writer for
  // JSON-RPC and send every other stdout write to stderr.
  const out = process.stdout.write.bind(process.stdout);
  process.stdout.write = process.stderr.write.bind(process.stderr) as typeof process.stdout.write;
  const handle = createHandler(dir);
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
  await dir.close();
}
