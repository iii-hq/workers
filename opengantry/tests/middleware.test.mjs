import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';

import { createMiddlewareHandler } from '../src/lib/middleware.js';

test('middleware denies promote without a verify pass', async () => {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'og-mw-'));
  const state = {
    leaseStores: new Map(),
    forwardTrigger: async (fid, payload) => ({ ok: true, fid, payload }),
  };
  const middleware = createMiddlewareHandler(state);
  const result = await middleware({
    function_id: 'demo::promote',
    payload: {},
    context: { msn_id: 'MSN-0175', worktree_path: repoRoot },
  });
  assert.equal(result.status, 'failed');
});
