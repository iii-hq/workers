import { spawnSync } from 'node:child_process';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import type {
  AddedHook,
  Call,
  DeliverHook,
  IIIClient,
  MatchPredicate,
  ObserveSource,
  Observer,
  RemovedHook,
} from 'iii-sdk';
import { type CliRow, registerAgents } from '../../src/agents.js';
import { TerminalConfigSchema } from '../../src/config.js';
import { observeReplyConfig, ReplyPermission } from '../../src/configuration.js';
import { registerExternalReplies } from '../../src/external-replies.js';
import { registerTerminal } from '../../src/terminal/terminal.js';

export const id = '11111111-1111-4111-8111-111111111111';
export const recordId = (n: number) => `abcdef01-0000-4000-8000-${String(n).padStart(12, '0')}`;
export function record(n: number, parent: number | null, type: string, content: unknown) {
  return {
    uuid: recordId(n),
    parentUuid: parent === null ? null : recordId(parent),
    type,
    message: { role: type, content },
  };
}

export function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { resolve, promise };
}

const EVENT = 'claude::agents::on-reply';
type Request = {
  function_id: string;
  payload: Record<string, unknown>;
  metadata?: Call['metadata'];
};
type Result = {
  claude_session_id: string;
  name: string | null;
  cwd: string | null;
  status: string;
  last_reply: { id: string; text: string } | null;
};
type ListResult = { agents: Array<{ claude_session_id?: string }> };
type Event = { claude_session_id: string; reply: NonNullable<Result['last_reply']> };
type Handler = (value: unknown) => unknown;
type WrapHandler = (fn: string, handler: Handler) => Handler;
type Hooks = {
  added?: AddedHook;
  removed?: RemovedHook;
  match?: MatchPredicate;
  deliver?: DeliverHook;
};

async function makeHost(root: string, confirmed: boolean, wrapHandler?: WrapHandler) {
  const projects = join(root, 'projects');
  const project = join(projects, 'synthetic');
  const file = join(project, `${id}.jsonl`);
  await mkdir(project, { recursive: true });
  await writeFile(file, `${JSON.stringify(record(1, null, 'assistant', 'fixture answer'))}\n`);
  const cli: CliRow[] = [
    { kind: 'interactive', sessionId: id, status: 'idle', name: 'fixture', cwd: '/fixture' },
  ];
  const host = {
    cli,
    records: [] as Array<Record<string, unknown>>,
    config: async (): Promise<unknown> => ({ external_sessions: true }),
    beforeRead: async () => {},
    beforeDelivery: async () => {},
    beforeInventory: async () => {},
    beforePrepare: async () => {},
    inventoryOK: true,
    socketOpen: true,
    connectionEpoch: 1,
    configWatchAnswered: true,
    outputs: [] as string[],
    snapshots: [] as string[],
    hostOutput: (stdout: string) => stdout,
    calls: [] as Request[],
    deliveries: [] as Array<{ observer: string; event: Event }>,
  };
  const functions = new Map<string, Handler>();
  const hooks = new Map<string, Hooks>();
  const observers = new Map<string, Observer>();
  const timers = new Set<() => Promise<void>>();
  let configWatch!: { source: ObserveSource; target: string };
  const backend = {
    isOpen: () => host.socketOpen,
    observeOnline: () => host.socketOpen,
    get connectionEpoch() {
      return host.connectionEpoch;
    },
    observe(source: ObserveSource, target: string) {
      configWatch = { source, target };
      return {
        state: 'active',
        // SDK 0.27 clears `answered` when replaying but does not notify
        // state listeners when the new acknowledgement is also active.
        get answered() {
          return host.configWatchAnswered;
        },
        onStateChange() {},
      };
    },
    registerFunction(fn: string, handler: Handler) {
      functions.set(fn, wrapHandler?.(fn, handler) ?? handler);
      const h: Hooks = {};
      hooks.set(fn, h);
      const ref = {
        onObserverAdded(f: AddedHook) {
          h.added = f;
          return ref;
        },
        onObserverRemoved(f: RemovedHook) {
          h.removed = f;
          return ref;
        },
        onObservationMatch(f: MatchPredicate) {
          h.match = f;
          return ref;
        },
        onObservationDeliver(f: DeliverHook) {
          h.deliver = f;
          return ref;
        },
      };
      return ref;
    },
    async call(
      request: Request | { observer: Observer; payload: { output: Event } },
    ): Promise<unknown> {
      if ('observer' in request) {
        host.deliveries.push({ observer: request.observer.id, event: request.payload.output });
        return;
      }
      host.calls.push(request);
      const { function_id: fn, payload } = request;
      if (fn === 'configuration::get') return { value: await host.config() };
      if (fn === 'state::list') {
        if (payload.scope === 'claude_sessions') return host.records;
        if (payload.scope === 'claude_turns') return [];
      }
      if (fn === 'shell::pty::sessions') return { sessions: [] };
      if (fn === 'compose::list') return { projects: [] };
      if (fn === 'http::status') return { url: 'http://127.0.0.1:3111' };
      if (fn === 'engine::functions::list') {
        return { functions: [{ function_id: 'mcp-server::handler' }] };
      }
      if (fn === 'shell::exec') {
        if (payload.command === 'pwd' && JSON.stringify(payload.args) === '["-P"]') {
          await host.beforePrepare();
          return { exit_code: 0, stdout: `${payload.cwd}\n`, stderr: '' };
        }
        const env = payload.env as Record<string, string>;
        if (env?.CLAUDE_CONFIG_DIR !== root) {
          throw new Error('missing synthetic configuration directory');
        }
        if (
          ["'fixture-cli' agents --json --all", "'fixture-cli' agents --json"].includes(
            String(payload.command),
          )
        ) {
          await host.beforeInventory();
          return {
            exit_code: host.inventoryOK ? 0 : 1,
            stdout: JSON.stringify(cli),
            stderr: 'host diagnostic',
          };
        }
        // Record the request only. Never invoke a real CLI action.
        if (payload.command === "'fixture-cli' stop deadbeef")
          return { exit_code: 0, stdout: '', stderr: '' };
        if (payload.command !== 'python3') throw new Error('unexpected host command');
        await host.beforeRead();
        const r = spawnSync('python3', payload.args as string[], {
          env: { ...env, PATH: '/usr/bin:/bin', HOME: root },
          cwd: root,
          encoding: 'utf8',
          timeout: Number(payload.timeout_ms),
          maxBuffer: 128 * 1024,
        });
        host.outputs.push(r.stdout ?? '', r.stderr ?? '');
        if (r.status === 0 && r.stdout) {
          // Inspect the actual host boundary, not the lookup's filtered result.
          const pad = Buffer.from(env.CLAUDE_REPLY_SECRET_PAD, 'base64');
          const ciphertext = Buffer.from(r.stdout, 'base64').subarray(32);
          host.snapshots.push(ciphertext.map((byte, i) => byte ^ pad[32 + i]).toString('utf8'));
        }
        return { exit_code: r.status, stdout: host.hostOutput(r.stdout ?? ''), stderr: r.stderr };
      }
      const handler = functions.get(fn);
      if (!handler) throw new Error(`unexpected function: ${fn}`);
      const output = await handler(payload);
      if (fn === EVENT) {
        const call = {
          function_id: fn,
          status: 'success',
          input: payload,
          output,
          metadata: request.metadata,
        } as Call;
        const h = hooks.get(fn);
        const matched = [...observers.values()].filter((obs) => h?.match?.(obs, call));
        // The SDK snapshots matches before delivery. Its default reporter can
        // flush them later without calling the match predicate a second time.
        await host.beforeDelivery();
        if (h?.deliver) return h.deliver(call, matched);
        for (const obs of matched)
          host.deliveries.push({ observer: obs.id, event: output as Event });
      }
      return output;
    },
  };
  const iii = backend as unknown as IIIClient;
  const permission = new ReplyPermission();
  const refresh = () => permission.refresh(iii);
  let external = true;
  // An enabled legacy setting never authorizes live inventory or reply disclosure.
  const agents = registerAgents(
    iii,
    () => ({
      executable: 'fixture-cli',
      env: { CLAUDE_CONFIG_DIR: root },
      args: [],
      detail: '',
      workspace: '',
    }),
    { has: () => false },
    permission,
  );
  const register = () =>
    registerExternalReplies(iii, permission, agents.replyRow, (f) => {
      timers.add(f);
      return () => timers.delete(f);
    });
  register();
  if (!confirmed) host.config = async () => null;
  await observeReplyConfig(iii, permission, (runtime) => {
    external = runtime?.external_sessions ?? true;
  });
  registerTerminal(
    iii,
    () => ({ home: root, executable: 'fixture-cli', env: {}, detail: '', bridge: '' }),
    () => TerminalConfigSchema.parse({ setup_workspace: false }),
    agents.resolveOpen,
    agents.openVisibility,
  );
  const client = {
    call: (r: Request) => backend.call(r) as Promise<Result>,
    async observe(fn: string, key: string, session: string) {
      const accepted = deferred<void>();
      const obs = {
        id: key,
        args: { claude_session_id: session },
        accepted: () => accepted.promise,
        capture: ['output'],
        matches: () => true,
      } as Observer;
      const h = hooks.get(fn);
      if (!h?.added) throw new Error('not an observer source');
      await h.added(obs);
      observers.set(key, obs);
      accepted.resolve();
      return async () => {
        observers.delete(key);
        await h.removed?.(obs);
      };
    },
  };
  return {
    host,
    client,
    refresh,
    get externalSetting() {
      return external;
    },
    list: () =>
      backend.call({
        function_id: 'claude::agents::list',
        payload: { all: true },
      }) as Promise<ListResult>,
    fork: () =>
      backend.call({
        function_id: 'claude::terminal::describe',
        payload: { session: id, mode: 'fork', claude_session_id: id },
      }),
    root,
    project,
    projects,
    file,
    claude_session_id: id,
    async configEvent(event_type: string) {
      const input = { id: process.env.III_CONFIG_NAME?.trim() || 'claude-code', event_type };
      const { source, target } = configWatch;
      if (source.function_id !== 'configuration::on-updated') return;
      for (const [key, value] of Object.entries(input)) {
        const filter = source.filters?.[`input.${key}`] as { $eq?: string } | undefined;
        if (filter && filter.$eq !== value) return;
      }
      await functions.get(target)?.({ output: input });
    },
    async restart() {
      timers.clear();
      observers.clear();
      permission.invalidate();
      register();
      await refresh();
    },
    async poll() {
      for (const f of [...timers]) {
        timers.delete(f);
        await f();
      }
    },
    timers,
  };
}
export type SyntheticHost = Awaited<ReturnType<typeof makeHost>>;

/** No worker, Vitest globals, personal directories, or live engine required. */
export async function withSyntheticHost<T>(
  run: (s: SyntheticHost) => Promise<T>,
  confirmed = true,
  wrapHandler?: WrapHandler,
) {
  const root = await mkdtemp(join(tmpdir(), 'claude-synthetic-'));
  const originalTracePolicy = process.env.III_DISABLE_TRACE_PAYLOADS;
  try {
    return await run(await makeHost(root, confirmed, wrapHandler));
  } finally {
    if (originalTracePolicy === undefined) delete process.env.III_DISABLE_TRACE_PAYLOADS;
    else process.env.III_DISABLE_TRACE_PAYLOADS = originalTracePolicy;
    await rm(root, { recursive: true, force: true });
  }
}
