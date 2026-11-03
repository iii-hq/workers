import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';

import {
  REFRESH_INTERVALS,
  ROUTES,
  SCHEDULE_PRESETS,
  createWikiSchedules,
  isRawCron,
  observeRoutes,
  observeTurnEvents,
} from '../src/lib/observers.mjs';

// A worker that records each observe() and each handle's dispose() in one log,
// in the order they happen.
function fakeWorker({ disposeRejects = false } = {}) {
  const log = [];
  let next = 0;
  const worker = {
    observe(source, target, options) {
      const id = `observer-${++next}`;
      log.push(
        options === undefined ? { op: 'observe', id, source, target } : { op: 'observe', id, source, target, options },
      );
      const listeners = [];
      return {
        id,
        onStateChange(listener) {
          listeners.push(listener);
          return () => {};
        },
        emit(state, reason) {
          for (const listener of listeners) listener(state, reason);
        },
        ready: async () => {},
        dispose: async () => {
          log.push({ op: 'dispose', id });
          if (disposeRejects) throw new Error('observer already gone');
        },
      };
    },
  };
  return { worker, log };
}

const INDEX = fs.readFileSync(new URL('../src/index.mjs', import.meta.url), 'utf8');

test('routes: one http::on-request observer per route, args { method, path } with a leading slash', () => {
  const { worker, log } = fakeWorker();
  const handles = observeRoutes(worker);
  assert.equal(handles.length, ROUTES.length);
  assert.equal(log.length, ROUTES.length);
  for (const [i, entry] of log.entries()) {
    const { function_id, method, path } = ROUTES[i];
    assert.deepEqual(entry, {
      op: 'observe',
      id: handles[i].id,
      source: { function_id: 'http::on-request', args: { method, path } },
      target: function_id,
    });
    assert.ok(path.startsWith('/openwiki'), path);
    assert.ok(['GET', 'POST', 'PUT', 'DELETE'].includes(method), method);
    assert.match(function_id, /^openwiki::http::/);
  }
  // No two observers claim the same route.
  const keys = ROUTES.map((r) => `${r.method} ${r.path}`);
  assert.equal(new Set(keys).size, keys.length);
});

test('routes: every route targets a function index.mjs registers, and index.mjs observes them', () => {
  const registered = new Set([...INDEX.matchAll(/registerFunction\(\s*'([^']+)'/g)].map((m) => m[1]));
  for (const { function_id } of ROUTES) assert.ok(registered.has(function_id), `${function_id} is registered`);
  assert.match(INDEX, /^observeRoutes\(worker\);$/m);
  assert.ok(registered.has('openwiki::cron::refresh-due'));
  assert.ok(registered.has('openwiki::on-turn-completed'));
  assert.ok(registered.has('openwiki::on-turn-started'));
});

test('routes: a rejected route is logged, not thrown', (t) => {
  const warn = t.mock.method(console, 'warn', () => {});
  const { worker } = fakeWorker();
  const [handle] = observeRoutes(worker, [ROUTES[0]]);
  handle.emit('active');
  assert.equal(warn.mock.callCount(), 0);
  handle.emit('error', { code: 'HTTP_ROUTE_CONFLICT', message: 'taken' });
  assert.equal(warn.mock.callCount(), 1);
  assert.deepEqual(warn.mock.calls[0].arguments.slice(1), ['GET', '/openwiki', 'HTTP_ROUTE_CONFLICT', 'taken']);
});

test('schedule: a preset observes cron::on-tick with its expression and metadata { wiki_id }', () => {
  const { worker, log } = fakeWorker();
  const schedules = createWikiSchedules(worker);
  schedules.apply('w1', '6h');
  assert.deepEqual(log, [
    {
      op: 'observe',
      id: 'observer-1',
      source: { function_id: 'cron::on-tick', args: { expression: '0 0 */6 * * *' } },
      target: 'openwiki::cron::refresh-due',
      options: { metadata: { wiki_id: 'w1' } },
    },
  ]);
  assert.equal(schedules.handles.get('w1').id, 'observer-1');
});

test('schedule: every preset maps to a 6-field cron expression', () => {
  const { worker, log } = fakeWorker();
  const schedules = createWikiSchedules(worker);
  for (const preset of SCHEDULE_PRESETS.filter((p) => p !== 'off')) {
    schedules.apply(`w-${preset}`, preset);
  }
  const expressions = log.map((e) => e.source.args.expression);
  assert.deepEqual(
    expressions,
    Object.values(REFRESH_INTERVALS).map((i) => i.cron),
  );
  for (const expression of expressions) assert.ok(isRawCron(expression), expression);
});

test('schedule: a raw cron string is observed as given', () => {
  const { worker, log } = fakeWorker();
  createWikiSchedules(worker).apply('w2', '*/15 * * * *');
  assert.deepEqual(log[0].source, { function_id: 'cron::on-tick', args: { expression: '*/15 * * * *' } });
  assert.deepEqual(log[0].options, { metadata: { wiki_id: 'w2' } });
});

test('schedule: a new schedule is observed before the old one is disposed', async () => {
  const { worker, log } = fakeWorker();
  const schedules = createWikiSchedules(worker);
  schedules.apply('w1', '6h');
  schedules.apply('w1', 'daily');
  // The same expression again: the cron worker keeps that expression's job,
  // because the new observer shares it before the old one goes.
  schedules.apply('w1', 'daily');
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(
    log.map((e) => (e.op === 'observe' ? `observe ${e.id} ${e.source.args.expression}` : `dispose ${e.id}`)),
    [
      'observe observer-1 0 0 */6 * * *',
      'observe observer-2 0 0 3 * * *',
      'dispose observer-1',
      'observe observer-3 0 0 3 * * *',
      'dispose observer-2',
    ],
  );
  assert.equal(schedules.handles.get('w1').id, 'observer-3');
});

test("schedule: 'off' disposes the wiki's observer and observes nothing", async () => {
  const { worker, log } = fakeWorker();
  const schedules = createWikiSchedules(worker);
  schedules.apply('w1', 'off');
  assert.deepEqual(log, []);
  schedules.apply('w1', '3h');
  schedules.apply('w1', 'off');
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(
    log.map((e) => `${e.op} ${e.id}`),
    ['observe observer-1', 'dispose observer-1'],
  );
  assert.equal(schedules.handles.has('w1'), false);
});

test('schedule: schedules of different wikis are independent', async () => {
  const { worker, log } = fakeWorker();
  const schedules = createWikiSchedules(worker);
  schedules.apply('w1', '6h');
  schedules.apply('w2', '6h');
  schedules.apply('w1', 'off');
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(
    log.map((e) => `${e.op} ${e.id}`),
    ['observe observer-1', 'observe observer-2', 'dispose observer-1'],
  );
  assert.equal(schedules.handles.get('w2').id, 'observer-2');
});

test('schedule: a dispose that fails is swallowed', async () => {
  const { worker, log } = fakeWorker({ disposeRejects: true });
  const schedules = createWikiSchedules(worker);
  schedules.apply('w1', '6h');
  schedules.apply('w1', 'off');
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(
    log.map((e) => e.op),
    ['observe', 'dispose'],
  );
});

test('schedule: an observer the cron worker rejects is logged with the wiki id', (t) => {
  const warn = t.mock.method(console, 'warn', () => {});
  const { worker } = fakeWorker();
  const schedules = createWikiSchedules(worker);
  schedules.apply('w1', '* * * * * * *x');
  schedules.handles.get('w1').emit('error', { code: 'INVALID_CRON_EXPRESSION', message: 'bad' });
  assert.equal(warn.mock.callCount(), 1);
  assert.deepEqual(warn.mock.calls[0].arguments.slice(1), ['w1', 'INVALID_CRON_EXPRESSION', 'bad']);
});

test('schedule: set-schedule and delete in index.mjs go through the wiki schedules', () => {
  assert.match(INDEX, /const wikiSchedules = createWikiSchedules\(worker\);/);
  assert.match(INDEX, /const applyWikiSchedule = wikiSchedules\.apply;/);
  assert.match(INDEX, /'openwiki::set-schedule'[\s\S]*?applyWikiSchedule\(id, schedule\);/);
  assert.equal((INDEX.match(/applyWikiSchedule\(id, 'off'\)/g) || []).length, 2);
});

test('turn events: harness::on-turn-completed and on-turn-started, deliver output, no filters', () => {
  const { worker, log } = fakeWorker();
  const handles = observeTurnEvents(worker);
  assert.equal(handles.length, 2);
  assert.deepEqual(
    log.map(({ source, target, options }) => ({ source, target, options })),
    [
      {
        source: { function_id: 'harness::on-turn-completed', deliver: 'output' },
        target: 'openwiki::on-turn-completed',
        options: undefined,
      },
      {
        source: { function_id: 'harness::on-turn-started', deliver: 'output' },
        target: 'openwiki::on-turn-started',
        options: undefined,
      },
    ],
  );
  assert.match(INDEX, /const turnObservers = observeTurnEvents\(worker\);/);
});
