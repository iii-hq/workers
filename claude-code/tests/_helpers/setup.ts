/**
 * Every test file: a headless turn checks the `iii` plugin with the real
 * `claude plugin ...` commands on this machine. No test may run them (they
 * would read and change the developer's own Claude Code settings), so the
 * runner is a fake Claude CLI here, reset before each test: `iii@iii`
 * installed for the user, so a turn finds it present. A test that cares sets
 * its own with `vi.mocked(localRun).mockImplementation(...)`.
 */

import { beforeEach, vi } from 'vitest';
import { fakeClaude } from './fake-claude.js';

vi.mock('../../src/iii-plugin/local-run.js', () => ({ localRun: vi.fn() }));

const { localRun } = await import('../../src/iii-plugin/local-run.js');

beforeEach(() => {
  const claude = fakeClaude({ installs: [{ id: 'iii@iii', scope: 'user', enabled: true }] });
  vi.mocked(localRun).mockReset();
  vi.mocked(localRun).mockImplementation(claude.run);
});
