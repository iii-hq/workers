/** Ids shared by the page, the config form and the transcript renderer. */
export const PAGE_ID = 'sentinel'
export const CONFIGURATION_ID = 'sentinel'

export const FN = {
  status: 'sentinel::status',
  groupsList: 'sentinel::groups::list',
  groupsGet: 'sentinel::groups::get',
  resolve: 'sentinel::groups::resolve',
  ignore: 'sentinel::groups::ignore',
  unignore: 'sentinel::groups::unignore',
  reopen: 'sentinel::groups::reopen',
  occurrences: 'sentinel::occurrences::list',
  evidence: 'sentinel::evidence::get',
  investigate: 'sentinel::investigate',
  investigationsGet: 'sentinel::investigations::get',
  investigationsList: 'sentinel::investigations::list',
  cancel: 'sentinel::investigations::cancel',
  record: 'sentinel::diagnosis::record',
} as const

export const EVENT = {
  groupChanged: 'sentinel::group-changed',
  investigationChanged: 'sentinel::investigation-changed',
} as const

/** Every page call is a local RPC; ten seconds is already generous. */
export const RPC_TIMEOUT_MS = 10_000
