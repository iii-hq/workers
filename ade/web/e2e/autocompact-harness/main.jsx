/*
 * Auto-compact regression harness (chat-projects phase 3): the real
 * Settings › Chat › Auto-compact dialog, the real context meter and the
 * real transcript rows, over an in-memory engine (./mock-iii.js) and the
 * tab's conversations (./conversations.js). No socket, no live engine.
 */
import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { AutocompactDialog } from '@/components/chat/AutocompactDialog'
import { ContextUsage } from '@/components/chat/ContextUsage'
import { Message } from '@/components/chat/Message'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { seedChatProjectsForTests } from '@/hooks/use-chat-projects'
import { setChatCapabilitiesForTests } from '@/lib/chat-capabilities'
import { entrySegments } from '@/lib/sessions/entry-mapper'
import { PROJECTS } from './mock-iii.js'
import '@/index.css'

const parameters = new URLSearchParams(location.search)
document.documentElement.dataset.theme = parameters.get('theme') ?? 'dark'

// Phase 3 on this engine (D168), with the chat settings it writes through.
setChatCapabilitiesForTests({
  autocompact: true,
  chatSettings: true,
  projects: false,
})
seedChatProjectsForTests(PROJECTS)

const acme = {
  scope: 'project',
  project_id: 'prj_web',
  project_name: 'acme-web',
}

/* The transcript's records as the harness writes them (contract C): the
   start notice, then auto, inline and limit markers, and the two that read
   as today, /compact and a record from before phase 3. */
const records = [
  {
    entry_id: 'e-notice',
    custom: {
      custom_type: 'notice',
      data: {
        reason: 'autocompact_started',
        message: 'Auto-compacting at 160k…',
      },
    },
  },
  {
    entry_id: 'e-auto',
    custom: {
      custom_type: 'compaction',
      data: {
        summary: 'The parser fix, the open TODOs and PR 412.',
        tokens_before: 140_000,
        trigger: 'auto',
        source: 'auto',
        threshold_tokens: 160_000,
        rule: acme,
        context_before: 161_000,
        context_after: 34_000,
      },
    },
  },
  {
    entry_id: 'e-inline',
    custom: {
      custom_type: 'compaction',
      data: {
        summary: 'The webhook retry plan.',
        tokens_before: 150_000,
        trigger: 'inline',
        source: 'auto_inline',
        threshold_tokens: 160_000,
        rule: acme,
        context_before: 183_000,
        context_after: 31_000,
      },
    },
  },
  {
    entry_id: 'e-limit',
    custom: {
      custom_type: 'compaction',
      data: {
        summary: 'The migration steps.',
        tokens_before: 100_000,
        trigger: 'limit',
        source: 'limit',
        context_before: 116_000,
        context_after: 30_000,
      },
    },
  },
  {
    entry_id: 'e-command',
    custom: {
      custom_type: 'compaction',
      data: {
        summary: 'Everything before /compact.',
        tokens_before: 1_200,
        trigger: 'command',
        source: 'command',
        context_before: 9_000,
        context_after: 2_000,
      },
    },
  },
  {
    entry_id: 'e-older',
    custom: {
      custom_type: 'compaction',
      data: { summary: 'From before phase 3.', tokens_before: 98_000 },
    },
  },
]
const transcript = records.flatMap((record) => entrySegments(record))

function App() {
  const [open, setOpen] = useState(!parameters.has('closed'))
  return (
    <TooltipProvider delayDuration={0}>
      <main className="flex min-h-dvh flex-col gap-6 bg-background p-6 text-ink">
        <section data-harness-meter="" className="flex items-center gap-4">
          <ContextUsage
            messages={[]}
            reported={{ total: 142_000, usable: 916_000, free: 774_000 }}
            autocompact={{ effective: 400_000, rule: { scope: 'global' } }}
          />
          <button
            type="button"
            data-harness-open=""
            onClick={() => setOpen(true)}
            className="rounded-md bg-surface px-3 py-1 text-sm"
          >
            Open Auto-compact
          </button>
        </section>
        <section data-harness-transcript="" className="max-w-3xl">
          {transcript.map((message) => (
            <Message key={message.id} message={message} />
          ))}
        </section>
        <AutocompactDialog open={open} onOpenChange={setOpen} />
      </main>
    </TooltipProvider>
  )
}

createRoot(document.getElementById('root')).render(<App />)
