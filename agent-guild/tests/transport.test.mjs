import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createOperations } from '../dist/lib/worker.js';
import { observed, target } from './fixtures.mjs';

const run = (fetch) => createOperations({ fetch, timeoutMs: 100 }).preflight({ url: target });
for (const [name, body, type] of [
  ['duplicate', '{"target":"one","target":"two"}', 'application/json'],
  ['malformed', '{', 'application/json'],
  ['oversized', ' '.repeat(65537), 'application/json'],
  ['nonfinite', '1e999', 'application/json'],
  ['wrong MIME', '{}', 'text/html'],
])
  test(`response ${name}`, async () => {
    const result = await run(async () => new Response(body, { headers: { 'content-type': type } }));
    assert.equal(result.status, 'rejected');
  });
for (const status of [301, 400, 402, 429, 500])
  test(`HTTP ${status} no retries`, async () => {
    let calls = 0;
    const result = await run(async () => {
      calls++;
      return new Response('{}', { status, headers: { 'content-type': 'application/json' } });
    });
    assert.equal(result.status, 'unavailable');
    assert.equal(calls, 1);
  });
test('network failure is fixed no remote prose', async () => {
  let calls = 0;
  const r = await run(async () => {
    calls++;
    throw Error('secret remote prose');
  });
  assert.equal(r.code, 'request_unavailable');
  assert.equal(calls, 1);
  assert(!JSON.stringify(r).includes('secret'));
});
test('deadline bounds waiting and sends abort for uncooperative override', async () => {
  let signal;
  const before = Date.now();
  const result = await run(async (_url, init) => {
    signal = init.signal;
    return new Promise(() => {});
  });
  assert.equal(result.code, 'request_timeout');
  assert(signal.aborted);
  assert(Date.now() - before < 1000);
});
test('deadline includes stalled body and attempts stream cancellation', async () => {
  let cancelled = false;
  const result = await run(
    async () =>
      new Response(
        new ReadableStream({
          start(c) {
            c.enqueue(new TextEncoder().encode('{'));
          },
          cancel() {
            cancelled = true;
          },
        }),
        { headers: { 'content-type': 'application/json' } },
      ),
  );
  assert.equal(result.code, 'request_timeout');
  assert(cancelled);
});
test('explicit stop signal cancels operation', async () => {
  const c = new AbortController();
  c.abort();
  let calls = 0;
  const ops = createOperations({
    fetch: async () => {
      calls++;
      throw Error();
    },
  });
  assert.equal((await ops.preflight({ url: target }, c.signal)).code, 'request_aborted');
  assert.equal(calls, 0);
});
test('late response cannot convert timeout into observation', async () => {
  let resolve;
  const operation = run(
    async () =>
      new Promise((r) => {
        resolve = r;
      }),
  );
  const result = await operation;
  assert.equal(result.code, 'request_timeout');
  resolve(
    new Response(JSON.stringify(observed()), { headers: { 'content-type': 'application/json' } }),
  );
  await new Promise((r) => setTimeout(r, 10));
  assert.equal(result.code, 'request_timeout');
});
test('redirected response rejected', async () => {
  const response = new Response('{}', { headers: { 'content-type': 'application/json' } });
  Object.defineProperty(response, 'redirected', { value: true });
  assert.equal((await run(async () => response)).code, 'http_unavailable');
});
