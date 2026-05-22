import { $createParagraphNode, $createTextNode, $getRoot } from 'lexical'
import { useState } from 'react'
import { Composer } from '@/components/chat/Composer'
import { $createFunctionMentionNode } from '@/components/chat/lexical/FunctionMentionNode'
import { STATIC_FUNCTIONS } from '@/lib/functions'
import type { Attachment, Mode, ModelId } from '@/types/chat'
import { STATIC_MODEL_OPTIONS } from '@/types/chat'
import { Section, VariantCard } from '../Section'

const noop = () => {}

const sampleAttachments: Attachment[] = [
  { id: 'spec', name: 'spec.md', size: 4_312, type: 'text/markdown' },
  { id: 'arch', name: 'architecture.svg', size: 18_440, type: 'image/svg+xml' },
]

/** Seed the editor with a sentence containing a function mention. */
function seedWithMention() {
  const root = $getRoot()
  root.clear()
  const para = $createParagraphNode()
  para.append($createTextNode('ping '))
  para.append($createFunctionMentionNode('engine::echo'))
  para.append($createTextNode(' with my prompt'))
  root.append(para)
}

/** Seed the editor with plain text only. */
function seedWithText() {
  const root = $getRoot()
  root.clear()
  const para = $createParagraphNode()
  para.append($createTextNode('how do i wire the engine into the agent loop?'))
  root.append(para)
}

export function ComposerVariantsSection() {
  const [mode, setMode] = useState<Mode>('agent')
  const [model, setModel] = useState<ModelId>(STATIC_MODEL_OPTIONS[0].id)

  return (
    <Section
      id="composer-variants"
      eyebrow="composer"
      title="composer variants"
      description="live lexical instances. type @ in the default one to insert a function mention."
    >
      <div className="flex flex-col gap-6">
        <VariantCard label="default (empty)">
          <Composer
            mode={mode}
            model={model}
            modelOptions={STATIC_MODEL_OPTIONS}
            functionEntries={STATIC_FUNCTIONS}
            autoAccept={false}
            onModeChange={setMode}
            onModelChange={setModel}
            onAutoAcceptChange={noop}
            onSubmit={noop}
          />
        </VariantCard>

        <VariantCard label="pre-seeded with plain text">
          <Composer
            mode="ask"
            model="anthropic::claude-opus-4-7"
            modelOptions={STATIC_MODEL_OPTIONS}
            functionEntries={STATIC_FUNCTIONS}
            autoAccept={false}
            onModeChange={noop}
            onModelChange={noop}
            onAutoAcceptChange={noop}
            onSubmit={noop}
            initialContent={seedWithText}
          />
        </VariantCard>

        <VariantCard
          label="pre-seeded with a function mention"
          className="@3xl:col-span-2"
        >
          <Composer
            mode="agent"
            model="openai::gpt-5"
            modelOptions={STATIC_MODEL_OPTIONS}
            functionEntries={STATIC_FUNCTIONS}
            autoAccept={false}
            onModeChange={noop}
            onModelChange={noop}
            onAutoAcceptChange={noop}
            onSubmit={noop}
            initialContent={seedWithMention}
          />
        </VariantCard>

        <VariantCard label="with attachments">
          <Composer
            mode="agent"
            model="openai::gpt-5"
            modelOptions={STATIC_MODEL_OPTIONS}
            functionEntries={STATIC_FUNCTIONS}
            autoAccept={false}
            onModeChange={noop}
            onModelChange={noop}
            onAutoAcceptChange={noop}
            onSubmit={noop}
            initialAttachments={sampleAttachments}
          />
        </VariantCard>

        <VariantCard label="disabled (streaming)">
          <Composer
            mode="plan"
            model="openai::gpt-5-mini"
            modelOptions={STATIC_MODEL_OPTIONS}
            functionEntries={STATIC_FUNCTIONS}
            autoAccept={false}
            onModeChange={noop}
            onModelChange={noop}
            onAutoAcceptChange={noop}
            onSubmit={noop}
            onStop={noop}
            isStreaming
          />
        </VariantCard>
      </div>
    </Section>
  )
}
