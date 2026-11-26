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
    kind: i === 2 ? 'background' : 'interactive',
    alive: i !== 4,
    status: i === 2 ? 'busy' : i === 4 ? 'done' : 'idle',
    cwd: '/fixture/workspace',
    ...(i === 1 ? { pty_session_id: 'fixture-console' } : {}),
    ...(i === 3 ? { pty_session_id: 'fixture-other-console' } : {}),
    ...(i === 2 ? { short_id: id.slice(0, 8) } : {}),
  };
});
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
}>();
const configId = 'fixture-claude';
let allowed = true;
let finishedPty: { id: string; confirmed: boolean } | undefined;
let mismatchedDescribe = false;
let backgroundFinished = false;
const currentRow = (row: (typeof rows)[number]) =>
  row === rows[2] && backgroundFinished ? { ...row, alive: false, status: 'done' } : row;

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
let lastReply: Reply | null = reply();
let offset = 1;
let activeReplies = [{ offset, reply: lastReply }];
let delayAdmission = false;
let afterLookup: (() => void) | undefined;
let holdNextLookup: Promise<void> | undefined;
const host = {
  iii: {
    browserId: 'fixture',
    on(id: string, fn: (event: unknown) => void) {
      handlers.set(`${id}::fixture`, fn);
      return () => handlers.delete(`${id}::fixture`);
    },
    observe(source: string | Source, target: string) {
      const w = { source: typeof source === 'string' ? { function_id: source } : source, target };
      let removed = false;
      const admit = () => {
        if (!removed) watches.add(w);
      };
      if (delayAdmission && w.source.function_id === 'claude::agents::on-reply')
        afterLookup = admit;
      else admit();
      return () => {
        removed = true;
        watches.delete(w);
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
      },
    ) {
      if (fn === 'claude::agents::list') {
        return {
          agents: allowed ? rows.map(currentRow) : [rows[1]],
          ...(p.include_config_id ? { config_id: configId } : {}),
        };
      }
      const listed = rows.find((r) => r.id === (p.session ?? p.claude_session_id));
      const row = listed && currentRow(listed);
      if (!row || (!allowed && (fn === 'claude::agents::get' || !row.pty_session_id)))
        throw new Error('Session unavailable');
      if (fn === 'claude::agents::get' || fn === 'claude::agents::follow-snapshot') {
        const hold = holdNextLookup;
        holdNextLookup = undefined;
        if (hold) await hold;
        if (!allowed) throw new Error('Session unavailable');
        return {
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
      }
      if (fn === 'claude::terminal::describe') {
        if (p.claude_session_id !== row.id) throw new Error('Conversation changed');
        if (p.mode === 'original' && row.pty_session_id)
          return {
            pty_session_id: row.pty_session_id,
            claude_session_id: mismatchedDescribe ? rows[3].id : row.id,
          };
        if (p.mode === 'original' && row.alive && !row.short_id)
          return { follow: true, claude_session_id: row.id };
        if (p.mode === 'original' && row.id === rows[4].id && finishedPty) {
          if (!finishedPty.confirmed)
            throw new Error('Could not verify which conversation a console terminal is running');
          return { pty_session_id: finishedPty.id, claude_session_id: row.id };
        }
        const args =
          row.alive && row.short_id && p.mode !== 'fork'
            ? ['attach', row.short_id]
            : ['--resume', row.id, ...(p.mode === 'fork' ? ['--fork-session'] : [])];
        return {
          claude_session_id: row.id,
          program: 'mock-claude',
          cwd: '/fixture',
          env: {},
          args,
          copy: args.includes('--fork-session'),
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
  const firstPtys = useRef(new Map<string, string>());
  const [reopened, setReopened] = useState<{ text: string; reused: boolean } | null>(null);
  const [leases] = useState(() => new Map<string, typeof screen>());
  const [openCalls, setOpenCalls] = useState(0);
  const terminal: AgentTerminalApi = {
    current: screen.pty || null,
    currentKey: screen.key || null,
    has: (key) => leases.has(key),
    claim: () => {},
    async open({ key, pty, spec }) {
      setOpenCalls((n) => n + 1);
      if (pty === 'fixture-other-console')
        throw new Error('It is open in another console tab or window.');
      const held = pty ? [...leases.values()].find((lease) => lease.pty === pty) : leases.get(key);
      const args = spec?.args ?? [];
      const text =
        pty === 'fixture-console'
          ? 'Original console PTY'
          : (held?.text ??
            (args.includes('--fork-session')
              ? 'Separate fork terminal'
              : args[0] === 'attach' && args[1] === rows[2].short_id
                ? 'Original background terminal'
                : args[0] === '--resume' && args[1] === rows[4].id
                  ? 'Finished original at workspace trust'
                  : args[0] === '--resume' && args[1] === rows[2].id
                    ? 'Finished background original'
                    : null));
      if (!text) {
        if (!pty && !spec) return false;
        throw new Error('No matching fixture terminal');
      }
      if (!pty && !held) setCount((n) => n + 1);
      const next = { key, pty: pty ?? held?.pty ?? `fixture-${++nextPtyId.current}`, text };
      if (!held && text === 'Finished original at workspace trust')
        finishedPty = { id: next.pty, confirmed: false };
      if (
        text === 'Original background terminal' ||
        text === 'Finished original at workspace trust'
      ) {
        const firstPty = firstPtys.current.get(text);
        if (firstPty) setReopened({ text, reused: next.pty === firstPty });
        else firstPtys.current.set(text, next.pty);
      } else {
        setReopened(null);
      }
      leases.set(key, next);
      setScreen(next);
      return true;
    },
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
    if (e.code === 'KeyS') scenario('stale');
    if (e.code === 'KeyN') scenario('sibling');
    if (e.code === 'KeyA') {
      deliver();
      deliver();
      afterLookup?.();
    }
    if (e.code === 'KeyB') scenario('burst');
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
                if (finishedPty && finishedPty.id !== screen.pty) {
                  const id = finishedPty.id;
                  for (const [key, lease] of leases) if (lease.pty === id) leases.delete(key);
                  finishedPty = undefined;
                }
              }}
            >
              Expire detached finished PTY
            </button>
            <button
              type="button"
              onClick={() => {
                if (finishedPty) finishedPty.confirmed = true;
              }}
            >
              Confirm finished PTY identity
            </button>
            <button
              type="button"
              onClick={() => {
                mismatchedDescribe = true;
              }}
            >
              Return mismatched terminal identity
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
              {reopened?.text === screen.text ? (
                <p className="text-sm">
                  {screen.text === 'Original background terminal'
                    ? 'Background attachment'
                    : 'Finished resume'}
                  {' on reopen: '}
                  {reopened.reused ? 'reused' : 'replaced'}
                </p>
              ) : null}
            </main>
          </div>
        </div>
      </ExtensionScopeProvider>
    </TooltipProvider>
  );
}
const root = document.getElementById('root');
if (root) createRoot(root).render(<Preview />);
