import assert from 'node:assert/strict';
import { test } from 'node:test';

import { InvocationError } from 'iii-sdk';

import {
  createVerdictEmitter,
  VERDICT_ARGS_FORMAT,
  VERDICT_OBSERVABLE,
  VERDICT_TRIGGER,
} from '../src/verdict-events.js';
import { BoundedMap } from '../src/stores.js';
import {
  createVerifyHandler,
  VerifyCoalescer,
  VerifyCoalescerSaturationError,
} from '../src/verify.js';

function testDeps(overrides = {}) {
  return {
    forwardTrigger: async () => ({ ok: true }),
    leaseStores: new BoundedMap(32),
    governance: new BoundedMap(32),
    coalescer: new VerifyCoalescer(),
    resolveVerdictKeyringPath: () => '/tmp/keyring.json',
    resolveLeaseStorePath: () => '/tmp/leases.json',
    emitVerdict: async () => {},
    ...overrides,
  };
}

test('verdict emitter fans out to registered bindings', async () => {
  const calls = [];
  const { handler, emit } = createVerdictEmitter({
    trigger: async (request) => {
      calls.push(request);
      return { ok: true };
    },
  });
  await handler.registerTrigger({
    id: 'audit-1',
    function_id: 'audit::on-verdict',
    config: {},
  });
  await emit({
    status: 'passed',
    repo_root: '/tmp/repo',
    msn_id: 'MSN-0001',
    mission_rel_path: '.gitagent/missions/MSN-0001.yaml',
  });
  assert.equal(calls.length, 1, 'expected one fan-out call');
  assert.equal(calls[0]?.function_id, 'audit::on-verdict');
  assert.equal(calls[0]?.payload?.status, 'passed');
});

test('verdict emitter stops fan-out after unregister', async () => {
  const calls = [];
  const { handler, emit } = createVerdictEmitter({
    trigger: async (request) => {
      calls.push(request);
      return { ok: true };
    },
  });
  await handler.registerTrigger({
    id: 'audit-1',
    function_id: 'audit::on-verdict',
    config: {},
  });
  await handler.unregisterTrigger({
    id: 'audit-1',
    function_id: 'audit::on-verdict',
    config: {},
  });
  await emit({ status: 'failed', error_code: 'GATE_FAILED' });
  assert.equal(calls.length, 0, 'unregistered binding should not receive events');
});

test('verdict emitter ignores subscriber failures', async () => {
  const { handler, emit } = createVerdictEmitter({
    trigger: async () => {
      throw new Error('subscriber down');
    },
  });
  await handler.registerTrigger({
    id: 'audit-1',
    function_id: 'audit::on-verdict',
    config: {},
  });
  await assert.doesNotReject(() => emit({ status: 'passed' }));
});

test('verify handler emits verdict event on saturation failure', async () => {
  const events = [];
  const coalescer = {
    async run(_key, _fn) {
      throw new VerifyCoalescerSaturationError();
    },
  };
  const verify = createVerifyHandler(
    testDeps({
      coalescer,
      emitVerdict: async (event) => {
        events.push(event);
      },
    }),
  );
  const result = await verify({ repo_root: '/tmp/repo' });
  assert.equal(result.error_code, 'GXT_VERIFY_SATURATED');
  assert.equal(events.length, 1, 'saturation failure should emit verdict event');
  assert.equal(events[0]?.status, 'failed');
  assert.equal(events[0]?.error_code, 'GXT_VERIFY_SATURATED');
});

test('verify handler emits even when emitVerdict throws', async () => {
  const coalescer = {
    async run(_key, _fn) {
      throw new VerifyCoalescerSaturationError();
    },
  };
  const verify = createVerifyHandler(
    testDeps({
      coalescer,
      emitVerdict: async () => {
        throw new Error('emitter broken');
      },
    }),
  );
  const result = await verify({ repo_root: '/tmp/repo' });
  assert.equal(result.error_code, 'GXT_VERIFY_SATURATED');
});

/** A stand-in for the SDK's Observer: `call` records what it is handed. */
function fakeObserver(id, { fail } = {}) {
  const received = [];
  return {
    id,
    function_id: VERDICT_OBSERVABLE,
    target: `test::observer::${id}`,
    args: {},
    received,
    async call(payload) {
      if (fail) throw fail;
      received.push(structuredClone(payload));
      return null;
    },
  };
}

test('gantry::on-verdict is owner-delivered with the trigger config as args', () => {
  const { observe } = createVerdictEmitter({ trigger: async () => ({ ok: true }) });
  assert.equal(VERDICT_OBSERVABLE, 'gantry::on-verdict');
  assert.equal(VERDICT_TRIGGER, 'gantry::verdict');
  assert.equal(observe.delivery, 'owner');
  assert.equal(observe.args_format, VERDICT_ARGS_FORMAT);
  assert.equal(VERDICT_ARGS_FORMAT.type, 'object');
  assert.equal(typeof observe.onObserverAdded, 'function');
  assert.equal(typeof observe.onObserverRemoved, 'function');
});

test('gantry::on-verdict refuses direct invocation with FORBIDDEN', async () => {
  const calls = [];
  const { onVerdict } = createVerdictEmitter({
    trigger: async (request) => {
      calls.push(request);
      return { ok: true };
    },
  });
  for (const payload of [{}, { status: 'passed' }, undefined]) {
    await assert.rejects(
      () => onVerdict(payload),
      (err) => {
        assert.ok(err instanceof InvocationError, String(err));
        assert.equal(err.code, 'FORBIDDEN');
        assert.match(err.message, /gantry::on-verdict/);
        return true;
      },
    );
  }
  assert.equal(calls.length, 0, 'a refused call delivers nothing');
});

test('an observer of gantry::on-verdict receives the verdict a binding receives', async () => {
  const calls = [];
  const { handler, observe, emit } = createVerdictEmitter({
    trigger: async (request) => {
      calls.push(request);
      return { ok: true };
    },
  });
  await handler.registerTrigger({ id: 'audit-1', function_id: 'audit::on-verdict', config: {} });
  const observer = fakeObserver('o1');
  await observe.onObserverAdded(observer);

  await emit({ status: 'failed', error_code: 'GATE_FAILED', repo_root: '/tmp/repo' });

  assert.equal(calls.length, 1, 'the binding still gets its call');
  assert.equal(observer.received.length, 1);
  assert.deepEqual(observer.received[0], calls[0].payload);
  assert.deepEqual(observer.received[0], {
    status: 'failed',
    error_code: 'GATE_FAILED',
    repo_root: '/tmp/repo',
    msn_id: null,
    mission_rel_path: null,
  });
  // The observer is not routed through worker.trigger().
  assert.ok(calls.every((c) => c.function_id === 'audit::on-verdict'));
});

test('a removed observer receives no more verdicts; the binding still does', async () => {
  const calls = [];
  const { handler, observe, emit } = createVerdictEmitter({
    trigger: async (request) => {
      calls.push(request);
      return { ok: true };
    },
  });
  await handler.registerTrigger({ id: 'audit-1', function_id: 'audit::on-verdict', config: {} });
  const observer = fakeObserver('o1');
  await observe.onObserverAdded(observer);
  await emit({ status: 'passed' });
  await observe.onObserverRemoved(observer);
  await emit({ status: 'passed' });
  assert.equal(observer.received.length, 1);
  assert.equal(calls.length, 2);
});

test('an observer id never collides with a binding id', async () => {
  const calls = [];
  const { handler, observe, emit } = createVerdictEmitter({
    trigger: async (request) => {
      calls.push(request);
      return { ok: true };
    },
  });
  await handler.registerTrigger({ id: 'same', function_id: 'audit::on-verdict', config: {} });
  const observer = fakeObserver('same');
  await observe.onObserverAdded(observer);
  await emit({ status: 'passed' });
  assert.equal(calls.length, 1);
  assert.equal(observer.received.length, 1);
  // Unbinding the trigger leaves the observer in place, and vice versa.
  await handler.unregisterTrigger({ id: 'same', function_id: 'audit::on-verdict', config: {} });
  await emit({ status: 'passed' });
  assert.equal(calls.length, 1);
  assert.equal(observer.received.length, 2);
});

test('a skipping or failing observer never fails the emit nor starves the others', async () => {
  const { observe, emit } = createVerdictEmitter({ trigger: async () => ({ ok: true }) });
  const skipped = fakeObserver('skip', {
    fail: Object.assign(new Error('skip'), { code: 'FILTERS_NOT_MATCHED' }),
  });
  const broken = fakeObserver('broken', { fail: new Error('target gone') });
  const ok = fakeObserver('ok');
  for (const obs of [skipped, broken, ok]) await observe.onObserverAdded(obs);
  await assert.doesNotReject(() => emit({ status: 'passed' }));
  assert.equal(ok.received.length, 1);
  assert.equal(ok.received[0].status, 'passed');
});
