import type { CaseGroup } from './cases.ts'
import { ERROR_CASES } from './cases-errors.ts'
import { KEEP_CASES } from './cases-keep.ts'
import { REGISTER_CASES } from './cases-register.ts'
import { RUN_CASES } from './cases-run.ts'

/** Order matters only for readability; cases are independent by construction. */
export const ALL_GROUPS: CaseGroup[] = [
  { name: 'run', cases: RUN_CASES },
  { name: 'keep', cases: KEEP_CASES },
  { name: 'register', cases: REGISTER_CASES },
  { name: 'errors', cases: ERROR_CASES },
]
