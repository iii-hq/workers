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
import { observeConfigChanges, ReplyPermission } from '../../src/configuration.js';
import { registerExternalReplies } from '../../src/external-replies.js';

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
type Event = { claude_session_id: string; reply: NonNullable<Result['last_reply']> };
type Hooks = {
  added?: AddedHook;
  removed?: RemovedHook;
  match?: MatchPredicate;
  deliver?: DeliverHook;
};

async function makeHost(root: string, confirmed: boolean) {
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
    inventoryOK: true,
    socketOpen: true,
    outputs: [] as string[],
    calls: [] as Request[],
    deliveries: [] as Array<{ observer: string; event: Event }>,
  };
  const functions = new Map<string, (v: unknown) => unknown>();
  const hooks = new Map<string, Hooks>();
  const observers = new Map<string, Observer>();
  const timers = new Set<() => Promise<void>>();
  let configWatch!: { source: ObserveSource; target: string };
  const backend = {
    isOpen: () => host.socketOpen,
    observe(source: ObserveSource, target: string) {
      configWatch = { source, target };
      return { state: 'active', onStateChange() {} };
    },
    registerFunction(fn: string, handler: (v: unknown) => unknown) {
      functions.set(fn, handler);
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
      if (fn === 'state::list' && payload.scope === 'claude_sessions') return host.records;
      if (fn === 'shell::pty::sessions') return { sessions: [] };
      if (fn === 'shell::exec') {
        if ((payload.env as Record<string, string>)?.CLAUDE_CONFIG_DIR !== root) {
          throw new Error('missing synthetic configuration directory');
        }
        if (payload.command === "'fixture-cli' agents --json --all") {
          await host.beforeInventory();
          return {
            exit_code: host.inventoryOK ? 0 : 1,
            stdout: JSON.stringify(cli),
            stderr: 'host diagnostic',
          };
        }
        if (payload.command !== 'python3') throw new Error('unexpected host command');
        await host.beforeRead();
        const r = spawnSync('python3', payload.args as string[], {
          env: { PATH: '/usr/bin:/bin', HOME: root, CLAUDE_CONFIG_DIR: root },
          cwd: root,
          encoding: 'utf8',
          timeout: Number(payload.timeout_ms),
          maxBuffer: 65536,
        });
        host.outputs.push(r.stdout ?? '', r.stderr ?? '');
        return { exit_code: r.status, stdout: r.stdout, stderr: r.stderr };
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
  let permission = new ReplyPermission();
  const refresh = () => permission.refresh(iii);
  // An enabled seed for ordinary inventory never authorizes the reply surface.
  const agents = registerAgents(
    iii,
    () => ({
      executable: 'fixture-cli',
      env: { CLAUDE_CONFIG_DIR: root },
      args: [],
      detail: '',
      workspace: '',
    }),
    () => true,
    { has: () => false },
  );
  const register = () =>
    registerExternalReplies(iii, permission, agents.replyRow, (f) => {
      timers.add(f);
      return () => timers.delete(f);
    });
  register();
  if (!confirmed) host.config = async () => null;
  await observeConfigChanges(
    iii,
    async () => {
      await refresh();
    },
    (active) => {
      if (!active) permission.invalidate();
    },
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
      permission = new ReplyPermission();
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
) {
  const root = await mkdtemp(join(tmpdir(), 'claude-synthetic-'));
  try {
    return await run(await makeHost(root, confirmed));
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}
