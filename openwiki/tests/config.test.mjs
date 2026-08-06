import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as configuration from '../src/lib/configuration.mjs';

test('config has sane defaults', () => {
  const d = configuration.defaults();
  assert.equal(typeof d.model, 'string');
  assert.ok(d.max_parallel >= 1);
});

test('fetchConfig merges the stored value over defaults', async () => {
  const worker = {
    async trigger({ function_id }) {
      if (function_id === 'configuration::get') return { value: { max_parallel: 7 } };
      return {};
    },
  };
  const c = await configuration.fetchConfig(worker);
  assert.equal(c.max_parallel, 7);
  assert.ok(c.model, 'default model preserved when not overridden');
});

test('fetchConfig falls back to defaults when the config worker errors', async () => {
  const worker = {
    async trigger() {
      throw new Error('no configuration worker');
    },
  };
  const c = await configuration.fetchConfig(worker);
  assert.equal(c.max_parallel, configuration.defaults().max_parallel);
});
