import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';

import * as observers from '../src/lib/observers.mjs';
import * as turnbus from '../src/lib/turnbus.mjs';

const INDEX = fs.readFileSync(new URL('../src/index.mjs', import.meta.url), 'utf8');

// What a capture: ['output'] observer target receives.
function envelope(functionId, output) {
  return {
    observer_id: 'obs_1',
    activation_id: 'act_1',
    function_id: functionId,
    invocation_id: 'inv_1',
    status: 'success',
    started_at_ms: 1,
    duration_ms: 0.5,
    ...(output === undefined ? {} : { output }),
  };
}

test('turn events: the target reads the event from the observed envelope', () => {
  const turn = { session_id: 'child', parent_session_id: 'root', turn_id: 't1', status: 'completed' };
  assert.equal(typeof observers.turnEventOf, 'function');
  assert.deepEqual(observers.turnEventOf(envelope('harness::on-turn-completed', turn)), turn);
  assert.equal(observers.turnEventOf(envelope('harness::on-turn-completed')), null);
  assert.equal(observers.turnEventOf(null), null);
});

test('turn events: an envelope routes to the generation that owns the parent root', () => {
  const seen = [];
  const off = turnbus.register('root-envelope', {
    onSpawn: (sid) => seen.push(['spawn', sid]),
  });
  try {
    const started = observers.turnEventOf(
      envelope('harness::on-turn-started', { session_id: 'child', parent_session_id: 'root-envelope' }),
    );
    assert.equal(turnbus.deliverStarted(started), true);
    assert.equal(turnbus.deliverStarted(observers.turnEventOf(envelope('harness::on-turn-started'))), false);
  } finally {
    off?.();
  }
  assert.deepEqual(seen, [['spawn', 'child']]);
});

test('turn events: index.mjs unwraps the envelope before routing', () => {
  assert.match(INDEX, /'openwiki::on-turn-completed',[\s\S]*?turnbus\.deliver\(turnEventOf\(evt\)\)/);
  assert.match(INDEX, /'openwiki::on-turn-started',[\s\S]*?turnbus\.deliverStarted\(turnEventOf\(evt\)\)/);
});
