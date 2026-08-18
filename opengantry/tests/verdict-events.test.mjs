import assert from 'node:assert/strict';
import { test } from 'node:test';

import { createVerdictEmitter } from '../src/verdict-events.js';
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
