/**
 * Which engine the directory plugin talks to.
 *
 * The plugin runs in two places: inside a session this worker spawned (the
 * worker exports `III_URL`, so the answer is already known), and inside a plain
 * `claude` a person started on their own machine, where nothing told it
 * anything. The second case is why auto-detection exists: an engine on the
 * standard port should just work, and anything else is one setting away.
 *
 * Resolution order, first hit wins:
 *   1. `--url <ws://...>` on the server's command line
 *   2. `III_DIRECTORY_PLUGIN_URL` (this plugin only)
 *   3. `CLAUDE_PLUGIN_OPTION_ENGINE_URL` (the plugin's `engine_url` user option)
 *   4. `III_URL`, then `III_ENGINE_URL` (shared with every iii worker)
 *   5. auto-detect: probe each port x host until one accepts a TCP connection
 *
 * An explicit address is never second-guessed by probing: if it is down, the
 * error says so, rather than silently attaching to a different engine.
 */

import { connect } from 'node:net';

/** The engine's standard WebSocket port, then the ones a second local engine
 *  (a dev checkout, a worktree) usually lands on. */
export const STANDARD_PORTS = [49134, 49135, 49136];
export const STANDARD_HOSTS = ['127.0.0.1', 'localhost'];

export type EngineSource =
  | 'flag'
  | 'plugin-env'
  | 'plugin-option'
  | 'III_URL'
  | 'III_ENGINE_URL'
  | 'auto-detect';

export type Resolved =
  | { url: string; source: EngineSource }
  | { url: ''; source: 'none'; detail: string };

export type ResolveOptions = {
  argv?: string[];
  env?: NodeJS.ProcessEnv;
  /** Probe override, for tests. Resolves true when something listens. */
  probe?: (host: string, port: number, timeoutMs: number) => Promise<boolean>;
  timeoutMs?: number;
};

function flag(argv: string[]): string {
  const i = argv.indexOf('--url');
  if (i >= 0 && argv[i + 1]) return argv[i + 1];
  const eq = argv.find((a) => a.startsWith('--url='));
  return eq ? eq.slice('--url='.length) : '';
}

/** Accept `host:port`, `:port`, or a bare port as shorthand for a ws:// URL.
 *  Unexpanded `${...}` placeholders (an unset plugin option) count as empty. */
export function normalizeUrl(raw: string): string {
  const value = raw.trim();
  if (!value || value.includes('${')) return '';
  if (/^wss?:\/\//i.test(value)) return value;
  if (/^\d+$/.test(value)) return `ws://127.0.0.1:${value}`;
  if (value.startsWith(':')) return `ws://127.0.0.1${value}`;
  return `ws://${value}`;
}

function list(raw: string | undefined, fallback: string[]): string[] {
  const items = (raw ?? '')
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean);
  return items.length ? items : fallback;
}

export function tcpProbe(host: string, port: number, timeoutMs: number): Promise<boolean> {
  return new Promise((resolve) => {
    const socket = connect({ host, port });
    const done = (ok: boolean) => {
      socket.removeAllListeners();
      socket.destroy();
      resolve(ok);
    };
    socket.setTimeout(timeoutMs, () => done(false));
    socket.once('connect', () => done(true));
    socket.once('error', () => done(false));
  });
}

export async function resolveEngine(options: ResolveOptions = {}): Promise<Resolved> {
  const env = options.env ?? process.env;
  const explicit: [EngineSource, string | undefined][] = [
    ['flag', flag(options.argv ?? process.argv.slice(2))],
    ['plugin-env', env.III_DIRECTORY_PLUGIN_URL],
    ['plugin-option', env.CLAUDE_PLUGIN_OPTION_ENGINE_URL],
    ['III_URL', env.III_URL],
    ['III_ENGINE_URL', env.III_ENGINE_URL],
  ];
  for (const [source, raw] of explicit) {
    const url = normalizeUrl(raw ?? '');
    if (url) return { url, source };
  }

  const probe = options.probe ?? tcpProbe;
  const timeoutMs = options.timeoutMs ?? 400;
  const hosts = list(env.III_DIRECTORY_PLUGIN_HOSTS, STANDARD_HOSTS);
  const ports = list(env.III_DIRECTORY_PLUGIN_PORTS, STANDARD_PORTS.map(String))
    .map(Number)
    .filter((p) => Number.isInteger(p) && p > 0 && p < 65536);
  const tried: string[] = [];
  for (const port of ports) {
    for (const host of hosts) {
      tried.push(`${host}:${port}`);
      if (await probe(host, port, timeoutMs)) {
        return { url: `ws://${host}:${port}`, source: 'auto-detect' };
      }
    }
  }
  return {
    url: '',
    source: 'none',
    detail: `no iii engine found (tried ${tried.join(', ')}); start one with \`iii\`, or set III_URL or the plugin's engine_url option`,
  };
}
