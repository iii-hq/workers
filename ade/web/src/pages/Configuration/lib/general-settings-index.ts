import type { SettingsSearchField } from './settings-search'

/**
 * What settings search knows about the General page. The anchors are the DOM
 * ids `ConsoleSettingsTab` puts on its sections and rows: a result scrolls to
 * its row, or to the row's section while the row is hidden (the allowlist
 * row only shows in Auto mode). Keep this list in step with that page.
 */
export const GENERAL_ANCHOR = {
  appearance: 'settings-general-appearance',
  theme: 'settings-general-theme',
  notifications: 'settings-general-notifications',
  completionBell: 'settings-general-completion-bell',
  soundCheck: 'settings-general-sound-check',
  permissions: 'settings-general-permissions',
  defaultMode: 'settings-general-default-mode',
  allowlist: 'settings-general-allowlist',
  workspace: 'settings-general-workspace',
  tabLimit: 'settings-general-tab-limit',
  providers: 'settings-general-providers',
  modelProviders: 'settings-general-model-providers',
  filesystem: 'settings-general-filesystem',
  permanentRoots: 'settings-general-permanent-roots',
} as const

const A = GENERAL_ANCHOR

const BASE_FIELDS: SettingsSearchField[] = [
  {
    key: 'appearance',
    label: 'Appearance',
    trail: [],
    anchor: A.appearance,
  },
  {
    key: 'theme',
    label: 'Interface theme',
    trail: ['Appearance'],
    description: 'Applied immediately and stored for this browser.',
    keywords: ['light', 'dark', 'system', 'color scheme', 'mode'],
    anchor: A.theme,
  },
  {
    key: 'notifications',
    label: 'Notifications',
    trail: [],
    anchor: A.notifications,
  },
  {
    key: 'completion-bell',
    label: 'Completion bell',
    trail: ['Notifications'],
    description: 'Play a gentle bell when a top-level conversation finishes.',
    keywords: ['sound', 'audio', 'chime', 'tone', 'finished'],
    anchor: A.completionBell,
  },
  {
    key: 'sound-check',
    label: 'Sound check',
    trail: ['Notifications'],
    description: 'Play the success tone now.',
    keywords: ['test bell', 'audio'],
    anchor: A.soundCheck,
  },
]

const PERMISSION_FIELDS: SettingsSearchField[] = [
  {
    key: 'permissions',
    label: 'Permissions',
    trail: [],
    anchor: A.permissions,
  },
  {
    key: 'default-mode',
    label: 'Default permission mode',
    trail: ['Permissions'],
    description:
      'Manual asks before every function. Auto skips functions on the allowlist. Full skips all prompts.',
    keywords: ['manual', 'auto', 'full', 'approval', 'approve'],
    anchor: A.defaultMode,
  },
  {
    key: 'allowlist',
    label: 'Auto-mode allowlist',
    trail: ['Permissions'],
    description: 'Functions that new conversations can run without asking.',
    keywords: ['allow list', 'approval', 'approve'],
    anchor: A.allowlist,
    fallbackAnchor: A.defaultMode,
  },
]

const TRAILING_FIELDS: SettingsSearchField[] = [
  {
    key: 'workspace',
    label: 'Workspace',
    trail: [],
    anchor: A.workspace,
  },
  {
    key: 'tab-limit',
    label: 'Tab limit',
    trail: ['Workspace'],
    description: 'How many workspace tabs this ADE lets you open at once.',
    keywords: ['tabs', 'maximum'],
    anchor: A.tabLimit,
  },
  {
    key: 'providers',
    label: 'Providers',
    trail: [],
    anchor: A.providers,
  },
  {
    key: 'model-providers',
    label: 'Model providers',
    trail: ['Providers'],
    description:
      'Manage model credentials, endpoints, and provider-specific settings.',
    keywords: ['api keys', 'LLM router'],
    anchor: A.modelProviders,
  },
  {
    key: 'filesystem',
    label: 'Filesystem access',
    trail: [],
    anchor: A.filesystem,
  },
  {
    key: 'permanent-roots',
    label: 'Permanent roots',
    trail: ['Filesystem access'],
    description: 'Folders available to every conversation.',
    keywords: ['directories', 'host roots', 'grants'],
    anchor: A.permanentRoots,
  },
]

/** The General page's searchable fields; Permissions only when it renders. */
export function generalSearchFields(
  approvalGateAvailable: boolean,
): SettingsSearchField[] {
  return [
    ...BASE_FIELDS,
    ...(approvalGateAvailable ? PERMISSION_FIELDS : []),
    ...TRAILING_FIELDS,
  ]
}
