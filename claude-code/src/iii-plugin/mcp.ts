/**
 * The plugin's way to the engine: the `mcp-server` worker, over MCP's
 * Streamable HTTP transport.
 *
 * Claude Code itself talks to mcp-server through the plugin's `.mcp.json`;
 * this client is for the plugin's own commands (`sync`, `export`, `status`),
 * which run outside a Claude session. It speaks just enough MCP for that:
 * initialize, tools/call, and DELETE to end the session. Plain `fetch`, no
 * iii-sdk: every engine call goes through mcp-server, and so through
 * rbac-proxy's rules.
 */

import type { AgentProfile, AgentRow, Skill, SkillRow } from './directory.js';

export const DEFAULT_MCP_URL = 'http://127.0.0.1:3111/mcp';
const PROTOCOL = '2025-06-18';
const TIMEOUT_MS = 15_000;
const SETUP =
  'The iii plugin needs the mcp-server worker running on your iii engine, behind rbac-proxy ' +
  '(https://github.com/iii-hq/workers/tree/main/mcp-server). If it runs elsewhere, set the ' +
  "plugin's mcp_url option (or III_PLUGIN_MCP_URL, or --url).";

export type McpUrlSource = '--url' | 'III_PLUGIN_MCP_URL' | 'plugin option mcp_url' | 'default';

/** `3211`, `:3211`, `host:3211` or a URL without a path become `http://…/mcp`. */
export function normalizeMcpUrl(raw: string): string {
  let value = raw.trim();
  if (/^wss?:\/\//i.test(value)) {
    throw new Error(
      `${value} is an engine WebSocket address; the iii plugin talks to mcp-server over HTTP (for example ${DEFAULT_MCP_URL})`,
    );
  }
  if (/^\d+$/.test(value)) value = `127.0.0.1:${value}`;
  if (value.startsWith(':')) value = `127.0.0.1${value}`;
  if (!/^https?:\/\//i.test(value)) value = `http://${value}`;
  const url = new URL(value);
  if (url.pathname === '/' || url.pathname === '') url.pathname = '/mcp';
  return url.toString();
}

function given(value: string | undefined): string {
  const v = value?.trim() ?? '';
  // Claude Code leaves `${user_config.x}` as written when the option is unset.
  return v.includes('${') ? '' : v;
}

/** Where mcp-server is: `--url`, `III_PLUGIN_MCP_URL`, the plugin option, then the default. */
export function resolveMcpUrl(
  argv: string[],
  env: NodeJS.ProcessEnv = process.env,
): { url: string; source: McpUrlSource } {
  const i = argv.indexOf('--url');
  const candidates: [string, McpUrlSource][] = [
    [i >= 0 ? (argv[i + 1] ?? '') : '', '--url'],
    [given(env.III_PLUGIN_MCP_URL), 'III_PLUGIN_MCP_URL'],
    [given(env.CLAUDE_PLUGIN_OPTION_MCP_URL), 'plugin option mcp_url'],
  ];
  for (const [value, source] of candidates) {
    if (value.trim()) return { url: normalizeMcpUrl(value), source };
  }
  return { url: DEFAULT_MCP_URL, source: 'default' };
}

type RpcReply = {
  id?: number;
  result?: { content?: { type: string; text?: string }[]; isError?: boolean } & Record<
    string,
    unknown
  >;
  error?: { code: number; message: string };
};

function describe(err: unknown): string {
  const cause = (err as { cause?: unknown })?.cause;
  const base = err instanceof Error ? err.message : String(err);
  return cause instanceof Error ? `${base}: ${cause.message}` : base;
}

/** The JSON-RPC answer to request `id`, from a JSON or an SSE body. */
function pickReply(body: string, contentType: string, id: number): RpcReply | undefined {
  if (!contentType.includes('text/event-stream')) return JSON.parse(body) as RpcReply;
  for (const event of body.split(/\r?\n\r?\n/)) {
    const data = event
      .split(/\r?\n/)
      .filter((line) => line.startsWith('data:'))
      .map((line) => line.slice(5).trimStart())
      .join('\n');
    if (!data) continue;
    const msg = JSON.parse(data) as RpcReply;
    if (msg.id === id) return msg;
  }
  return undefined;
}

export class McpClient {
  private session: string | null = null;
  private opening: Promise<void> | null = null;
  private nextId = 1;

  constructor(
    readonly url: string,
    private readonly fetchFn: typeof fetch = fetch,
  ) {}

  private headers(): Record<string, string> {
    return {
      'content-type': 'application/json',
      accept: 'application/json, text/event-stream',
      'mcp-protocol-version': PROTOCOL,
      ...(this.session ? { 'mcp-session-id': this.session } : {}),
    };
  }

  private async post(message: Record<string, unknown>): Promise<Response> {
    let res: Response;
    try {
      res = await this.fetchFn(this.url, {
        method: 'POST',
        headers: this.headers(),
        body: JSON.stringify(message),
        signal: AbortSignal.timeout(TIMEOUT_MS),
      });
    } catch (err) {
      throw new Error(`mcp-server is not reachable at ${this.url} (${describe(err)}). ${SETUP}`);
    }
    if (!res.ok) {
      throw new Error(`no mcp-server answers at ${this.url} (HTTP ${res.status}). ${SETUP}`);
    }
    return res;
  }

  private async request(
    method: string,
    params: Record<string, unknown>,
  ): Promise<RpcReply['result']> {
    const id = this.nextId++;
    const res = await this.post({ jsonrpc: '2.0', id, method, params });
    const reply = pickReply(await res.text(), res.headers.get('content-type') ?? '', id);
    if (method === 'initialize') this.session = res.headers.get('mcp-session-id');
    if (!reply) throw new Error(`mcp-server at ${this.url} sent no answer to ${method}`);
    if (reply.error) throw new Error(`mcp-server ${method}: ${reply.error.message}`);
    return reply.result;
  }

  private open(): Promise<void> {
    this.opening ??= (async () => {
      await this.request('initialize', {
        protocolVersion: PROTOCOL,
        capabilities: {},
        clientInfo: { name: 'iii-plugin', version: '0.2.0' },
      });
      await this.post({ jsonrpc: '2.0', method: 'notifications/initialized' });
    })().catch((err) => {
      this.opening = null;
      throw err;
    });
    return this.opening;
  }

  /** Call a tool; its text is parsed as JSON when it is JSON. */
  async callTool(name: string, args: Record<string, unknown> = {}): Promise<unknown> {
    await this.open();
    const result = await this.request('tools/call', { name, arguments: args });
    const text = (result?.content ?? []).map((c) => c.text ?? '').join('');
    if (result?.isError) throw new Error(`${name}: ${text}`);
    try {
      return JSON.parse(text);
    } catch {
      return text;
    }
  }

  /** End the MCP session, if one was opened. */
  async close(): Promise<void> {
    if (!this.session) return;
    const session = this.session;
    this.session = null;
    await this.fetchFn(this.url, {
      method: 'DELETE',
      headers: { 'mcp-session-id': session },
      signal: AbortSignal.timeout(TIMEOUT_MS),
    }).catch(() => {});
  }
}

/** Profiles and skills, read through mcp-server's tools. */
export class McpDirectory {
  constructor(readonly client: McpClient) {}

  async listAgents(includeHidden = false): Promise<AgentRow[]> {
    const res = await this.client.callTool('iii_list_agents', { include_hidden: true });
    const agents = (
      Array.isArray(res) ? res : ((res as { agents?: AgentRow[] })?.agents ?? [])
    ) as AgentRow[];
    return includeHidden ? agents : agents.filter((a) => !a.hidden);
  }

  async getAgent(id: string): Promise<AgentProfile> {
    return (await this.client.callTool('iii_get_agent', { id })) as AgentProfile;
  }

  async listSkills(filter: { search?: string; prefix?: string } = {}): Promise<SkillRow[]> {
    const res = await this.client.callTool('iii_list_skills', filter);
    return (
      Array.isArray(res) ? res : ((res as { skills?: SkillRow[] })?.skills ?? [])
    ) as SkillRow[];
  }

  async getSkill(id: string): Promise<Skill> {
    const res = await this.client.callTool('iii_get_skill', { id });
    return typeof res === 'string' ? { id, body: res } : ({ id, ...(res as object) } as Skill);
  }

  async status(): Promise<Record<string, unknown>> {
    return (await this.client.callTool('iii_status')) as Record<string, unknown>;
  }

  close(): Promise<void> {
    return this.client.close();
  }
}
