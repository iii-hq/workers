import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createOperations, withoutEngineCaller } from '../dist/lib/worker.js';
import { jsonResponse, observed, passportInput, target, verifier } from './fixtures.mjs';

const uuid = '01234567-89ab-4cde-8abc-0123456789ab';
test('only well-shaped exact engine caller metadata is consumed', async () => {
  let calls = 0;
  const op = createOperations({
    fetch: async () => {
      calls++;
      return jsonResponse(observed());
    },
  });
  assert.equal(
    (await op.preflight(withoutEngineCaller({ url: target, _caller_worker_id: uuid }))).status,
    'observed',
  );
  assert.equal((await op.preflight(withoutEngineCaller({ url: target }))).status, 'observed');
  for (const bad of ['', null, 42, 'spoof', `${uuid}x`])
    assert.equal(
      (await op.preflight(withoutEngineCaller({ url: target, _caller_worker_id: bad }))).code,
      'invalid_input',
    );
  assert.equal(
    (await op.preflight(withoutEngineCaller({ url: target, _caller_worker_id: uuid, _other: 1 })))
      .code,
    'invalid_input',
  );
  assert.equal(
    (await op.preflight({ url: target, _caller_worker_id: uuid })).code,
    'invalid_input',
  );
  assert.equal(calls, 2);
});
test('engine metadata handling never invokes accessors or drops credential claims', async () => {
  let getters = 0;
  const accessor = { url: target };
  Object.defineProperty(accessor, '_caller_worker_id', {
    enumerable: true,
    get() {
      getters++;
      return uuid;
    },
  });
  const op = createOperations({
    fetch: async () => {
      throw Error('must not request');
    },
  });
  assert.equal((await op.preflight(withoutEngineCaller(accessor))).code, 'invalid_input');
  assert.equal(getters, 0);
  const input = passportInput();
  input.credential.credentialSubject._caller_worker_id = 'public nested claim';
  let sent;
  const check = createOperations({
    fetch: async (_url, init) => {
      sent = JSON.parse(init.body);
      return jsonResponse(verifier());
    },
  });
  assert.equal(
    (await check.verifyPassport(withoutEngineCaller({ ...input, _caller_worker_id: uuid })))
      .verified,
    true,
  );
  assert.deepEqual(sent, input.credential);
});
