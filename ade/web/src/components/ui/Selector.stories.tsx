import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { Selector } from './Selector'

type Model = 'claude' | 'gpt' | 'gemini' | 'local'

const GROUPS = [
  {
    label: 'hosted',
    options: [
      {
        value: 'claude' as const,
        label: 'Claude Sonnet 4.5',
        description: 'Anthropic · balanced reasoning and coding',
        keywords: ['anthropic', 'reasoning'],
      },
      {
        value: 'gpt' as const,
        label: 'GPT-5 Codex',
        description: 'OpenAI · long-running software engineering',
        keywords: ['openai', 'code'],
      },
      {
        value: 'gemini' as const,
        label: 'Gemini 2.5 Pro',
        description: 'Google · unavailable in this workspace',
        disabled: true,
      },
    ],
  },
  {
    label: 'local',
    options: [
      {
        value: 'local' as const,
        label: 'local/instrument-panel-model-with-a-deliberately-long-name',
        description:
          'A long description that verifies clipping inside a narrow 200%-zoom-shaped pane.',
      },
    ],
  },
]

const meta = {
  title: 'UI/Selector',
  parameters: { layout: 'padded' },
} satisfies Meta

export default meta
type Story = StoryObj

function SelectorDemo({ narrow = false }: { narrow?: boolean }) {
  const [value, setValue] = useState<Model | undefined>('gpt')
  return (
    <div className={narrow ? 'w-[240px]' : 'w-[360px] max-w-full'}>
      <Selector<Model>
        value={value}
        onChange={setValue}
        onClear={() => setValue(undefined)}
        allowEmpty
        emptyLabel="use workspace default"
        groups={GROUPS}
        aria-label="model"
        placeholder="choose a model…"
        searchPlaceholder="search models…"
      />
    </div>
  )
}

export const GroupedDark: Story = {
  name: 'grouped · dark · keyboard/disabled',
  render: () => <SelectorDemo />,
}

export const NarrowLightLongContent: Story = {
  name: 'narrow · light · long content',
  globals: { theme: 'light' },
  render: () => <SelectorDemo narrow />,
}

export const Loading: Story = {
  render: () => (
    <div className="w-[320px] max-w-full">
      <Selector
        value={undefined}
        onChange={() => undefined}
        options={[]}
        loading
        aria-label="loading selector"
      />
    </div>
  ),
}

export const Empty: Story = {
  render: () => (
    <div className="w-[320px] max-w-full">
      <Selector
        value={undefined}
        onChange={() => undefined}
        options={[]}
        emptyMessage="no matching workers"
        aria-label="empty selector"
      />
    </div>
  ),
}

export const ErrorAndValidation: Story = {
  render: () => (
    <div className="w-[320px] max-w-full">
      <Selector
        value={undefined}
        onChange={() => undefined}
        options={[]}
        error="catalog could not be loaded"
        invalid
        validationMessage="choose an available model"
        aria-label="invalid selector"
      />
    </div>
  ),
}
