import assert from 'node:assert/strict';
import { test } from 'node:test';

import { FUNCTION_FORMATS } from '../src/formats.js';
import { createVerdictEmitter, ON_VERDICT_FN, toVerdictEvent } from '../src/verdict-events.js';
import { BoundedMap } from '../src/stores.js';
import {
  createVerifyHandler,
  VerifyCoalescer,
  VerifyCoalescerSaturationError,
} from '../src/verify.js';

function testDeps(overrides = {}) {
  return {
    forwardCall: async () => ({ ok: true }),
    leaseStores: new BoundedMap(32),
    governance: new BoundedMap(32),
    coalescer: new VerifyCoalescer(),
    resolveVerdictKeyringPath: () => '/tmp/keyring.json',
    resolveLeaseStorePath: () => '/tmp/leases.json',
    emitVerdict: async () => {},
    ...overrides,
  };
}

test('gantry::on-verdict returns the event it is called with', () => {
  const { handler } = createVerdictEmitter({ call: async () => {}, isObserved: () => true });
  const event = toVerdictEvent({ status: 'passed' });
  assert.equal(handler(event), event);
});

test('verdict emitter calls gantry::on-verdict fire-and-forget while observed', async () => {
  const calls = [];
  const { emit } = createVerdictEmitter({
    call: async (request) => {
      calls.push(request);
    },
    isObserved: () => true,
  });
  await emit({
    status: 'passed',
    repo_root: '/tmp/repo',
    msn_id: 'MSN-0001',
    mission_rel_path: '.gitagent/missions/MSN-0001.yaml',
  });
  assert.equal(calls.length, 1, 'expected one call');
  assert.equal(calls[0]?.function_id, ON_VERDICT_FN);
  assert.equal(ON_VERDICT_FN, 'gantry::on-verdict');
  assert.deepEqual(calls[0]?.action, { type: 'void' });
  assert.deepEqual(calls[0]?.payload, {
    status: 'passed',
    error_code: null,
    repo_root: '/tmp/repo',
    msn_id: 'MSN-0001',
    mission_rel_path: '.gitagent/missions/MSN-0001.yaml',
  });
});

test('verdict emitter makes no call while nothing observes gantry::on-verdict', async () => {
  const calls = [];
  const { emit } = createVerdictEmitter({
    call: async (request) => {
      calls.push(request);
    },
    isObserved: () => false,
  });
  await emit({ status: 'failed', error_code: 'GATE_FAILED' });
  assert.equal(calls.length, 0, 'an unobserved verdict should not be sent');
});

test('verdict emitter ignores a refused call', async () => {
  const { emit } = createVerdictEmitter({
    call: async () => {
      throw new Error('OVERFLOW');
    },
    isObserved: () => true,
  });
  await assert.doesNotReject(() => emit({ status: 'passed' }));
});

test('gantry::on-verdict is registered private with the verdict event as both formats', () => {
  const formats = FUNCTION_FORMATS[ON_VERDICT_FN];
  assert.equal(formats?.private, true);
  assert.equal(formats?.request, formats?.response);
  assert.doesNotThrow(() => formats.request.parse(toVerdictEvent({ status: 'passed' })));
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
