import { useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import type { AgentTerminalApi } from '@iii-workers/agent-terminal-ui';
import { PageSidebar } from '../../../ade/web/src/components/ui/PageSidebar';
import { TooltipProvider } from '../../../ade/web/src/components/shadcn/tooltip';
import { ExtensionScopeProvider } from '../../../ade/web/src/lib/ui-scope';
import '../../../ade/web/src/index.css';
import '../../ui/styles.css';
import { SessionsRail } from '../../ui/sessions';

const rows = [
  'Design review',
  'Console original',
  'Background indexing',
  'Other console',
  'Finished review',
].map((name, i) => {
  const id = `1234567${i}-1234-4321-8123-000000000001`;
  return {
    id,
    claude_session_id: id,
    name,
    external_only: i !== 1 && i !== 3,
    kind: i === 2 ? 'background' : 'interactive',
    alive: i !== 4,
    status: i === 2 ? 'busy' : i === 4 ? 'done' : 'idle',
    cwd: i === 0 ? '/fixture/external-only' : '/fixture/workspace',
    ...(i === 1 ? { pty_session_id: 'fixture-console' } : {}),
    ...(i === 3 ? { pty_session_id: 'fixture-other-console' } : {}),
    ...(i === 2 ? { short_id: id.slice(0, 8) } : {}),
  };
});
// Headless, finished rows with a conversation: both Message and Fork show.
for (const [i, name] of ['Draft A', 'Draft B'].entries()) {
  const id = `1234567${rows.length + i}-1234-4321-8123-000000000001`;
  rows.push({
    id,
    claude_session_id: id,
    name,
    external_only: false,
    kind: 'headless',
    alive: false,
    status: 'done',
    cwd: '/fixture/workspace',
  });
}
type Source = {
  function_id: string;
  args?: { claude_session_id?: string };
  filters?: { 'input.id'?: { $eq: string } };
  capture?: string[];
};
const handlers = new Map<string, (event: unknown) => void>();
const watches = new Set<{
  source: Source;
  target: string;
  id?: string;
}>();
const configId = 'fixture-claude';
let allowed = true;
let livePermission: boolean | null = true;
let orderingScenario = false;
let holdVisibilityList: Promise<void> | undefined;
let releaseVisibilityList: (() => void) | undefined;
const notifyConfig = () =>
  emit('configuration::on-updated', { id: configId, event_type: 'configuration:updated' });
let visibility = 1;
const externalVisibility = () => (allowed ? `fixture-${visibility}` : null);
type Listing = 'normal' | 'empty' | 'unavailable' | 'partial';
let listing: Listing = 'normal';
const workerRow = {
  id: 'fixture-worker',
  name: 'Worker run',
  kind: 'headless',
  status: 'done',
  alive: false,
};
let sourceAvailable = true;
let backgroundFinished = false;
const currentRow = (row: (typeof rows)[number]) =>
  row === rows[2] && backgroundFinished ? { ...row, alive: false, status: 'done' } : row;

const refuseConfigWatch = new URLSearchParams(window.location.search).has('refuse-config-watch');
const missingForkDirectory = new URLSearchParams(window.location.search).has(
  'missing-fork-directory',
);
const forkFails = new URLSearchParams(window.location.search).has('fork-fails');
// Access off with host sessions the worker left out: every host row hidden and
// none listed (all), hidden below a worker row (worker), or no host session (none).
const hiddenHost = new URLSearchParams(window.location.search).get('hidden-host');
if (hiddenHost) allowed = false;
// Why the worker hid them: the setting is off, or access is not yet confirmed.
const hiddenReason = new URLSearchParams(window.location.search).get('hidden-reason');
// After the first snapshot shows 'First reply', a second reply is written and
// the next cursor read is rejected once.
const retryFollowOnce = new URLSearchParams(window.location.search).has('retry-follow-once');
// Cursor reads of the followed session still to answer with `{retry: true}`.
let rejectCursorReads = 0;
const count = (key: 'followReads' | 'followRejections') => {
  const data = document.documentElement.dataset;
  data[key] = String(Number(data[key] ?? 0) + 1);
};

// Loopback is usually a secure context. Simulate the crypto surface of plain HTTP LAN.
if (new URLSearchParams(window.location.search).has('no-random-uuid'))
  Object.defineProperty(crypto, 'randomUUID', { value: undefined });
let sequence = 1;
const reply = () => ({
  id: `reply-${sequence}`,
  text:
    sequence === 1
      ? 'The layout review is ready.'
      : `Review update ${sequence}: the changes are ready to check.`,
});
type Reply = ReturnType<typeof reply>;
let lastReply: Reply | null = retryFollowOnce ? { id: 'first', text: 'First reply' } : reply();
let offset = 1;
let activeReplies = [{ offset, reply: lastReply }];
let delayAdmission = false;
let afterLookup: (() => void) | undefined;
let holdNextLookup: Promise<void> | undefined;
let releaseHeldLookup: (() => void) | undefined;
// Gates that hold a follow snapshot after it was read, one per read, released in order.
const snapshotGates: Promise<void>[] = [];
const snapshotReleases: (() => void)[] = [];
let holdDescribe: Promise<void> | undefined;
let releaseDescribe: (() => void) | undefined;
const hiddenRows = new Set<string>();
const replyWatchCount = () => {
  document.documentElement.dataset.replyWatches = String(
    [...watches].filter((watch) => watch.source.function_id === 'claude::agents::on-reply').length,
  );
};
const host = {
  iii: {
    browserId: 'fixture',
    on(id: string, fn: (event: unknown) => void) {
      handlers.set(`${id}::fixture`, fn);
      return () => handlers.delete(`${id}::fixture`);
    },
    observe(source: string | Source, target: string, options?: { id?: string }) {
      const w = {
        source: typeof source === 'string' ? { function_id: source } : source,
        target,
        id: options?.id,
      };
      let removed = false;
      const admit = () => {
        if (!removed) watches.add(w);
        replyWatchCount();
      };
      if (refuseConfigWatch && w.source.function_id === 'configuration::on-updated') {
        // Like ADE, return a disposer before the engine decides admission.
        queueMicrotask(() => {
          if (!removed) document.documentElement.dataset.configWatch = 'refused';
        });
      } else if (delayAdmission && w.source.function_id === 'claude::agents::on-reply')
        afterLookup = admit;
      else admit();
      return () => {
        removed = true;
        watches.delete(w);
        replyWatchCount();
      };
    },
    addConnectionStateListener: () => () => {},
    async call(
      fn: string,
      p: {
        session?: string;
        claude_session_id?: string;
        mode?: string;
        cursor?: string;
        include_config_id?: boolean;
        confirm_live_config?: boolean;
        id?: string;
        all?: boolean;
      },
    ) {
      if (fn === 'engine::observers::get') {
        const watch = [...watches].find((w) => w.id === p.id);
        if (!watch) throw new Error('Observer not found');
        return {
          id: watch.id,
          function_id: watch.source.function_id,
          target: { function_id: watch.target },
          state: 'active',
        };
      }
      if (fn === 'claude::agents::visibility') {
        const token = externalVisibility();
        if (!p.confirm_live_config) return { external_visibility: token };
        return token && livePermission === true
          ? { external_visibility: token, live_config_confirmed: true }
          : { external_visibility: null };
      }
      if (fn === 'claude::agents::list') {
        if (holdVisibilityList) {
          document.getElementById('root')?.setAttribute('data-inventory-waiting', 'true');
          await holdVisibilityList;
        }
        if (hiddenHost) {
          return {
            inventory_status: 'available',
            agents: hiddenHost === 'worker' ? [workerRow] : [],
            ...(hiddenReason ? { hidden_reason: hiddenReason } : {}),
            ...(hiddenHost === 'none'
              ? {}
              : {
                  detail: `${rows.length} sessions on the terminal host are hidden; set external_sessions: true to list them`,
                }),
            ...(p.include_config_id ? { config_id: configId, external_visibility: null } : {}),
          };
        }
        const unavailable = listing === 'partial' || listing === 'unavailable';
        return {
          inventory_status: unavailable ? 'unavailable' : 'available',
          ...(unavailable ? { detail: 'synthetic diagnostic at /fixture/private-inventory' } : {}),
          agents:
            listing === 'partial'
              ? [workerRow]
              : listing !== 'normal'
                ? []
                : rows
                    .map((row) =>
                      row === rows[0] && orderingScenario
                        ? { ...row, status: 'waiting', waiting_for: 'permission confirmation' }
                        : currentRow(row),
                    )
                    .filter(
                      (row) =>
                        !hiddenRows.has(row.id) &&
                        (allowed || !row.external_only) &&
                        (p.all || row.kind !== 'background' || row.alive),
                    ),
          ...(p.include_config_id
            ? { config_id: configId, external_visibility: externalVisibility() }
            : {}),
        };
      }
      const listed = rows.find((r) => r.id === (p.session ?? p.claude_session_id));
      const row = listed && currentRow(listed);
      if (!row || !allowed) throw new Error('Session unavailable');
      if (fn === 'claude::agents::follow-snapshot') {
        const hold = holdNextLookup;
        holdNextLookup = undefined;
        if (hold) await hold;
        if (!allowed || !sourceAvailable) throw new Error('Session unavailable');
        if (row.id !== rows[0].id) {
          return {
            ...row,
            last_reply: { id: row.id, text: `Synthetic reply for ${row.name}.` },
            cursor: '1',
            replies: [],
          };
        }
        count('followReads');
        document.documentElement.dataset.followCursor = p.cursor ?? '';
        if (p.cursor !== undefined && rejectCursorReads > 0) {
          rejectCursorReads--;
          count('followRejections');
          return { retry: true };
        }
        if (retryFollowOnce && p.cursor === undefined && lastReply?.id === 'first') {
          window.setTimeout(() => {
            lastReply = { id: 'second', text: 'Second reply' };
            activeReplies.push({ offset: ++offset, reply: lastReply });
            rejectCursorReads = 1;
            emit('claude::agents::on-reply', { claude_session_id: rows[0].id, reply: lastReply });
          }, 50);
        }
        const snapshot = {
          ...row,
          last_reply: lastReply,
          cursor: String(offset),
          replies:
            p.cursor === undefined
              ? []
              : activeReplies
                  .filter((entry) => entry.offset > Number(p.cursor))
                  .map((entry) => entry.reply),
        };
        const gate = snapshotGates.shift();
        if (gate) await gate;
        return snapshot;
      }
      if (fn === 'claude::terminal::describe' && p.mode === 'fork') {
        const held = holdDescribe;
        holdDescribe = undefined;
        if (held) await held;
        if (p.claude_session_id !== row.id) throw new Error('Conversation changed');
        if (forkFails) throw new Error('the terminal host refused the fork');
        if (missingForkDirectory)
          throw new Error(`the folder ${row.cwd} is not reachable on the terminal host`);
        return {
          claude_session_id: row.id,
          program: 'mock-claude',
          cwd: '/fixture',
          env: {},
          args: ['--resume', row.id, '--fork-session'],
          copy: true,
        };
      }
      throw new Error(`Unexpected synthetic call: ${fn}`);
    },
  },
};
function emit(
  id: string,
  event: { id?: string; event_type?: string; claude_session_id?: string; reply?: Reply },
) {
  for (const w of watches) {
    const entry = w.source.filters?.['input.id']?.$eq;
    if (
      w.source.function_id !== id ||
      (entry !== undefined && entry !== event.id) ||
      (w.source.args && w.source.args.claude_session_id !== event.claude_session_id)
    )
      continue;
    // The reply source delivers captured output without an observer_id, so ADE
    // passes the envelope through rather than projecting the output field.
    const delivery =
      w.source.capture?.length === 0
        ? { observer_id: w.target }
        : id === 'claude::agents::on-reply'
          ? { output: event }
          : event;
    handlers.get(w.target)?.(delivery);
  }
}
function Preview() {
  const [screen, setScreen] = useState({ pty: '', key: '', text: 'No terminal selected' });
  const [narrow, setNarrow] = useState(false);
  const [count, setCount] = useState(0);
  const nextPtyId = useRef(0);
  const [openCalls, setOpenCalls] = useState(0);
  const terminal: AgentTerminalApi = {
    current: screen.pty || null,
    currentKey: screen.key || null,
    has: () => false,
    claim: () => {},
    async open({ key, pty, spec }) {
      setOpenCalls((n) => n + 1);
      const args = spec?.args;
      if (pty || args?.at(-3) !== '--resume' || args.at(-1) !== '--fork-session')
        throw new Error('The fixture only supports an explicit fork');
      const copy = ++nextPtyId.current;
      setCount(copy);
      setScreen({
        pty: `fixture-fork-${copy}`,
        key,
        text: `Separate fork terminal · copy-${copy}`,
      });
      return true;
    },
  };
  const changeListing = (value: Listing) => {
    listing = value;
    emit('claude::terminal::activity', {});
  };
  const deliver = () => {
    if (allowed) {
      sequence++;
      lastReply = reply();
      activeReplies.push({ offset: ++offset, reply: lastReply });
      emit('claude::agents::on-reply', { claude_session_id: rows[0].id, reply: lastReply });
    }
  };
  const scenario = (mode: 'stale' | 'sibling' | 'burst') => {
    // Hold the first post-delivery read until all deliveries (or the branch
    // switch) have happened. No dependence on network speed or a timer.
    let release = () => {};
    holdNextLookup = new Promise<void>((resolve) => {
      release = resolve;
    });
    if (mode !== 'burst') {
      lastReply = { id: 'abandoned-reply', text: 'Abandoned branch answer' };
      emit('claude::agents::on-reply', { claude_session_id: rows[0].id, reply: lastReply });
      offset++;
      lastReply = mode === 'stale' ? null : { id: 'sibling-base', text: 'Current branch answer' };
      activeReplies = lastReply ? [{ offset: 1, reply: lastReply }] : [];
    } else {
      for (let i = 1; i <= 21; i++) {
        lastReply = { id: `burst-${i}`, text: `Burst reply ${i}` };
        activeReplies.push({ offset: ++offset, reply: lastReply });
        const event = { claude_session_id: rows[0].id, reply: lastReply };
        emit('claude::agents::on-reply', event);
        if (i === 1) emit('claude::agents::on-reply', event);
      }
    }
    release();
  };
  // Fixture keyboard shortcuts work while the real modal owns focus.
  window.onkeydown = (e) => {
    if (!e.altKey || e.repeat) return;
    if (e.code === 'KeyU') deliver();
    if (e.code === 'KeyC') {
      emit('configuration::on-updated', { id: 'unrelated', event_type: 'configuration:updated' });
    }
    if (e.code === 'KeyR') {
      allowed = false;
      emit('configuration::on-updated', { id: configId, event_type: 'configuration:updated' });
      // The worker's reload is still pending; there is no completion notification.
    }
    if (e.code === 'KeyV') {
      // Model dropped browser observers while the worker gate still receives updates.
      for (const watch of watches) {
        if (
          ['configuration::on-updated', 'claude::on-config-change'].includes(
            watch.source.function_id,
          )
        )
          watches.delete(watch);
      }
      allowed = false;
    }
    if (e.code === 'KeyW') {
      // Change only the synthetic worker snapshot; the browser watch stays active.
      allowed = !allowed;
      visibility++;
    }
    if (e.code === 'KeyG') {
      livePermission = null;
      notifyConfig();
    }
    if (e.code === 'KeyH') {
      orderingScenario = true;
      livePermission = true;
      allowed = true;
      visibility++;
      emit('claude::on-config-change', {});
    }
    if (e.code === 'KeyJ') {
      holdVisibilityList = new Promise<void>((resolve) => {
        releaseVisibilityList = resolve;
      });
      notifyConfig();
    }
    if (e.code === 'KeyK') {
      holdVisibilityList = undefined;
      releaseVisibilityList?.();
    }
    if (e.code === 'KeyS') scenario('stale');
    if (e.code === 'KeyN') scenario('sibling');
    if (e.code === 'KeyA') {
      deliver();
      deliver();
      afterLookup?.();
    }
    if (e.code === 'KeyB') scenario('burst');
    if (e.code === 'KeyL') {
      sourceAvailable = false;
      deliver();
    }
    if (e.code === 'KeyT') {
      holdNextLookup = new Promise<void>((resolve) => {
        releaseHeldLookup = resolve;
      });
      deliver();
    }
    if (e.code === 'KeyY') releaseHeldLookup?.();
    if (e.code === 'KeyP') {
      // Hold the next two follow snapshots after they were read.
      for (let i = 0; i < 2; i++)
        snapshotGates.push(new Promise<void>((resolve) => snapshotReleases.push(resolve)));
    }
    if (e.code === 'KeyQ') snapshotReleases.shift()?.();
    // Reject the next cursor read of the followed session (X), or the next three (Z).
    if (e.code === 'KeyX') rejectCursorReads += 1;
    if (e.code === 'KeyZ') rejectCursorReads += 3;
    if (e.code === 'KeyD') {
      holdDescribe = new Promise<void>((resolve) => {
        releaseDescribe = resolve;
      });
    }
    if (e.code === 'KeyE') releaseDescribe?.();
    if (e.code === 'KeyO' || e.code === 'KeyI') {
      // The forked row leaves the list: an external one (O) or a worker-owned one (I).
      hiddenRows.add(rows[e.code === 'KeyO' ? 0 : 1].id);
      emit('claude::terminal::activity', {});
    }
    if (e.code === 'KeyM') notifyConfig();
    if (e.code === 'KeyF') {
      backgroundFinished = true;
      emit('claude::terminal::activity', {});
    }
  };
  return (
    <TooltipProvider>
      <ExtensionScopeProvider scope="claude">
        <div
          style={{
            display: 'flex',
            flexDirection: 'column',
            height: '100dvh',
            width: narrow ? '390px' : '100%',
            maxWidth: '100%',
          }}
        >
          <header className="flex flex-wrap items-center gap-3 border-b p-2 text-sm">
            <strong>Synthetic controls</strong>
            <label>
              Inventory fixture{' '}
              <select
                defaultValue="normal"
                onChange={(e) => changeListing(e.currentTarget.value as Listing)}
              >
                <option value="normal">Listed sessions</option>
                <option value="empty">Empty</option>
                <option value="unavailable">Unavailable</option>
                <option value="partial">Unavailable with worker run</option>
              </select>
            </label>
            <button type="button" onClick={deliver}>
              Deliver reply (Alt+U)
            </button>
            <button
              type="button"
              onClick={() =>
                window.setTimeout(() => {
                  deliver();
                  deliver();
                }, 5000)
              }
            >
              Deliver two replies after 5 seconds
            </button>
            <button
              type="button"
              onClick={() => {
                allowed = !allowed;
                visibility++;
                emit('configuration::on-updated', {
                  id: configId,
                  event_type: 'configuration:updated',
                });
                emit('claude::on-config-change', {});
              }}
            >
              Toggle access
            </button>
            <button
              type="button"
              onClick={() => {
                delayAdmission = true;
              }}
            >
              Delay reply observer admission
            </button>
            <button
              type="button"
              onClick={() => {
                backgroundFinished = true;
              }}
            >
              Finish background without refreshing
            </button>
            <span>
              While following: Alt+S switches to an empty branch; Alt+B delivers 21 replies.
            </span>
            <button type="button" onClick={() => setNarrow(!narrow)}>
              Resize pane
            </button>
          </header>
          <div data-iii-ui="claude" className="flex min-h-0 flex-1 flex-row-reverse">
            <PageSidebar
              label="Sessions"
              side="right"
              defaultWidth={300}
              minWidth={240}
              maxWidth={440}
              resizable
              collapsible
              narrowBelow={960}
              narrowMode="drawer"
              closeDrawerOnSelect={false}
            >
              <SessionsRail host={host} terminal={terminal} />
            </PageSidebar>
            <main className="min-w-0 flex-1 p-4">
              <h1 className="font-medium">Claude terminal</h1>
              <p>{screen.text}</p>
              <p className="text-sm">Terminal open calls: {openCalls}</p>
              <p className="text-sm">Terminals created: {count}</p>
            </main>
          </div>
        </div>
      </ExtensionScopeProvider>
    </TooltipProvider>
  );
}
const root = document.getElementById('root');
if (root) createRoot(root).render(<Preview />);
