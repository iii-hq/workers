import type { ChatBackend } from '@/lib/backend'
import type { Mode } from '@/types/chat'
import { abortMidThought } from './abort-mid-thought'
import { coderMutate } from './coder-mutate'
import { coderUpdate } from './coder-update'
import { errorOnFcall } from './error-on-fcall'
import { fastTokens } from './fast-tokens'
import { happyAgent } from './happy-agent'
import { happyAsk } from './happy-ask'
import { happyPlan } from './happy-plan'
import { longMarkdown } from './long-markdown'
import { markdownStress } from './markdown-stress'
import { multiFunctionAgent } from './multi-function-agent'
import { pendingApproval } from './pending-approval'
import { slowTokens } from './slow-tokens'

export type ScenarioGroup =
  | 'happy paths'
  | 'timing'
  | 'failure modes'
  | 'markdown'
  | 'agent'

export interface PlaygroundScenario {
  id: string
  label: string
  description: string
  group: ScenarioGroup
  /** mode the chat surface should be put into to make the scenario read right */
  preferredMode?: Mode
  backend: ChatBackend
}

export const SCENARIOS: PlaygroundScenario[] = [
  {
    id: 'happy-plan',
    label: 'happy · plan',
    description: 'thought + assistant body, no function triggers.',
    group: 'happy paths',
    preferredMode: 'plan',
    backend: happyPlan,
  },
  {
    id: 'happy-ask',
    label: 'happy · ask',
    description: 'assistant body only, no thought, no function triggers.',
    group: 'happy paths',
    preferredMode: 'ask',
    backend: happyAsk,
  },
  {
    id: 'happy-agent',
    label: 'happy · agent',
    description: 'thought + one function trigger + assistant body.',
    group: 'happy paths',
    preferredMode: 'agent',
    backend: happyAgent,
  },
  {
    id: 'coder-mutate',
    label: 'coder · mutate',
    description:
      'coder::tree scout, then coder::create-file writing workers/iii/skills/SKILL.md.',
    group: 'happy paths',
    preferredMode: 'agent',
    backend: coderMutate,
  },
  {
    id: 'coder-update',
    label: 'coder · update',
    description:
      'coder::search → numbered coder::read-file window → coder::update-file with post-apply echoes.',
    group: 'happy paths',
    preferredMode: 'agent',
    backend: coderUpdate,
  },
  {
    id: 'slow-tokens',
    label: 'slow tokens',
    description:
      '~200ms between assistant tokens — watch for renderer flicker.',
    group: 'timing',
    preferredMode: 'ask',
    backend: slowTokens,
  },
  {
    id: 'fast-tokens',
    label: 'fast tokens',
    description: '~5ms between assistant tokens — stresses the patch path.',
    group: 'timing',
    preferredMode: 'ask',
    backend: fastTokens,
  },
  {
    id: 'abort-mid-thought',
    label: 'abort mid-thought',
    description:
      'half a thought, then throws AbortError. ChatView should clean up and stay responsive.',
    group: 'failure modes',
    preferredMode: 'plan',
    backend: abortMidThought,
  },
  {
    id: 'error-on-fcall',
    label: 'error on fcall',
    description:
      'function trigger ends with an error payload (rate_limited) instead of data.',
    group: 'failure modes',
    preferredMode: 'agent',
    backend: errorOnFcall,
  },
  {
    id: 'multi-function-agent',
    label: 'multi-function agent',
    description:
      'three sequential function triggers before the assistant body — surfaces fcall pointer reuse.',
    group: 'agent',
    preferredMode: 'agent',
    backend: multiFunctionAgent,
  },
  {
    id: 'pending-approval',
    label: 'pending approval',
    description:
      'fcall that requires user approval; auto-resolves after a delay so you can watch the lifecycle.',
    group: 'agent',
    preferredMode: 'agent',
    backend: pendingApproval,
  },
  {
    id: 'long-markdown',
    label: 'long markdown',
    description:
      '~4kB body: headings, lists, tables, fenced code in 3 langs, blockquotes, task lists.',
    group: 'markdown',
    preferredMode: 'ask',
    backend: longMarkdown,
  },
  {
    id: 'markdown-stress',
    label: 'markdown stress',
    description:
      'pathological markdown: nested lists, footnotes, autolinks, hard breaks, busy tables.',
    group: 'markdown',
    preferredMode: 'ask',
    backend: markdownStress,
  },
]

export const SCENARIO_GROUPS: ScenarioGroup[] = [
  'happy paths',
  'agent',
  'failure modes',
  'timing',
  'markdown',
]

export function findScenario(id: string): PlaygroundScenario | undefined {
  return SCENARIOS.find((s) => s.id === id)
}
