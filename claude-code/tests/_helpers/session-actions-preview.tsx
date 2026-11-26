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
const handlers = new Map<string, (event: unknown) => void>();
const watches = new Set<{
  source: { function_id: string; args?: { claude_session_id?: string } };
  target: string;
}>();
let allowed = true;
let sequence = 1;
const reply = () => ({
  id: `reply-${sequence}`,
  text:
    sequence === 1
      ? 'The layout review is ready.'
      : `Review update ${sequence}: the changes are ready to check.`,
});
const host = {
  iii: {
    browserId: 'fixture',
    on(id: string, fn: (event: unknown) => void) {
      handlers.set(`${id}::fixture`, fn);
      return () => handlers.delete(`${id}::fixture`);
    },
    observe(
      source: string | { function_id: string; args?: { claude_session_id?: string } },
      target: string,
    ) {
      const w = { source: typeof source === 'string' ? { function_id: source } : source, target };
      watches.add(w);
      return () => watches.delete(w);
    },
    addConnectionStateListener: () => () => {},
    async call(
      fn: string,
      p: {
        session?: string;
        claude_session_id?: string;
        mode?: string;
        reopen_held_resume?: boolean;
      },
    ) {
      if (fn === 'claude::agents::list') return { agents: allowed ? rows : [rows[1]] };
      const row = rows.find((r) => r.id === (p.session ?? p.claude_session_id));
      if (!row || (!allowed && (fn === 'claude::agents::get' || !row.pty_session_id)))
        throw new Error('Session unavailable');
      if (fn === 'claude::agents::get') return { ...row, last_reply: reply() };
      if (fn === 'claude::terminal::describe') {
        if (p.mode === 'original' && row.pty_session_id)
          return { pty_session_id: row.pty_session_id };
        if (p.mode === 'original' && row.alive && !row.short_id)
          return { follow: true, claude_session_id: row.id };
        if (p.mode === 'original' && !row.alive && p.reopen_held_resume)
          return { reopen_held_resume: true };
        const args =
          row.short_id && p.mode !== 'fork'
            ? ['attach', row.short_id]
            : ['--resume', row.id, ...(p.mode === 'fork' ? ['--fork-session'] : [])];
        return {
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
function emit(id: string, event: { claude_session_id?: string; reply?: ReturnType<typeof reply> }) {
  for (const w of watches) {
    if (
      w.source.function_id !== id ||
      (w.source.args && w.source.args.claude_session_id !== event.claude_session_id)
    )
      continue;
    // The reply source delivers captured output without an observer_id, so ADE
    // passes the envelope through rather than projecting the output field.
    handlers.get(w.target)?.(id === 'claude::agents::on-reply' ? { output: event } : event);
  }
}
function Preview() {
  const [screen, setScreen] = useState({ pty: '', key: '', text: 'No terminal selected' });
  const [narrow, setNarrow] = useState(false);
  const [count, setCount] = useState(0);
  const firstPtys = useRef(new Map<string, string>());
  const [reopened, setReopened] = useState<{ text: string; reused: boolean } | null>(null);
  const [leases] = useState(() => new Map<string, typeof screen>());
  const terminal: AgentTerminalApi = {
    current: screen.pty || null,
    currentKey: screen.key || null,
    has: (key) => leases.has(key),
    claim: () => {},
    async open({ key, pty, spec }) {
      if (pty === 'fixture-other-console')
        throw new Error('It is open in another console tab or window.');
      const held = leases.get(key);
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
                  : null));
      if (!text) {
        if (!pty && !spec) return false;
        throw new Error('No matching fixture terminal');
      }
      if (!pty && !held) setCount((n) => n + 1);
      const next = { key, pty: pty ?? held?.pty ?? `fixture-${crypto.randomUUID()}`, text };
      // The finished resume stays unlisted by PID, as at a trust prompt.
      // Switch to the console or a fork, then reopen either original without a new PTY.
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
      emit('claude::agents::on-reply', { claude_session_id: rows[0].id, reply: reply() });
    }
  };
  // A fixture keyboard shortcut also works while the real modal owns focus.
  window.onkeydown = (e) => {
    if (e.altKey && e.code === 'KeyU') deliver();
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
                emit('configuration::on-updated', {});
                emit('claude::on-config-change', {});
              }}
            >
              Toggle access
            </button>
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
