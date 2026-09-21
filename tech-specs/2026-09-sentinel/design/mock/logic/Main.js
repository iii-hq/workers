class Component extends DCLogic {
  /* ===================== seed data ===================== */

  groups() {
    if (this._groups) return this._groups
    const G = [
      {
        id: 'cas', type: 'CasMismatch', msg: 'expected version <n>, found <n>',
        worker: 'state', fn: 'state::compare-and-set', source: 'trace', status: 'regressed',
        verFirst: '0.22.0', verLast: '0.23.0', resolvedVer: '0.22.1', seed: 7, hot: 6,
        count: 1284, sessions: 39,
        first: '14 d ago', firstAbs: '2026-09-02 09:14', last: '12 s ago', lastAbs: '2026-09-16 14:02:07',
        triage: {
          hyp: 'A retry that reuses a stale state version — always off by one, always after a re-enqueue.',
          look: ['harness/src/turn_loop.rs', 'state/src/store.rs'], cat: 'likely bug',
        },
        span: {
          name: 'execute state::compare-and-set', fn: 'state::compare-and-set', svc: 'state · 0.23.0',
          status: 'CasMismatch', dur: '4.2 ms', trace: '7e2b9c04a1d3…f08e',
          session: 'promptlab-real-t3-qv5', turn: 't_8f21c3a0', tag: 'step', prop: '2 spans',
          exType: 'CasMismatch',
          exMsg: 'expected version 41, found 42 (scope=harness key=session:s_7a1:turn)',
          stack: 'state::store::compare_and_set (src/store.rs:214)\n                     state::functions::cas::handle (src/functions/cas.rs:58)\n                     iii_sdk::handler::execute (handler.rs:131)',
        },
        path: [
          { d: 0, tone: 'alert', name: 'execute harness::send', svc: 'harness', dur: '1.24 s' },
          { d: 1, tone: 'alert', name: 'harness::turn step', svc: 'harness', tag: 'propagated', dur: '1.19 s' },
          { d: 2, tone: 'ok', name: 'call router::chat', svc: 'iii → llm-router', dur: '1.02 s' },
          { d: 2, tone: 'alert', name: 'call state::compare-and-set', svc: 'iii', tag: 'propagated', dur: '5.1 ms' },
          { d: 3, tone: 'alert', name: 'execute state::compare-and-set', svc: 'state', origin: true, dur: '4.2 ms' },
          { d: 2, tone: 'ok', name: 'session::append', svc: 'session-manager', dur: '2.3 ms' },
        ],
        logs: [
          { t: '14:02:07.118', lvl: 'ERROR', svc: 'state', body: 'cas mismatch scope=harness key=session:s_7a1:turn expected=41 found=42' },
          { t: '14:02:07.120', lvl: 'WARN', svc: 'harness', body: 'turn step failed; re-enqueueing (attempt 2/3)' },
          { t: '14:02:07.402', lvl: 'ERROR', svc: 'harness', body: 'turn t_8f21c3a0 failed: state conflict after 3 attempts' },
        ],
        occ: [
          { at: '12 s ago', ver: '0.23.0', session: 'promptlab-real-t3-qv5', turn: 't_8f21c3a0', ev: 'full' },
          { at: '3 min ago', ver: '0.23.0', session: 'promptlab-min-t4-qv4', turn: 't_1c90ffab', ev: 'full' },
          { at: '11 min ago', ver: '0.23.0', session: 'fact-nine-p8r1', turn: 't_44b27e01', ev: 'full' },
          { at: '2 h ago', ver: '0.22.1', session: 'promptlab-base-t3-qv4', turn: 't_7fe3a220', ev: 'none' },
          { at: '14 d ago', ver: '0.22.0', session: 'promptlab-real-t1-qv5', turn: 't_0093ac1e', ev: 'full' },
        ],
        hist: [
          { at: '12 s ago', kind: 'alert', what: 'Regressed', note: 'first occurrence on 0.23.0' },
          { at: '3 d ago', kind: 'ok', what: 'Resolved', note: 'until version change · on 0.22.1' },
          { at: '3 d ago', kind: 'ink', what: 'Diagnosed', note: 'confidence high' },
          { at: '14 d ago', kind: 'ghost', what: 'First seen', note: 'state 0.22.0' },
        ],
      },
      {
        id: 'rate', type: 'RateLimitError', msg: "429 rate_limit_error: this request would exceed your organization's rate limit",
        worker: 'provider-anthropic', fn: 'router::chat', source: 'trace', status: 'new',
        verFirst: '0.23.0', verLast: '0.23.0', seed: 42, hot: 0, count: 212, sessions: 18,
        first: '3 h ago', firstAbs: '2026-09-16 11:07', last: '41 s ago', lastAbs: '2026-09-16 14:01:38',
        triage: {
          hyp: 'Upstream throttling, not a defect — but nothing backs off, so every turn retries into the same wall.',
          look: ['provider-anthropic/src/chat.rs', 'llm-router/src/retry.rs'], cat: 'likely transient',
        },
        span: {
          name: 'execute router::chat', fn: 'router::chat', svc: 'provider-anthropic · 0.23.0',
          status: 'RateLimitError', dur: '210 ms', trace: 'b41c0e77d9a2…31f5',
          session: 'promptlab-min-t3-qv4', turn: 't_5521ba90', tag: 'generate', prop: '1 span',
          exType: 'RateLimitError',
          exMsg: "429 rate_limit_error: this request would exceed your organization's rate limit (retry-after 12s)",
          stack: 'provider_anthropic::chat::send (src/chat.rs:188)\n                     llm_router::route::generate (src/route.rs:96)',
        },
        path: [
          { d: 0, tone: 'alert', name: 'execute harness::send', svc: 'harness', dur: '0.4 s' },
          { d: 1, tone: 'alert', name: 'call router::chat', svc: 'iii', tag: 'propagated', dur: '0.3 s' },
          { d: 2, tone: 'alert', name: 'execute router::chat', svc: 'provider-anthropic', origin: true, dur: '210 ms' },
        ],
        logs: [
          { t: '14:01:38.004', lvl: 'ERROR', svc: 'provider-anthropic', body: 'anthropic 429 rate_limit_error retry_after=12' },
          { t: '14:01:38.006', lvl: 'WARN', svc: 'llm-router', body: 'no fallback provider configured for claude-sonnet-5' },
        ],
        occ: [
          { at: '41 s ago', ver: '0.23.0', session: 'promptlab-min-t3-qv4', turn: 't_5521ba90', ev: 'full' },
          { at: '1 min ago', ver: '0.23.0', session: 'promptlab-min-t4-qv4', turn: 't_49ca01bd', ev: 'full' },
          { at: '4 min ago', ver: '0.23.0', session: 'fact-seven-x7p2', turn: 't_2ab7cc31', ev: 'none' },
        ],
        hist: [
          { at: '41 s ago', kind: 'ghost', what: 'Last occurrence', note: '212 in 3 h' },
          { at: '3 h ago', kind: 'ghost', what: 'First seen', note: 'provider-anthropic 0.23.0' },
        ],
      },
      {
        id: 'contract', type: 'ContractError', msg: 'output did not match schema: missing "summary"',
        worker: 'harness', fn: 'harness::turn', source: 'trace', status: 'new',
        verFirst: '0.23.0', verLast: '0.23.0', seed: 311, hot: 0, count: 37, sessions: 9,
        first: '2 d ago', firstAbs: '2026-09-14 08:22', last: '6 min ago', lastAbs: '2026-09-16 13:56:12',
        triage: {
          hyp: 'A JSON output contract the model answers in prose when the turn runs out of room.',
          look: ['harness/src/types/output.rs', 'harness/src/turn_loop.rs'], cat: 'likely bug',
        },
        span: {
          name: 'harness::turn step', fn: 'harness::turn', svc: 'harness · 0.23.0',
          status: 'ContractError', dur: '18.4 s', trace: '2d77af10bb43…90c2',
          session: 'security-scan-analysis-9f2', turn: 't_a01f77de', tag: 'step', prop: '1 span',
          exType: 'ContractError',
          exMsg: 'output did not match schema: missing "summary" at /',
          stack: 'harness::types::output::validate (src/types/output.rs:142)\n                     harness::turn_loop::finalize (src/turn_loop.rs:688)',
        },
        path: [
          { d: 0, tone: 'alert', name: 'execute harness::send', svc: 'harness', dur: '19.1 s' },
          { d: 1, tone: 'alert', name: 'harness::turn step', svc: 'harness', origin: true, dur: '18.4 s' },
          { d: 2, tone: 'ok', name: 'call router::chat', svc: 'iii → llm-router', dur: '17.9 s' },
        ],
        logs: [
          { t: '13:56:12.771', lvl: 'ERROR', svc: 'harness', body: 'turn t_a01f77de failed: output contract violation (json)' },
        ],
        occ: [
          { at: '6 min ago', ver: '0.23.0', session: 'security-scan-analysis-9f2', turn: 't_a01f77de', ev: 'full' },
          { at: '1 h ago', ver: '0.23.0', session: 'security-scan-analysis-8b1', turn: 't_66d0c410', ev: 'full' },
          { at: '2 d ago', ver: '0.23.0', session: 'eval-run-77', turn: 't_1180ff3a', ev: 'none' },
        ],
        hist: [
          { at: '6 min ago', kind: 'ghost', what: 'Last occurrence', note: '37 in 2 d' },
          { at: '2 d ago', kind: 'ghost', what: 'First seen', note: 'harness 0.23.0' },
        ],
      },
      {
        id: 'timeout', type: 'TimeoutError', msg: 'vector store timed out after <n>ms',
        worker: 'memory', fn: 'memory::recall', source: 'trace', status: 'new',
        verFirst: '0.2.0', verLast: '0.2.0', seed: 98, hot: 0, count: 19, sessions: 7,
        first: '5 h ago', firstAbs: '2026-09-16 09:30', last: '22 min ago', lastAbs: '2026-09-16 13:40:55',
        triage: {
          hyp: 'The vector store is slower than the recall timeout under concurrent turns.',
          look: ['memory/src/recall.rs', 'memory/src/config.rs'], cat: 'likely configuration',
        },
        span: {
          name: 'execute memory::recall', fn: 'memory::recall', svc: 'memory · 0.2.0',
          status: 'TimeoutError', dur: '120 ms', trace: 'ac19bb4e0f72…7731',
          session: 'promptlab-base-t4-qv4', turn: 't_31c7d9aa', tag: 'hook', prop: '1 span',
          exType: 'TimeoutError', exMsg: 'vector store timed out after 120ms',
          stack: 'memory::recall::search (src/recall.rs:96)\n                     memory::hooks::pre_generate (src/hooks.rs:41)',
        },
        path: [
          { d: 0, tone: 'alert', name: 'harness::turn step', svc: 'harness', dur: '2.1 s' },
          { d: 1, tone: 'alert', name: 'call memory::recall', svc: 'iii', tag: 'propagated', dur: '124 ms' },
          { d: 2, tone: 'alert', name: 'execute memory::recall', svc: 'memory', origin: true, dur: '120 ms' },
        ],
        logs: [
          { t: '13:40:55.310', lvl: 'ERROR', svc: 'memory', body: 'recall timed out bank=default k=8 elapsed=120ms' },
        ],
        occ: [
          { at: '22 min ago', ver: '0.2.0', session: 'promptlab-base-t4-qv4', turn: 't_31c7d9aa', ev: 'full' },
          { at: '1 h ago', ver: '0.2.0', session: 'promptlab-base-t3-qv4', turn: 't_09b1c7f0', ev: 'full' },
        ],
        hist: [
          { at: '22 min ago', kind: 'ghost', what: 'Last occurrence', note: '19 in 5 h' },
          { at: '5 h ago', kind: 'ghost', what: 'First seen', note: 'memory 0.2.0' },
        ],
      },
      {
        id: 'conflict', type: 'ConflictError', msg: 'message <id> already exists in session <id>',
        worker: 'session-manager', fn: 'session::append', source: 'trace', status: 'diagnosed',
        verFirst: '0.23.0', verLast: '0.23.0', seed: 1204, hot: 0, count: 8, sessions: 8,
        first: '1 d ago', firstAbs: '2026-09-15 10:02', last: '3 h ago', lastAbs: '2026-09-16 11:14:20',
        triage: {
          hyp: 'Two writers append the same entry id — probably a redelivered queue message.',
          look: ['session-manager/src/store.rs', 'harness/src/queue.rs'], cat: 'likely bug',
        },
        span: {
          name: 'execute session::append', fn: 'session::append', svc: 'session-manager · 0.23.0',
          status: 'ConflictError', dur: '3.1 ms', trace: '55c0192ef8ab…2d4e',
          session: 'promptlab-real-t1-qv5', turn: 't_7710aa02', tag: 'step', prop: '1 span',
          exType: 'ConflictError',
          exMsg: 'message msg-7f21 already exists in session s_7a1',
          stack: 'session_manager::store::append (src/store.rs:301)\n                     iii_sdk::handler::execute (handler.rs:131)',
        },
        path: [
          { d: 0, tone: 'alert', name: 'harness::turn step', svc: 'harness', dur: '0.9 s' },
          { d: 1, tone: 'alert', name: 'call session::append', svc: 'iii', tag: 'propagated', dur: '3.4 ms' },
          { d: 2, tone: 'alert', name: 'execute session::append', svc: 'session-manager', origin: true, dur: '3.1 ms' },
        ],
        logs: [
          { t: '11:14:20.880', lvl: 'ERROR', svc: 'session-manager', body: 'append conflict session=s_7a1 entry=msg-7f21' },
          { t: '11:14:20.877', lvl: 'WARN', svc: 'queue', body: 'redelivering harness-turn message after engine restart' },
        ],
        occ: [
          { at: '3 h ago', ver: '0.23.0', session: 'promptlab-real-t1-qv5', turn: 't_7710aa02', ev: 'full' },
          { at: '9 h ago', ver: '0.23.0', session: 'fact-cube-p7b8', turn: 't_5c2b3a71', ev: 'full' },
          { at: '1 d ago', ver: '0.23.0', session: 'new chat', turn: 't_2201cbb4', ev: 'none' },
        ],
        hist: [
          { at: '2 h ago', kind: 'ink', what: 'Diagnosed', note: 'confidence medium · claude-sonnet-5' },
          { at: '2 h ago', kind: 'accent', what: 'Investigation started', note: 'assisted · you' },
          { at: '1 d ago', kind: 'ghost', what: 'First seen', note: 'session-manager 0.23.0' },
        ],
        diag: {
          v: 1, source: 'first pass', when: '2 h ago', model: 'claude-sonnet-5',
          turns: '5 turns · 1 min 48 s', conf: 'medium', cat: 'bug', risk: 'low',
          summary: 'A queue redelivery replays a turn step that already appended its entry, and the append is keyed by a deterministic id, so the second write collides instead of being a no-op.',
          cause: 'The harness derives the entry id from the turn id, which is stable across redeliveries. `session::append` treats a duplicate id as a conflict rather than an idempotent no-op, so every redelivery after an engine restart surfaces as an error.',
          ev: [
            { kind: 'code', path: 'session-manager/src/store.rs:301', code: 'if self.entries.contains_key(&entry.id) {\n    return Err(ConflictError::new(&entry.id));\n}', why: 'The duplicate path is an error, with no equality check against the stored entry.' },
            { kind: 'log', path: 'queue · 11:14:20.877', code: 'redelivering harness-turn message after engine restart', why: 'Every sampled occurrence is preceded by a redelivery in the same trace.' },
          ],
          fix: 'Make `session::append` idempotent: when the stored entry is byte-identical to the incoming one, return it instead of raising.',
          steps: ['Compare the stored entry with the incoming one before raising.', 'Return the stored entry with `deduplicated: true`.', 'Add a test that appends the same entry twice.'],
          files: ['session-manager/src/store.rs', 'session-manager/tests/append.rs'],
          missing: ['Whether any caller depends on the conflict to detect a double send.'],
        },
      },
      {
        id: 'redeliv', type: 'redelivery exhausted', msg: 'after <n> attempts for message <id>',
        worker: 'queue', fn: 'fn_queue harness-turn', source: 'log', status: 'new',
        verFirst: '0.23.0', verLast: '0.23.0', seed: 77, hot: 0, count: 5, sessions: 0,
        first: '40 min ago', firstAbs: '2026-09-16 13:22', last: '9 min ago', lastAbs: '2026-09-16 13:53:41',
        triage: {
          hyp: 'A poison message that fails every attempt — the payload is likely rejected before the handler runs.',
          look: ['queue/src/delivery.rs', 'harness/src/functions/turn.rs'], cat: 'likely bug',
        },
        log: {
          t: '2026-09-16 13:53:41.220', lvl: 'ERROR', svc: 'queue · 0.23.0',
          body: 'redelivery exhausted after 3 attempts for message q_88fe12a0 (queue=harness-turn group=s_7a1)',
          target: 'queue::delivery', trace: 'no trace context', span: '—',
          attrs: [{ k: 'queue', v: 'harness-turn' }, { k: 'message_group', v: 's_7a1' }, { k: 'attempts', v: '3' }, { k: 'last_error', v: 'handler returned 400: unknown field "thinking_level"' }],
        },
        logs: [
          { t: '13:53:41.220', lvl: 'ERROR', svc: 'queue', body: 'redelivery exhausted after 3 attempts for message q_88fe12a0' },
          { t: '13:53:38.101', lvl: 'WARN', svc: 'queue', body: 'attempt 3/3 failed for q_88fe12a0: handler returned 400' },
          { t: '13:53:31.884', lvl: 'WARN', svc: 'queue', body: 'attempt 2/3 failed for q_88fe12a0: handler returned 400' },
        ],
        occ: [
          { at: '9 min ago', ver: '0.23.0', session: '—', turn: '—', ev: 'full' },
          { at: '21 min ago', ver: '0.23.0', session: '—', turn: '—', ev: 'full' },
          { at: '40 min ago', ver: '0.23.0', session: '—', turn: '—', ev: 'none' },
        ],
        hist: [
          { at: '9 min ago', kind: 'ghost', what: 'Last occurrence', note: '5 in 40 min' },
          { at: '40 min ago', kind: 'ghost', what: 'First seen', note: 'queue 0.23.0' },
        ],
      },
      {
        id: 'notfound', type: 'NotFound', msg: 'page "<str>" is not registered',
        worker: 'ade', fn: 'console::workspace::open', source: 'trace', status: 'new',
        verFirst: '0.23.0', verLast: '0.23.0', seed: 15, hot: 0, count: 3, sessions: 2,
        first: '6 h ago', firstAbs: '2026-09-16 08:12', last: '1 h ago', lastAbs: '2026-09-16 13:04:09',
        triage: {
          hyp: 'A workspace tab points at a page whose worker is no longer attached.',
          look: ['ade/src/server.rs', 'ade/web/src/lib/workspace-tabs.ts'], cat: 'likely configuration',
        },
        span: {
          name: 'execute console::workspace::open', fn: 'console::workspace::open', svc: 'ade · 0.23.0',
          status: 'NotFound', dur: '1.9 ms', trace: 'f0a7723c91de…0b18',
          session: '—', turn: '—', tag: '—', prop: 'none',
          exType: 'NotFound', exMsg: 'page "security-scan" is not registered',
          stack: 'ade::ui_assets::resolve_page (src/ui_assets.rs:277)',
        },
        path: [
          { d: 0, tone: 'alert', name: 'execute console::workspace::open', svc: 'ade', origin: true, dur: '1.9 ms' },
        ],
        logs: [
          { t: '13:04:09.551', lvl: 'ERROR', svc: 'ade', body: 'workspace tab references unregistered page security-scan' },
        ],
        occ: [
          { at: '1 h ago', ver: '0.23.0', session: '—', turn: '—', ev: 'full' },
          { at: '6 h ago', ver: '0.23.0', session: '—', turn: '—', ev: 'full' },
        ],
        hist: [
          { at: '1 h ago', kind: 'ghost', what: 'Last occurrence', note: '3 in 6 h' },
          { at: '6 h ago', kind: 'ghost', what: 'First seen', note: 'ade 0.23.0' },
        ],
      },
      {
        id: 'spawn', type: 'SpawnError', msg: 'ENOENT: pnpm not found on PATH',
        worker: 'shell', fn: 'shell::run', source: 'trace', status: 'new',
        verFirst: '0.23.0', verLast: '0.23.0', seed: 501, hot: 0, count: 2, sessions: 1,
        first: '2 h ago', firstAbs: '2026-09-16 12:10', last: '2 h ago', lastAbs: '2026-09-16 12:11:44',
        triage: {
          hyp: 'The engine environment has a different PATH than the operator shell.',
          look: ['shell/src/run.rs'], cat: 'likely configuration',
        },
        span: {
          name: 'execute shell::run', fn: 'shell::run', svc: 'shell · 0.23.0',
          status: 'SpawnError', dur: '0.8 ms', trace: '9012bd4477fa…c5a0',
          session: 'new chat', turn: 't_99b210cc', tag: 'tool', prop: '1 span',
          exType: 'SpawnError', exMsg: 'ENOENT: pnpm not found on PATH',
          stack: 'shell::run::spawn (src/run.rs:88)',
        },
        path: [
          { d: 0, tone: 'alert', name: 'harness::turn step', svc: 'harness', dur: '1.1 s' },
          { d: 1, tone: 'alert', name: 'execute shell::run', svc: 'shell', origin: true, dur: '0.8 ms' },
        ],
        logs: [
          { t: '12:11:44.002', lvl: 'ERROR', svc: 'shell', body: 'spawn pnpm ENOENT (cwd=/home/layon/workspaces/workers)' },
        ],
        occ: [
          { at: '2 h ago', ver: '0.23.0', session: 'new chat', turn: 't_99b210cc', ev: 'full' },
          { at: '2 h ago', ver: '0.23.0', session: 'new chat', turn: 't_99b210cc', ev: 'none' },
        ],
        hist: [
          { at: '2 h ago', kind: 'ghost', what: 'First seen', note: 'shell 0.23.0' },
        ],
      },
      {
        id: 'parse', type: 'ParseError', msg: 'invalid utf-8 at byte <n>',
        worker: 'storage', fn: 'storage::getObject', source: 'trace', status: 'resolved',
        verFirst: '0.22.0', verLast: '0.23.0', resolvedVer: '0.23.0', seed: 220, hot: 0, count: 64, sessions: 11,
        first: '21 d ago', firstAbs: '2026-08-26 14:40', last: '4 d ago', lastAbs: '2026-09-12 09:05:31',
        triage: null,
        span: {
          name: 'execute storage::getObject', fn: 'storage::getObject', svc: 'storage · 0.23.0',
          status: 'ParseError', dur: '12 ms', trace: '3aa19c5502bb…4411',
          session: '—', turn: '—', tag: '—', prop: '1 span',
          exType: 'ParseError', exMsg: 'invalid utf-8 at byte 4096',
          stack: 'storage::local::read_text (src/local.rs:151)',
        },
        path: [{ d: 0, tone: 'alert', name: 'execute storage::getObject', svc: 'storage', origin: true, dur: '12 ms' }],
        logs: [{ t: '09:05:31.700', lvl: 'ERROR', svc: 'storage', body: 'getObject bucket=uploads key=u/1/profile.jpg: invalid utf-8' }],
        occ: [{ at: '4 d ago', ver: '0.23.0', session: '—', turn: '—', ev: 'full' }],
        hist: [
          { at: '4 d ago', kind: 'ok', what: 'Resolved', note: 'now · by you' },
          { at: '21 d ago', kind: 'ghost', what: 'First seen', note: 'storage 0.22.0' },
        ],
      },
      {
        id: 'deadline', type: 'DeadlineExceeded', msg: 'query exceeded <n>ms',
        worker: 'database', fn: 'database::query', source: 'trace', status: 'resolved',
        verFirst: '0.23.0', verLast: '0.23.0', resolvedVer: '0.23.0', seed: 340, hot: 0, count: 27, sessions: 6,
        first: '9 d ago', firstAbs: '2026-09-07 17:20', last: '6 d ago', lastAbs: '2026-09-10 08:44:12',
        triage: null,
        span: {
          name: 'execute database::query', fn: 'database::query', svc: 'database · 0.23.0',
          status: 'DeadlineExceeded', dur: '30.0 s', trace: '77cc01ab98fd…1200',
          session: '—', turn: '—', tag: '—', prop: '1 span',
          exType: 'DeadlineExceeded', exMsg: 'query exceeded 30000ms',
          stack: 'database::handlers::query::run (src/handlers/query.rs:210)',
        },
        path: [{ d: 0, tone: 'alert', name: 'execute database::query', svc: 'database', origin: true, dur: '30.0 s' }],
        logs: [{ t: '08:44:12.100', lvl: 'ERROR', svc: 'database', body: 'query deadline exceeded db=primary' }],
        occ: [{ at: '6 d ago', ver: '0.23.0', session: '—', turn: '—', ev: 'full' }],
        hist: [
          { at: '6 d ago', kind: 'ok', what: 'Resolved', note: 'until version change · on 0.23.0' },
          { at: '9 d ago', kind: 'ghost', what: 'First seen', note: 'database 0.23.0' },
        ],
      },
      {
        id: 'unauth', type: 'Unauthorized', msg: '401 bad credentials',
        worker: 'github', fn: 'github::pr::create', source: 'trace', status: 'resolved',
        verFirst: '0.23.0', verLast: '0.23.0', resolvedVer: '0.23.0', seed: 660, hot: 0, count: 4, sessions: 2,
        first: '8 d ago', firstAbs: '2026-09-08 11:00', last: '8 d ago', lastAbs: '2026-09-08 11:42:03',
        triage: null,
        span: {
          name: 'execute github::pr::create', fn: 'github::pr::create', svc: 'github · 0.23.0',
          status: 'Unauthorized', dur: '340 ms', trace: '1ef0aa22c743…8ba9',
          session: '—', turn: '—', tag: '—', prop: '1 span',
          exType: 'Unauthorized', exMsg: '401 bad credentials',
          stack: 'github::client::post (src/client.rs:77)',
        },
        path: [{ d: 0, tone: 'alert', name: 'execute github::pr::create', svc: 'github', origin: true, dur: '340 ms' }],
        logs: [{ t: '11:42:03.410', lvl: 'ERROR', svc: 'github', body: 'github api 401 for POST /repos/iii-hq/workers/pulls' }],
        occ: [{ at: '8 d ago', ver: '0.23.0', session: '—', turn: '—', ev: 'full' }],
        hist: [
          { at: '8 d ago', kind: 'ok', what: 'Resolved', note: 'now · by you' },
          { at: '8 d ago', kind: 'ghost', what: 'First seen', note: 'github 0.23.0' },
        ],
      },
      {
        id: 'econn', type: 'ECONNRESET', msg: 'socket hang up',
        worker: 'browser', fn: 'browser::navigate', source: 'trace', status: 'ignored',
        ignoreRule: 'forever', verFirst: '0.23.0', verLast: '0.23.0', seed: 880, hot: 0, count: 156, sessions: 12,
        first: '30 d ago', firstAbs: '2026-08-17 09:00', last: '18 min ago', lastAbs: '2026-09-16 13:44:52',
        triage: null,
        span: {
          name: 'execute browser::navigate', fn: 'browser::navigate', svc: 'browser · 0.23.0',
          status: 'ECONNRESET', dur: '2.1 s', trace: 'c9a0e1bb77af…3e70',
          session: 'new chat', turn: 't_4471cb90', tag: 'tool', prop: '1 span',
          exType: 'ECONNRESET', exMsg: 'socket hang up',
          stack: 'browser::cdp::connect (src/cdp.rs:268)',
        },
        path: [{ d: 0, tone: 'alert', name: 'execute browser::navigate', svc: 'browser', origin: true, dur: '2.1 s' }],
        logs: [{ t: '13:44:52.900', lvl: 'ERROR', svc: 'browser', body: 'cdp websocket closed while navigating' }],
        occ: [{ at: '18 min ago', ver: '0.23.0', session: 'new chat', turn: 't_4471cb90', ev: 'full' }],
        hist: [
          { at: '12 d ago', kind: 'ink', what: 'Ignored', note: 'forever · by you' },
          { at: '30 d ago', kind: 'ghost', what: 'First seen', note: 'browser 0.23.0' },
        ],
      },
      {
        id: 'cancelled', type: 'Cancelled', msg: 'turn cancelled by user',
        worker: 'harness', fn: 'harness::stop', source: 'trace', status: 'ignored',
        ignoreRule: 'version', verFirst: '0.23.0', verLast: '0.23.0', seed: 910, hot: 0, count: 92, sessions: 31,
        first: '16 d ago', firstAbs: '2026-08-31 10:30', last: '35 min ago', lastAbs: '2026-09-16 13:27:10',
        triage: null,
        span: {
          name: 'execute harness::stop', fn: 'harness::stop', svc: 'harness · 0.23.0',
          status: 'Cancelled', dur: '6 ms', trace: '4b7702aa1c39…dd21',
          session: 'promptlab-real-t3-qv5', turn: 't_7781ba03', tag: 'step', prop: '1 span',
          exType: 'Cancelled', exMsg: 'turn cancelled by user',
          stack: 'harness::functions::stop::handle (src/functions/stop.rs:52)',
        },
        path: [{ d: 0, tone: 'alert', name: 'execute harness::stop', svc: 'harness', origin: true, dur: '6 ms' }],
        logs: [{ t: '13:27:10.004', lvl: 'ERROR', svc: 'harness', body: 'turn t_7781ba03 cancelled' }],
        occ: [{ at: '35 min ago', ver: '0.23.0', session: 'promptlab-real-t3-qv5', turn: 't_7781ba03', ev: 'full' }],
        hist: [
          { at: '9 d ago', kind: 'ink', what: 'Ignored', note: 'until harness version changes · 0.23.0' },
          { at: '16 d ago', kind: 'ghost', what: 'First seen', note: 'harness 0.23.0' },
        ],
      },
    ]
    this._groups = G
    return G
  }

  repoFor(worker) {
    const map = {
      workers: ['harness', 'ade', 'session-manager', 'context-manager', 'llm-router', 'state', 'queue', 'provider-anthropic', 'storage', 'database', 'github'],
      iii: ['iii'],
    }
    for (const id of Object.keys(map)) {
      if (map[id].indexOf(worker) >= 0) return { id, path: id === 'iii' ? '/home/layon/workspaces/iii' : '/home/layon/workspaces/workers' }
    }
    return null
  }

  models() {
    return [
      { id: 'anthropic::claude-sonnet-5', label: 'anthropic · claude-sonnet-5' },
      { id: 'openai::gpt-5.6-luna', label: 'openai · gpt-5.6-luna' },
      { id: 'deepseek::deepseek-v4-pro', label: 'deepseek · deepseek-v4-pro' },
    ]
  }

  /* ===================== state helpers ===================== */

  s() { return this.state || {} }
  get(k, d) { const v = this.s()[k]; return v === undefined ? d : v }
  set(patch) {
    this.setState(patch)
    if (patch && patch.toast) {
      if (this._toastTimer) clearTimeout(this._toastTimer)
      this._toastTimer = setTimeout(() => { this.setState({ toast: null }) }, 4200)
    }
  }

  statusOf(g) {
    const over = this.get('status', {})
    return over[g.id] || g.status
  }

  ruleOf(g) {
    const over = this.get('rule', {})
    return over[g.id] || g.ignoreRule || null
  }

  diagOf(g) {
    const over = this.get('diag', {})
    if (over[g.id]) return over[g.id]
    return g.diag ? [g.diag] : []
  }

  current() {
    const id = this.get('id', null)
    if (!id) return null
    return this.groups().find((g) => g.id === id) || null
  }

  modelLabel() {
    const id = this.get('model', 'anthropic::claude-sonnet-5')
    const m = this.models().find((x) => x.id === id)
    return m ? m.label : id
  }

  timer(ms, fn) {
    if (!this._timers) this._timers = []
    const t = setTimeout(() => {
      this._timers = this._timers.filter((x) => x !== t)
      fn()
    }, ms)
    this._timers.push(t)
  }

  clearTimers() {
    ;(this._timers || []).forEach(clearTimeout)
    this._timers = []
  }

  componentWillUnmount() { this.clearTimers() }

  /* ===================== sparkline ===================== */

  bars(seed, hot) {
    let x = seed
    const out = []
    for (let i = 0; i < 24; i++) {
      x = (x * 9301 + 49297) % 233280
      const h = 8 + Math.floor((x / 233280) * 70)
      const isHot = i >= 24 - (hot || 0)
      out.push({ h: (isHot ? 100 : h) + '%', cls: isHot ? 'hot' : '' })
    }
    return out
  }

  /* ===================== investigation ===================== */

  script(g) {
    const top = (g.span && g.span.stack ? g.span.stack.split('\n')[0] : '').trim()
    const file = top.replace(/^[^(]*\(/, '').replace(/\)$/, '') || 'src/lib.rs'
    const brief = g.source === 'log'
      ? 'Log record with no error span in its trace: ' + g.log.body
      : 'Evidence bundle attached: origin span, trace path, ' + g.logs.length + ' logs.'
    const intro =
      'Investigate group ' + g.type + ': ' + g.msg + ' — ' + g.worker + ' · ' + g.fn + ', ' +
      g.count + ' occurrences over ' + (g.sessions || 0) + ' sessions. ' + brief +
      (this.repoFor(g.worker)
        ? ' Repository ' + this.repoFor(g.worker).id + ' at 4662b0d, read-only.'
        : ' No repository is mapped for ' + g.worker + ' — work from the evidence alone and say so if that is not enough.')
    if (g.id === 'cas') {
      return [
        { kind: 'user', who: 'Sentinel', text: intro, chips: ['evidence · occ_01j8m2…', 'fs · workers/'] },
        { kind: 'thought', text: 'thought briefly' },
        { kind: 'call', fn: 'coder::search', arg: '(retry_step, harness/src)', dur: '0.3 s' },
        { kind: 'call', fn: 'coder::read-file', arg: 'harness/src/turn_loop.rs:380–440', dur: '0.1 s' },
        { kind: 'assistant', text: 'Every sampled occurrence carries attempt=2 on the failing step and expected = found − 1. retry_step clones the step context captured before attempt 1 — so the retry sends a version that was correct one write ago.' },
        { kind: 'assistant', text: 'I cannot yet see what writes that extra version between the two attempts. Recording what I have at medium confidence, with that gap named.' },
        { kind: 'call', fn: 'sentinel::diagnosis::record', arg: '{ group_id: grp_cas, confidence: medium }', dur: '0.1 s', record: 'v1' },
      ]
    }
    const repo = this.repoFor(g.worker)
    if (!repo) {
      return [
        { kind: 'user', who: 'Sentinel', text: intro, chips: ['evidence · occ_01j8m2…'] },
        { kind: 'thought', text: 'thought briefly' },
        { kind: 'call', fn: 'sentinel::evidence::get', arg: 'occ_01j8m2… (full bundle)', dur: '0.1 s' },
        { kind: 'call', fn: 'engine::logs::list', arg: 'trace ' + (g.span ? g.span.trace : '—'), dur: '0.2 s' },
        { kind: 'assistant', text: 'I have the evidence and no source for ' + g.worker + ' — nothing maps its repository on this machine. I can pin the failure point and its shape, not the line that causes it. Recording that at low confidence.' },
        { kind: 'call', fn: 'sentinel::diagnosis::record', arg: '{ group_id: grp_' + g.id + ', confidence: low }', dur: '0.1 s', record: 'v1' },
      ]
    }
    return [
      { kind: 'user', who: 'Sentinel', text: intro, chips: ['evidence · occ_01j8m2…', 'fs · ' + repo.id + '/'] },
      { kind: 'thought', text: 'thought briefly' },
      { kind: 'call', fn: 'coder::search', arg: '(' + g.fn.split('::').pop() + ', ' + g.worker + '/src)', dur: '0.4 s' },
      { kind: 'call', fn: 'coder::read-file', arg: file, dur: '0.2 s' },
      { kind: 'assistant', text: 'The failure is reproducible from the evidence, but the repository at this checkout does not show what produces it. Recording a low-confidence diagnosis that names what is missing rather than guessing.' },
      { kind: 'call', fn: 'sentinel::diagnosis::record', arg: '{ group_id: grp_' + g.id + ', confidence: low }', dur: '0.1 s', record: 'v1' },
    ]
  }

  firstPassDiag(g) {
    if (g.id === 'cas') {
      return {
        v: 1, source: 'first pass', when: 'just now', model: this.modelLabel().split(' · ').pop(),
        turns: '4 turns · 1 min 12 s', conf: 'medium', cat: 'bug', risk: 'low',
        summary: 'The harness re-enqueues a turn step after a transient failure without re-reading the session state version, so the second attempt sends a stale expected_version and the compare-and-set rejects it.',
        cause: 'turn_loop::retry_step clones the StepContext captured before the first attempt, including state_version. Something writes a new version between attempts — the mismatch is always exactly one — but this pass did not find the writer.',
        ev: [
          { kind: 'code', path: 'harness/src/turn_loop.rs:412', code: 'let ctx = self.step_context.clone();      // captured before attempt 1\nself.enqueue_retry(ctx, attempt + 1).await?;', why: 'The retry reuses the pre-attempt context; nothing refreshes ctx.state_version.' },
          { kind: 'trace', path: 'span 3f9a…c21e · harness::turn step · attempt 2', code: '', why: 'All 5 sampled occurrences carry attempt=2 and expected = found − 1; none on attempt 1.' },
        ],
        fix: 'Re-read the state version when building the retry context.',
        steps: ['Find what writes the version between attempts.', 'Carry that version into enqueue_retry instead of cloning.'],
        files: ['harness/src/turn_loop.rs'],
        missing: ['What writes a new state version between attempt 1 and attempt 2, and when that behaviour landed.'],
      }
    }
    return {
      v: 1, source: 'first pass', when: 'just now', model: this.modelLabel().split(' · ').pop(),
      turns: '3 turns · 48 s', conf: 'low', cat: 'unknown', risk: 'low',
      summary: 'The evidence pins down where and how often ' + g.worker + ' fails here, but not why. Stopping short of a cause rather than guessing at one.',
      cause: this.repoFor(g.worker)
        ? 'The origin span and the logs agree on the failure point in ' + g.worker + ', and the shape is consistent across occurrences. The checkout does not contain enough context to attribute a cause, so this pass stops short of one.'
        : 'No repository is mapped for ' + g.worker + ', so this pass had the evidence and no source. The failure point and its shape are solid; attributing a cause is not possible from the trace alone.',
      ev: [
        { kind: 'trace', path: g.source === 'log' ? 'log record · ' + g.worker : 'origin span · ' + g.fn, code: '', why: 'The same failure point in every sampled occurrence.' },
      ],
      fix: '',
      steps: [],
      files: [],
      missing: (this.repoFor(g.worker) ? [] : ['The source of ' + g.worker + ' — map its repository in the configuration and investigate again.']).concat([
        'A reproduction, or one occurrence with the caller arguments attached.',
        g.source === 'log' ? 'A trace: this group comes from a log record with no error span.' : 'Whether the caller or the callee owns the invalid state.',
      ]),
    }
  }

  updatedDiag(g, prev) {
    if (g.id === 'cas') {
      return {
        v: (prev ? prev.v : 1) + 1, source: 'update', when: 'just now', model: this.modelLabel().split(' · ').pop(),
        turns: '9 turns · 4 min 12 s', conf: 'high', cat: 'bug', risk: 'low',
        summary: 'The harness re-enqueues a turn step after a transient failure without re-reading the session state version, so the second attempt sends the stale expected_version and the compare-and-set rejects it. The mismatch is always expected = found − 1, and every occurrence follows a WARN re-enqueueing (attempt 2/3) in the same trace.',
        cause: 'turn_loop::retry_step clones the StepContext captured before the first attempt, including state_version. The 0.23.0 change that made steps durable (state.rs, "checkpoint before dispatch") writes a new version on every attempt, so the clone is stale by exactly one on retry.',
        ev: [
          { kind: 'code', path: 'harness/src/turn_loop.rs:412', code: 'let ctx = self.step_context.clone();      // captured before attempt 1\nself.enqueue_retry(ctx, attempt + 1).await?;', why: 'The retry reuses the pre-attempt context; nothing refreshes ctx.state_version.' },
          { kind: 'code', path: 'harness/src/state.rs:188', code: 'run_hidden("harness state", self.checkpoint(step, &ctx)).await?;   // bumps the version on every attempt since 0.23.0', why: 'Introduced by the durable-step checkpoint; absent in 0.22.1, which explains the regression on 0.23.0. Found after your note.' },
          { kind: 'trace', path: 'span 3f9a…c21e · harness::turn step · attempt 2', code: '', why: 'All 5 sampled occurrences carry attempt=2 and expected = found − 1; none on attempt 1.' },
        ],
        fix: 'Re-read the state version when building the retry context, or carry the version returned by checkpoint() into enqueue_retry.',
        steps: ['Make checkpoint() return the new state_version.', 'In retry_step, build the context from that value instead of cloning.', 'Add a regression test: fail the first attempt, assert the second CAS carries the bumped version.'],
        files: ['harness/src/turn_loop.rs', 'harness/src/state.rs', 'harness/tests/turn_retry.rs'],
        missing: [],
      }
    }
    const base = prev || this.firstPassDiag(g)
    const next = JSON.parse(JSON.stringify(base))
    next.v = (prev ? prev.v : 1) + 1
    next.source = 'update'
    next.when = 'just now'
    next.conf = 'medium'
    next.turns = '6 turns · 2 min 30 s'
    next.cause = base.cause + ' Your note in the session narrowed it to the path above, which raises the confidence but does not yet name a line.'
    return next
  }

  // A record only moves a group forward, from investigating or new to
  // diagnosed. Resolve, Ignore and a regression are human or ingest facts
  // the agent cannot undo: the state is read at the moment the record
  // lands, and if it was decided meanwhile it stays. Returns the kept state.
  promote(g, patch) {
    const cur = this.statusOf(g)
    const kept = cur === 'resolved' || cur === 'ignored' || cur === 'regressed'
    if (!kept && cur !== 'diagnosed') {
      const st = Object.assign({}, this.get('status', {}))
      st[g.id] = 'diagnosed'
      patch.status = st
    }
    return kept ? cur : null
  }

  startInvestigation(g, mode) {
    this.clearTimers()
    const chat = this.script(g)
    const status = Object.assign({}, this.get('status', {}))
    if (mode !== 'chat') status[g.id] = 'investigating'
    this.set({
      status,
      menu: null,
      dialog: null,
      tab: 'diagnosis',
      chatOpen: true,
      inv: { gid: g.id, mode: mode || 'assisted', step: mode === 'chat' ? 1 : 1, running: mode !== 'chat', chat, replies: 0 },
    })
    if (mode === 'chat') return
    const advance = () => {
      const inv = this.get('inv', null)
      if (!inv || inv.gid !== g.id || !inv.running) return
      if (inv.step >= chat.length) {
        this.set({ inv: Object.assign({}, inv, { running: false }) })
        return
      }
      const next = chat[inv.step]
      const patch = { inv: Object.assign({}, inv, { step: inv.step + 1 }) }
      if (next && next.record) {
        const dg = Object.assign({}, this.get('diag', {}))
        dg[g.id] = [this.firstPassDiag(g)]
        patch.diag = dg
        const kept = this.promote(g, patch)
        if (kept) patch.toast = 'Diagnosis v1 recorded. The group stays ' + kept + ' — a record never undoes your decision.'
      }
      this.set(patch)
      this.timer(950, advance)
    }
    this.timer(950, advance)
  }

  /* ===================== render ===================== */

  renderVals() {
    const self = this
    const s = this.s()
    const screen = this.get('screen', 'list')
    const filter = this.get('filter', 'open')
    const win = this.get('win', '24h')
    const workerFilter = this.get('worker', '')
    const q = this.get('q', '')
    const menu = this.get('menu', null)
    const dialog = this.get('dialog', null)
    const tab = this.get('tab', 'latest')
    const inv = this.get('inv', null)
    // The session column is a view of the investigation, not the
    // investigation: closing it hides the column and nothing else.
    const chatOpen = this.get('chatOpen', true)
    const toast = this.get('toast', null)
    const g = this.current()
    const gStatus = g ? this.statusOf(g) : null
    const diags = g ? this.diagOf(g) : []
    const diag = diags.length ? diags[0] : null

    const close = () => this.set({ menu: null })
    const openMenu = (m) => (e) => {
      if (e && e.stopPropagation) e.stopPropagation()
      this.set({ menu: menu === m ? null : m })
    }

    /* ---- list rows ---- */
    const inFilter = (st) => {
      if (filter === 'open') return st === 'new' || st === 'investigating' || st === 'diagnosed' || st === 'regressed'
      if (filter === 'regressed') return st === 'regressed'
      if (filter === 'ignored') return st === 'ignored'
      return st === 'resolved'
    }
    const needle = q.trim().toLowerCase()
    const all = this.groups()
    const matching = all.filter((x) => {
      const st = this.statusOf(x)
      if (!inFilter(st)) return false
      if (workerFilter && x.worker !== workerFilter) return false
      if (!needle) return true
      return (x.type + ' ' + x.msg + ' ' + x.fn + ' ' + x.worker).toLowerCase().indexOf(needle) >= 0
    })
    const order = { regressed: 0, investigating: 1, diagnosed: 2, new: 3, resolved: 4, ignored: 5 }
    matching.sort((a, b) => order[this.statusOf(a)] - order[this.statusOf(b)])

    const badge = (st) => {
      if (st === 'regressed') return { cls: 'badge alert', text: 'regressed', alert: true }
      if (st === 'investigating') return { cls: 'badge accent', text: 'investigating', live: true }
      if (st === 'diagnosed') return { cls: 'badge', text: 'diagnosed', file: true }
      if (st === 'resolved') return { cls: 'badge ok', text: 'resolved', check: true }
      if (st === 'ignored') return { cls: 'badge', text: 'ignored', ban: true }
      return { cls: 'badge', text: 'new' }
    }
    const dotFor = (st) =>
      st === 'regressed' ? 'dot alert' : st === 'investigating' ? 'dot' : st === 'diagnosed' ? 'dot ok' : st === 'resolved' ? 'dot ok' : 'dot ghost'

    const rows = matching.map((x) => {
      const st = this.statusOf(x)
      const b = badge(st)
      return {
        id: x.id,
        cls: 'tg-row is-row' + (st === 'regressed' ? ' regressed' : ''),
        dotCls: dotFor(st),
        type: x.type,
        msg: x.msg,
        isLog: x.source === 'log',
        meta: x.worker + ' · ' + x.fn + ' · ' + (x.verFirst === x.verLast ? x.verLast : x.verFirst + ' → ' + x.verLast),
        count: x.count.toLocaleString('en-US').replace(/,/g, ' '),
        sessions: x.sessions ? String(x.sessions) : '—',
        first: x.first,
        last: x.last,
        bars: this.bars(x.seed, x.hot),
        badgeCls: b.cls,
        badgeText: b.text,
        bAlert: !!b.alert,
        bLive: !!b.live,
        bFile: !!b.file,
        bCheck: !!b.check,
        bBan: !!b.ban,
        open: () => self.set({ screen: 'detail', id: x.id, tab: 'latest', menu: null }),
      }
    })

    const counts = {
      open: all.filter((x) => inFilterFor('open', this.statusOf(x))).length,
      regressed: all.filter((x) => this.statusOf(x) === 'regressed').length,
      ignored: all.filter((x) => this.statusOf(x) === 'ignored').length,
      resolved: all.filter((x) => this.statusOf(x) === 'resolved').length,
    }
    function inFilterFor(f, st) {
      if (f === 'open') return st === 'new' || st === 'investigating' || st === 'diagnosed' || st === 'regressed'
      if (f === 'regressed') return st === 'regressed'
      if (f === 'ignored') return st === 'ignored'
      return st === 'resolved'
    }

    const filters = [
      { key: 'open', label: 'Open' },
      { key: 'regressed', label: 'Regressed' },
      { key: 'ignored', label: 'Ignored' },
      { key: 'resolved', label: 'Resolved' },
    ].map((f) => ({
      label: f.label,
      cls: filter === f.key ? 'on' : '',
      pick: () => self.set({ filter: f.key, menu: null }),
    }))

    const windows = ['24 h', '7 d', '30 d', 'All'].map((w) => ({
      label: w,
      cls: win === w ? 'on' : '',
      pick: () => self.set({ win: w }),
    }))

    const workerNames = []
    all.forEach((x) => { if (workerNames.indexOf(x.worker) < 0) workerNames.push(x.worker) })
    workerNames.sort()
    const workerOptions = [{ label: 'All workers', value: '' }].concat(
      workerNames.map((w) => ({ label: w, value: w })),
    ).map((o) => ({
      label: o.label,
      cls: 'mi' + (workerFilter === o.value ? ' hi' : ''),
      pick: () => self.set({ worker: o.value, menu: null }),
    }))

    /* ---- detail ---- */
    let detail = null
    if (g) {
      const st = gStatus
      const rule = this.ruleOf(g)
      const actions = []
      const hasSession = !!(inv && inv.gid === g.id)
      if (st === 'investigating') {
        actions.push({ cls: 'btn pill sm', label: 'Stop', iconStop: true, click: () => {
          this.clearTimers()
          const sts = Object.assign({}, this.get('status', {}))
          sts[g.id] = g.status === 'investigating' ? 'new' : g.status
          // harness::stop ends the first pass; the session itself stays.
          const cur = this.get('inv', null)
          const stopped = cur && cur.gid === g.id ? Object.assign({}, cur, { running: false, stopped: true }) : cur
          this.set({ status: sts, inv: stopped, toast: 'First pass stopped. The session stays — open it to continue by hand.' })
        } })
      } else if (st === 'resolved') {
        actions.push({ cls: 'btn primary sm', label: 'Reopen', iconRotate: true, click: () => {
          const sts = Object.assign({}, this.get('status', {}))
          sts[g.id] = 'new'
          this.set({ status: sts, menu: null, toast: 'Reopened.' })
        } })
      } else if (st === 'ignored') {
        actions.push({ cls: 'btn primary sm', label: 'Unignore', iconRotate: true, click: () => {
          const sts = Object.assign({}, this.get('status', {}))
          const rules = Object.assign({}, this.get('rule', {}))
          sts[g.id] = 'new'
          delete rules[g.id]
          this.set({ status: sts, rule: rules, menu: null, toast: 'Back in the open list.' })
        } })
      } else if (st === 'diagnosed') {
        actions.push({ cls: 'btn primary sm', label: 'Resolve', iconCheck: true, caret: true, click: openMenu('resolve') })
        actions.push({ cls: 'btn pill sm', label: 'Continue in chat', iconChat: true, click: () => {
          if (hasSession) this.set({ menu: null, chatOpen: true })
          else this.startInvestigation(g, 'chat')
        } })
        actions.push({ cls: 'btn pill sm', label: 'Investigate', caret: true, click: openMenu('investigate') })
      } else {
        actions.push({ cls: 'btn primary sm', label: 'Investigate', iconScan: true, caret: true, click: openMenu('investigate') })
        actions.push({ cls: 'btn pill sm', label: 'Resolve', caret: true, click: openMenu('resolve') })
      }
      if (st !== 'ignored') {
        actions.push({ cls: 'btn pill sm' + (menu === 'ignore' ? ' is-open' : ''), label: 'Ignore', caret: true, click: openMenu('ignore') })
      }
      // The investigation outlives the column: with it closed, the header
      // offers the way back in (host.chat.selectConversation on the real page).
      if (hasSession && !chatOpen) {
        actions.unshift({ cls: 'btn pill sm', label: 'Open session', iconChat: true, live: !!inv.running, click: () => this.set({ chatOpen: true, menu: null }) })
      }

      const b = badge(st)
      const facts = [
        { dt: 'First seen', b: g.first, rest: ' · ' + g.firstAbs },
        { dt: 'Last seen', b: g.last, rest: ' · ' + g.lastAbs },
        { dt: 'Occurrences · sessions', b: g.count.toLocaleString('en-US').replace(/,/g, ' '), rest: ' · ' + (g.sessions ? g.sessions + ' distinct sessions' : 'no session context') },
        { dt: 'Worker version', b: g.verFirst, rest: g.verFirst === g.verLast ? '' : ' → ' + g.verLast },
      ]

      const tabs = [
        { key: 'latest', label: 'Latest occurrence', count: '' },
        { key: 'occurrences', label: 'Occurrences', count: g.count.toLocaleString('en-US').replace(/,/g, ' ') },
        { key: 'diagnosis', label: 'Diagnosis', count: diags.length ? String(diags.length) : '' },
        { key: 'history', label: 'History', count: '' },
      ].map((t) => ({
        label: t.label,
        count: t.count,
        cls: 'tab' + (tab === t.key ? ' on' : ''),
        pick: () => self.set({ tab: t.key, menu: null }),
      }))

      const evidenceLabel = (e) => (e === 'full' ? 'snapshot' : 'pruned')
      detail = {
        id: g.id,
        type: g.type,
        msg: g.msg,
        title: g.type + ': ' + g.msg,
        kicker: g.worker + ' · ' + g.fn,
        fp: 'fp ' + g.id + '7a…4f2a',
        source: 'source ' + g.source,
        dotCls: dotFor(st),
        badgeCls: b.cls,
        badgeText: b.text + (st === 'regressed' ? ' ' + g.last : ''),
        bAlert: !!b.alert,
        bLive: !!b.live,
        bFile: !!b.file,
        bCheck: !!b.check,
        bBan: !!b.ban,
        count: g.count.toLocaleString('en-US').replace(/,/g, ' ') + ' occurrences',
        sessions: (g.sessions || 0) + ' sessions',
        vers: g.verFirst === g.verLast ? g.verLast : g.verFirst + ' → ' + g.verLast,
        checkout: this.repoFor(g.worker) ? 'checkout ' + this.repoFor(g.worker).id + '@4662b0d' : 'no checkout — evidence only',
        hasRepo: !!this.repoFor(g.worker),
        noRepo: !this.repoFor(g.worker),
        repoLabel: this.repoFor(g.worker) ? this.repoFor(g.worker).id + '/ · read-only' : 'no repository mapped',
        noRepoNote: 'No repository is mapped for ' + g.worker + ', so an investigation reads the ' +
          (g.source === 'log' ? 'log window' : 'trace') +
          ' and nothing else. Grouping, evidence, regression and the whole list work the same — only the agent\'s code access is missing. Map it in the configuration to give the agent the source.',
        actions,
        facts,
        tabs,
        isRegressed: st === 'regressed',
        regressedNote: g.resolvedVer
          ? 'Resolved 3 d ago in ' + g.worker + ' ' + g.resolvedVer + ' with until version change; the first occurrence on ' + g.verLast + ' reopened it ' + g.last + '. Occurrences on ' + g.resolvedVer + ' kept counting without reopening.'
          : '',
        isIgnored: st === 'ignored',
        ignoreNote:
          rule === 'forever' ? 'Ignored forever. Occurrences still count; nothing surfaces in the open list.'
            : rule === 'version' ? 'Ignored until the ' + g.worker + ' version changes (baseline ' + g.verLast + '). Occurrences still count.'
              : rule === 'count' ? 'Ignored until 50 more occurrences. Occurrences still count.' : '',
        isResolved: st === 'resolved',
        resolvedNote: 'Resolved. A new occurrence reopens it as a regression.',
        hasTriage: !!g.triage,
        triageHyp: g.triage ? g.triage.hyp : '',
        triageLook: g.triage ? g.triage.look : [],
        triageCat: g.triage ? g.triage.cat : '',
        /* latest occurrence */
        isTraceSource: g.source === 'trace',
        isLogSource: g.source === 'log',
        span: g.span || null,
        path: (g.path || []).map((p) => ({
          cls: 'span-row' + (p.origin ? ' origin' : ''),
          style: 'padding-left: ' + (8 + p.d * 16) + 'px',
          dotCls: 'dot ' + p.tone,
          name: p.name,
          svc: p.svc,
          dur: p.dur,
          isProp: !!p.tag,
          isOrigin: !!p.origin,
        })),
        logRec: g.log || null,
        logs: g.logs || [],
        logCount: String((g.logs || []).length),
        /* occurrences */
        occ: (g.occ || []).map((o) => ({
          at: o.at, ver: o.ver, session: o.session, turn: o.turn,
          evCls: o.ev === 'full' ? 'chip' : 'chip ghost-chip',
          ev: evidenceLabel(o.ev),
          evFull: o.ev === 'full',
        })),
        occNote: (g.occ || []).length + ' of ' + g.count.toLocaleString('en-US').replace(/,/g, ' ') +
          ' rows kept · ' + (g.occ || []).filter(function (o) { return o.ev === 'full' }).length +
          ' carry a full evidence snapshot',
        /* history */
        hist: (g.hist || []).map((h) => ({ at: h.at, what: h.what, note: h.note, dotCls: 'dot ' + h.kind })),
      }
    }

    /* ---- diagnosis tab ---- */
    const running = !!(inv && g && inv.gid === g.id && inv.running)
    const diagView = {
      running,
      closed: running && !chatOpen,
      where: chatOpen ? 'in the session beside' : 'in the background — the session column is closed',
      openSession: () => self.set({ chatOpen: true }),
      hasDiag: !!diag,
      empty: !diag && !running,
      liveNote: 'When the agent has a probable cause it records it with sentinel::diagnosis::record — the cards appear here the moment it does. Type in the session at any time to steer it; a message you send folds into the running turn.',
      model: this.modelLabel(),
      turnLabel: inv ? 'turn ' + Math.max(1, Math.ceil(inv.step / 3)) : '',
    }
    if (diag) {
      diagView.d = {
        head: 'Diagnosis v' + diag.v + ' · recorded by the agent ' + diag.when + (diag.source === 'update' ? ' after your note' : ', first pass'),
        via: 'via sentinel::diagnosis::record',
        meta: this.modelLabel() + ' · ' + diag.turns,
        conf: 'confidence ' + diag.conf,
        confCls: 'chip ' + (diag.conf === 'high' ? 'success' : diag.conf === 'medium' ? 'warning' : ''),
        cat: diag.cat,
        catCls: 'chip ' + (diag.cat === 'bug' ? 'danger' : diag.cat === 'unknown' ? '' : 'warning'),
        summary: diag.summary,
        cause: diag.cause,
        ev: (diag.ev || []).map((e) => ({ kind: e.kind, path: e.path, code: e.code, hasCode: !!e.code, why: e.why })),
        hasFix: !!diag.fix,
        fix: diag.fix,
        steps: diag.steps || [],
        files: diag.files || [],
        hasMissing: (diag.missing || []).length > 0,
        missing: diag.missing || [],
        risk: 'risk ' + diag.risk,
      }
      diagView.list = diags.map((d) => ({
        at: d.when,
        label: 'v' + d.v + ' · ' + d.source,
        conf: 'confidence ' + d.conf,
        confCls: 'chip ' + (d.conf === 'high' ? 'success' : d.conf === 'medium' ? 'warning' : ''),
        note: d.missing && d.missing.length ? d.missing[0] : 'no open questions',
        dotCls: d === diag ? 'dot ok' : 'dot ghost',
        tag: d === diag ? 'current' : 'view',
      }))
      diagView.canUpdate = !!(inv && g && inv.gid === g.id)
    }

    /* ---- chat column ---- */
    let chat = null
    if (inv && g && inv.gid === g.id) {
      const entries = inv.chat.slice(0, inv.step).map((c) => ({
        model: self.modelLabel().split(' · ').pop(),
        isUser: c.kind === 'user',
        isThought: c.kind === 'thought',
        isCall: c.kind === 'call',
        isCallPlain: c.kind === 'call' && !c.record,
        isAssistant: c.kind === 'assistant',
        who: c.who || 'You',
        text: c.text,
        chips: c.chips || [],
        hasChips: !!(c.chips && c.chips.length),
        fn: c.fn,
        arg: c.arg,
        dur: c.dur,
        isRecord: !!c.record,
        recordLabel: c.record ? 'recorded ' + c.record : '',
        pending: !!c.pending,
      }))
      chat = {
        title: 'Sentinel: ' + g.type,
        sub: 'automation · ' + this.modelLabel().split(' · ').pop() + ' · ' + (this.repoFor(g.worker) ? this.repoFor(g.worker).id : 'no repository'),
        model: this.modelLabel().split(' · ').pop(),
        repo: this.repoFor(g.worker) ? this.repoFor(g.worker).id : 'no repository',
        status: inv.running ? 'running' : inv.stopped ? 'stopped' : 'ready',
        statusCls: inv.running ? 'dot pulse' : inv.stopped ? 'dot ghost' : 'dot ok',
        ctx: 'ctx ' + Math.min(48, 6 + inv.step * 4) + '%',
        entries,
        draft: this.get('draft', ''),
        onDraft: (e) => self.set({ draft: e.target.value }),
        send: () => self.say((self.get('draft', '') || '').trim()),
        close: () => self.set({ chatOpen: false }),
      }
    }

    /* ---- menus ---- */
    const menus = {
      worker: menu === 'worker',
      win: false,
      investigate: menu === 'investigate',
      resolve: menu === 'resolve',
      ignore: menu === 'ignore',
      model: menu === 'model',
      any: !!menu,
      close,
    }

    const investigateItems = g ? [
      { label: 'Investigate', hint: this.repoFor(g.worker) ? this.repoFor(g.worker).id + '/' : 'trace only', cls: 'mi hi', pick: () => this.startInvestigation(g, 'assisted') },
      { label: 'Investigate with…', hint: 'pick a model', cls: 'mi', pick: () => this.set({ menu: 'model' }) },
      { label: 'Open in chat', hint: 'no first pass', cls: 'mi', pick: () => this.startInvestigation(g, 'chat') },
    ] : []

    const resolveItems = g ? [
      { label: 'Resolve now', hint: 'reopens on any new occurrence', cls: 'mi hi', pick: () => {
        const sts = Object.assign({}, this.get('status', {}))
        sts[g.id] = 'resolved'
        this.set({ status: sts, menu: null, toast: 'Resolved. A new occurrence reopens it as a regression.' })
      } },
      { label: 'Resolve until version change', hint: g.worker + ' ' + g.verLast, cls: 'mi', pick: () => {
        const sts = Object.assign({}, this.get('status', {}))
        sts[g.id] = 'resolved'
        this.set({ status: sts, menu: null, toast: 'Resolved on ' + g.verLast + '. Occurrences on this version keep counting without reopening.' })
      } },
    ] : []

    const ignoreItems = g ? [
      { label: 'Forever', hint: '', cls: 'mi', pick: () => this.ignore(g, 'forever', 'Ignored forever.') },
      { label: '50 more occurrences', hint: '', cls: 'mi', pick: () => this.ignore(g, 'count', 'Ignored until 50 more occurrences.') },
      { label: g.worker + ' version changes', hint: g.verLast, cls: 'mi hi', pick: () => this.ignore(g, 'version', 'Ignored until ' + g.worker + ' leaves ' + g.verLast + '.') },
    ] : []

    const modelItems = this.models().map((m) => ({
      label: m.label,
      cls: 'mi' + (this.get('model', 'anthropic::claude-sonnet-5') === m.id ? ' hi' : ''),
      pick: () => {
        this.set({ model: m.id, menu: null })
        if (screen === 'detail' && g) this.startInvestigation(g, 'assisted')
      },
    }))

    /* ---- settings ---- */
    const settings = {
      model: this.modelLabel(),
      openModel: openMenu('model'),
      triageOn: this.get('triage', false),
      triageCls: this.get('triage', false) ? 'switch on' : 'switch',
      toggleTriage: () => this.set({ triage: !this.get('triage', false) }),
      spansCls: this.get('srcSpans', true) ? 'switch on' : 'switch',
      toggleSpans: () => this.set({ srcSpans: !this.get('srcSpans', true) }),
      logsCls: this.get('srcLogs', true) ? 'switch on' : 'switch',
      toggleLogs: () => this.set({ srcLogs: !this.get('srcLogs', true) }),
      triageRowCls: this.get('triage', false) ? 'srow' : 'srow is-off',
    }

    return {
      theme: this.props.theme ?? 'light',
      /* screens */
      isList: screen === 'list',
      isDetail: screen === 'detail' && !!g,
      isSettings: screen === 'settings',
      isSplit: !!chat && chatOpen,
      detailCls: chat && chatOpen ? 'detail narrow' : 'detail',
      logsHead: g && g.source === 'log' ? 'Surrounding log window' : 'Logs in this trace',
      /* header */
      act: {
        toList: () => this.set({ screen: 'list', menu: null }),
        toSettings: () => this.set({ screen: 'settings', menu: null }),
        refresh: () => this.set({ toast: 'Re-read from the store — 2 new occurrences.' }),
        closeToast: () => this.set({ toast: null }),
        closeMenu: close,
        askDiagnosis: () => {
          if (!g) return
          if (!inv || inv.gid !== g.id) {
            const c = this.script(g)
            this.set({ inv: { gid: g.id, mode: 'chat', step: c.length, running: false, chat: c, replies: 1 }, chatOpen: true })
          } else {
            this.set({ chatOpen: true })
          }
          this.timer(50, () => this.say('Record what you have so far.'))
        },
      },
      /* list */
      filters,
      windows,
      workerLabel: workerFilter || 'All workers',
      workerOptions,
      openWorker: openMenu('worker'),
      q,
      onQuery: (e) => self.set({ q: e.target.value }),
      rows,
      noRows: rows.length === 0,
      rowCount: rows.length + (rows.length === 1 ? ' group' : ' groups'),
      hiddenNote: counts.resolved + ' resolved · ' + counts.ignored + ' ignored hidden by this filter',
      regressionChip: counts.regressed + (counts.regressed === 1 ? ' regression' : ' regressions'),
      openChip: counts.open + ' open groups',
      hasRegression: counts.regressed > 0,
      /* detail */
      d: detail,
      tabLatest: tab === 'latest',
      tabOcc: tab === 'occurrences',
      tabDiag: tab === 'diagnosis',
      tabHist: tab === 'history',
      dg: diagView,
      /* chat */
      chat,
      /* menus */
      menus,
      investigateItems,
      resolveItems,
      ignoreItems,
      modelItems,
      /* settings */
      st: settings,
      /* toast */
      toast,
      hasToast: !!toast,
    }
  }

  say(text) {
    if (!text) return
    const cur = this.get('inv', null)
    if (!cur) return
    const g = this.groups().find((x) => x.id === cur.gid)
    const next = cur.chat.slice(0, cur.step).concat([
      { kind: 'user', who: 'You', text },
      { kind: 'thought', text: cur.running ? 'steering · folded into the running turn' : 'new turn · console rules' },
    ])
    this.set({ inv: Object.assign({}, cur, { chat: next, step: next.length, replies: cur.replies + 1, stopped: false }), draft: '' })
    this.timer(900, () => {
      const c2 = this.get('inv', null)
      if (!c2 || !g) return
      const dgs = this.diagOf(g)
      const nextV = (dgs[0] ? dgs[0].v : 0) + 1
      const added = c2.chat.concat(g.id === 'cas'
        ? [
            { kind: 'call', fn: 'coder::read-file', arg: 'harness/src/state.rs:160–210', dur: '1.8 s' },
            { kind: 'assistant', text: 'That is it: checkpoint() writes a new version on every attempt since 0.23.0, so the cloned context is stale by exactly one on retry — and 0.22.1 was clean because it had no checkpoint. Recording the cause at high confidence.' },
            { kind: 'call', fn: 'sentinel::diagnosis::record', arg: '{ group_id: grp_cas, confidence: high }', dur: '0.1 s', record: 'v' + nextV },
          ]
        : [
            { kind: 'assistant', text: 'Noted. That narrows it to the path you pointed at, which raises the confidence without naming a line yet. Recording the update.' },
            { kind: 'call', fn: 'sentinel::diagnosis::record', arg: '{ group_id: grp_' + g.id + ', confidence: medium }', dur: '0.1 s', record: 'v' + nextV },
          ])
      const dg = Object.assign({}, this.get('diag', {}))
      dg[g.id] = [this.updatedDiag(g, dgs[0])].concat(dgs)
      const patch = { inv: Object.assign({}, c2, { chat: added, step: added.length }), diag: dg }
      const kept = this.promote(g, patch)
      patch.toast = kept
        ? 'Diagnosis v' + nextV + ' recorded. The group stays ' + kept + ' — a record never undoes your decision.'
        : 'Diagnosis v' + nextV + ' recorded by the agent via sentinel::diagnosis::record.'
      this.set(patch)
    })
  }

  ignore(g, rule, note) {
    const sts = Object.assign({}, this.get('status', {}))
    const rules = Object.assign({}, this.get('rule', {}))
    sts[g.id] = 'ignored'
    rules[g.id] = rule
    this.set({ status: sts, rule: rules, menu: null, toast: note })
  }
}
