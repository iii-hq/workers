import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import type { ProviderListEntry } from '@/lib/models-catalog'
import type { RegistryWorker } from '@/lib/workers-registry'
import type { ModelId, ModelOption, ThinkingLevel } from '@/types/chat'
import { AddProviderPanel } from './AddProviderPanel'
import { ModelPickerPanel } from './ModelPicker'
import { ReasoningEffortSlider } from './ReasoningEffortSlider'

/* Two real marks (Simple Icons, CC0) so the rail shows worker-declared SVGs
   next to the initial-letter fallback the other providers get. */
const ANTHROPIC_MARK =
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M17.3041 3.541h-3.6718l6.696 16.918H24Zm-10.6082 0L0 20.459h3.7442l1.3693-3.5527h7.0052l1.3693 3.5528h3.7442L10.5363 3.5409Zm-.3712 10.2232 2.2914-5.9456 2.2914 5.9456Z"/></svg>'
const OPENCODE_MARK =
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M22 24H2V0h20zM17 4.8H7v14.4h10z"/></svg>'

const PROVIDERS: ProviderListEntry[] = [
  {
    id: 'anthropic',
    display_name: 'Anthropic',
    supports_model_listing: true,
    credential_env_var: 'ANTHROPIC_API_KEY',
    configured: true,
    available: true,
    icon_svg: ANTHROPIC_MARK,
  },
  {
    id: 'openai',
    display_name: 'OpenAI',
    supports_model_listing: true,
    credential_env_var: 'OPENAI_API_KEY',
    configured: true,
    available: true,
  },
  {
    id: 'opencode_go',
    display_name: 'OpenCode Go',
    supports_model_listing: true,
    credential_env_var: 'OPENCODE_API_KEY',
    configured: true,
    available: true,
    icon_svg: OPENCODE_MARK,
  },
  {
    id: 'xai',
    display_name: 'xAI',
    supports_model_listing: true,
    credential_env_var: 'XAI_API_KEY',
    configured: false,
    available: true,
  },
  {
    id: 'deepseek',
    display_name: 'DeepSeek',
    supports_model_listing: true,
    credential_env_var: 'DEEPSEEK_API_KEY',
    configured: true,
    available: false,
  },
]

const EFFORTS = [
  { effort: 'minimal', description: 'minimal reasoning overhead' },
  { effort: 'low', description: 'quick reasoning for routine tasks' },
  { effort: 'medium', description: 'balanced reasoning and latency' },
  { effort: 'high', description: 'deeper reasoning for complex tasks' },
  { effort: 'xhigh', description: 'extended reasoning for hard tasks' },
]

const OPTIONS: ModelOption[] = [
  {
    id: 'anthropic::claude-opus-4-7',
    label: 'claude opus 4.7',
    supportsThinking: true,
  },
  {
    id: 'anthropic::claude-sonnet-4-6',
    label: 'claude sonnet 4.6',
    supportsThinking: true,
  },
  {
    id: 'anthropic::claude-haiku-4-5',
    label: 'claude haiku 4.5',
    supportsThinking: false,
  },
  {
    id: 'openai::gpt-5',
    label: 'gpt-5',
    supportsThinking: true,
    reasoningEfforts: EFFORTS,
  },
  {
    id: 'openai::gpt-5-mini',
    label: 'gpt-5 mini',
    supportsThinking: true,
    reasoningEfforts: EFFORTS,
  },
  { id: 'openai::gpt-4.1', label: 'gpt-4.1', supportsThinking: false },
  {
    id: 'opencode_go::kimi-k2',
    label: 'kimi k2 (opencode)',
    supportsThinking: false,
  },
  {
    id: 'opencode_go::qwen3-coder',
    label: 'qwen3 coder (opencode)',
    supportsThinking: false,
  },
  {
    id: 'deepseek::deepseek-reasoner',
    label: 'deepseek reasoner',
    supportsThinking: true,
  },
]

const REGISTRY: RegistryWorker[] = [
  {
    name: 'provider-openai',
    description:
      'OpenAI Responses provider worker with Chat Completions compatibility.',
    version: '1.2.10',
    tags: ['llm', 'openai', 'provider'],
    totalDownloads: 1759,
    authorName: 'iii',
    authorVerified: true,
  },
  {
    name: 'provider-anthropic',
    description: 'Anthropic Messages API provider worker.',
    version: '1.3.1',
    tags: ['llm', 'anthropic', 'provider'],
    totalDownloads: 1620,
    authorName: 'iii',
    authorVerified: true,
  },
  {
    name: 'provider-github-copilot',
    description:
      'GitHub Copilot provider worker; signs in with your GitHub account and lists the models your plan allows.',
    version: '0.9.4',
    tags: ['llm', 'provider'],
    totalDownloads: 412,
    authorName: 'iii',
    authorVerified: true,
  },
  {
    name: 'provider-llamacpp',
    description: 'llama.cpp server Chat Completions provider worker.',
    version: '0.4.0',
    tags: ['llm', 'local', 'provider'],
    totalDownloads: 233,
    authorName: 'iii',
    authorVerified: true,
  },
  {
    name: 'cursor',
    description: 'Cursor CLI provider worker using the ACP protocol.',
    version: '0.2.3',
    tags: ['provider'],
    totalDownloads: 97,
    authorName: 'iii',
    authorVerified: true,
  },
]

function PanelFrame({
  width,
  children,
}: {
  width: number
  children: React.ReactNode
}) {
  return (
    <div
      className="flex h-[560px] flex-col overflow-hidden rounded-lg border border-edge bg-panel-raised pt-4 text-ink shadow-floating"
      style={{ width }}
    >
      {children}
    </div>
  )
}

function InteractivePanel({
  width = 520,
  initialModel = 'openai::gpt-5',
  initialEffort = 'medium',
  showReasoningEffort = true,
}: {
  width?: number
  initialModel?: ModelId | null
  initialEffort?: ThinkingLevel
  showReasoningEffort?: boolean
}) {
  const [model, setModel] = useState<ModelId | null>(initialModel)
  const [effort, setEffort] = useState<ThinkingLevel>(initialEffort)
  return (
    <PanelFrame width={width}>
      <ModelPickerPanel
        value={model}
        options={OPTIONS}
        providers={PROVIDERS}
        thinkingLevel={effort}
        onChange={setModel}
        onThinkingLevelChange={setEffort}
        onConfigureProvider={() => {}}
        onAddProvider={() => {}}
        showReasoningEffort={showReasoningEffort}
      />
    </PanelFrame>
  )
}

const meta = {
  title: 'Chat/ModelPicker',
  parameters: { layout: 'centered' },
} satisfies Meta

export default meta
type Story = StoryObj

/** The desktop dropdown body: provider rail, grouped list, effort slider. */
export const Panel: Story = {
  render: () => <InteractivePanel />,
}

/** A model whose provider only advertises a thinking flag — the generic scale. */
export const GenericThinkingLevels: Story = {
  render: () => (
    <InteractivePanel
      initialModel="anthropic::claude-opus-4-7"
      initialEffort="high"
    />
  ),
}

/** A model without reasoning levels keeps the footer, disabled. */
export const FixedEffort: Story = {
  render: () => <InteractivePanel initialModel="openai::gpt-4.1" />,
}

/** Profile pickers hide the effort footer entirely. */
export const WithoutReasoningEffort: Story = {
  render: () => <InteractivePanel showReasoningEffort={false} />,
}

/** The bottom-sheet width: 44px rail targets, taller rows. */
export const Narrow: Story = {
  render: () => <InteractivePanel width={360} />,
}

/** Loading and empty catalogs still offer the add affordance. */
export const Empty: Story = {
  render: () => (
    <PanelFrame width={520}>
      <ModelPickerPanel
        value={null}
        options={[]}
        providers={[]}
        thinkingLevel="default"
        onChange={() => {}}
        onThinkingLevelChange={() => {}}
        onConfigureProvider={() => {}}
        onAddProvider={() => {}}
      />
    </PanelFrame>
  ),
}

/** The registry page: installed workers are left out, the rest offer Add. */
export const AddProvider: Story = {
  render: () => (
    <PanelFrame width={520}>
      <div className="px-4 pb-3">
        <h2 className="font-sans text-lg font-semibold text-ink">
          Add a provider
        </h2>
        <p className="font-sans text-sm text-ink-faint">
          Provider workers from the workers registry.
        </p>
      </div>
      <AddProviderPanel
        providers={PROVIDERS}
        registryWorkers={REGISTRY}
        installedWorkerNames={['provider-anthropic', 'provider-openai']}
        onConfigureProvider={() => {}}
      />
    </PanelFrame>
  ),
}

/** The slider alone, across the scale, to check the colour ramp. */
export const EffortSlider: Story = {
  render: () => {
    const options = [{ effort: 'default' }, ...EFFORTS]
    return (
      <div className="flex w-[420px] flex-col gap-6 rounded-lg bg-panel-raised p-4 text-ink shadow-floating">
        {['default', 'low', 'high', 'xhigh'].map((value) => (
          <div key={value} className="rounded-md bg-surface px-4 py-3">
            <ReasoningEffortSlider
              options={options}
              value={value}
              onChange={() => {}}
            />
          </div>
        ))}
      </div>
    )
  },
}
