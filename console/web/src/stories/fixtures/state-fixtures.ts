import type { FunctionCallMessage } from '@/types/chat'
import { wrapHarness } from './sandbox-fixtures'

const now = Date.now()

function base(
  id: string,
  functionId: string,
  input: unknown,
  output?: unknown,
  extra?: Partial<FunctionCallMessage>,
): FunctionCallMessage {
  return {
    id,
    role: 'function-call',
    functionId,
    input,
    output,
    durationMs: 2,
    createdAt: now,
    ...extra,
  }
}

/* ---------------- state::get ---------------- */

export const stateGetValue = base(
  'state-get-value',
  'state::get',
  { scope: 'ops', key: 'build' },
  wrapHarness({ commit: 'abc123', status: 'green' }),
)

export const stateGetMissing = base(
  'state-get-missing',
  'state::get',
  { scope: 'ops', key: 'nonexistent' },
  wrapHarness(null),
)

export const stateGetRunning = base(
  'state-get-running',
  'state::get',
  { scope: 'ops', key: 'build' },
  undefined,
  { running: true },
)

/* ---------------- state::set / delete ---------------- */

export const stateSet = base(
  'state-set',
  'state::set',
  { scope: 'ops', key: 'build', value: { status: 'green' } },
  wrapHarness({ old_value: { status: 'red' }, new_value: { status: 'green' } }),
)

export const stateDelete = base(
  'state-delete',
  'state::delete',
  { scope: 'ops', key: 'build' },
  wrapHarness({ status: 'green' }),
)

/* ---------------- state::list_groups ---------------- */

export const stateListGroups = base(
  'state-list-groups',
  'state::list_groups',
  {},
  wrapHarness({ groups: ['ops', 'build', 'sessions'] }),
)

export const stateFixtures = [
  stateGetValue,
  stateGetMissing,
  stateGetRunning,
  stateSet,
  stateDelete,
  stateListGroups,
] as const
