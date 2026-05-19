import { FunctionCallGroup } from '@/components/chat/FunctionCallGroup'
import { FunctionCallMessage } from '@/components/chat/FunctionCallMessage'
import { Message } from '@/components/chat/Message'
import { ThoughtMessage } from '@/components/chat/ThoughtMessage'
import type {
  AssistantMessage,
  Attachment,
  FunctionCallMessage as FCallType,
  ThoughtMessage as ThoughtType,
  UserMessage,
} from '@/types/chat'
import { Section, VariantCard } from '../Section'

const sampleAttachments: Attachment[] = [
  {
    id: 'a1',
    name: 'spec.md',
    size: 4_312,
    type: 'text/markdown',
  },
  {
    id: 'a2',
    name: 'screenshot.png',
    size: 281_402,
    type: 'image/png',
  },
]

const userPlain: UserMessage = {
  id: 'u1',
  role: 'user',
  content: 'how do i wire @fn(engine::echo) into the agent loop?',
  createdAt: Date.now(),
}

const userWithAttachments: UserMessage = {
  id: 'u2',
  role: 'user',
  content: 'please review the attached spec and screenshot.',
  attachments: sampleAttachments,
  createdAt: Date.now(),
}

const assistantComplete: AssistantMessage = {
  id: 'a1',
  role: 'assistant',
  model: 'anthropic::claude-opus-4-7',
  mode: 'plan',
  content: `## the plan

three small steps, each independently revertible:

1. register the trigger on engine startup.
2. wire the callback through the dispatcher.
3. add a smoke test that exercises the happy path.

\`\`\`ts
engine.on('echo', (input) => ({ text: input.text }))
\`\`\`

> a one-liner you can keep in your back pocket.`,
  createdAt: Date.now(),
}

const assistantStreaming: AssistantMessage = {
  id: 'a2',
  role: 'assistant',
  model: 'openai::gpt-5',
  mode: 'ask',
  content: 'sure — a btree-backed index gives you both `O(log n)` lookups and cheap range',
  streaming: true,
  createdAt: Date.now(),
}

const assistantThinking: AssistantMessage = {
  id: 'a3',
  role: 'assistant',
  model: 'anthropic::claude-opus-4-7',
  mode: 'agent',
  content: '',
  streaming: true,
  createdAt: Date.now(),
}

const thoughtBrief: ThoughtType = {
  id: 't1',
  role: 'thought',
  content: 'this is a one-line clarification. trivial to resolve, no branching to consider.',
  durationMs: 800,
  createdAt: Date.now(),
}

const thoughtLong: ThoughtType = {
  id: 't2',
  role: 'thought',
  content:
    "restating the request in one line, then enumerating constraints. the user mentioned shape and direction but not scale, so i'll plan for the smaller end of the range and flag the bigger case as a follow-up.",
  durationMs: 2300,
  createdAt: Date.now(),
}

const thoughtStreaming: ThoughtType = {
  id: 't3',
  role: 'thought',
  content: 'enumerating the constraints…',
  durationMs: 0,
  streaming: true,
  createdAt: Date.now(),
}

const fcallPendingSingle: FCallType = {
  id: 'f0a',
  role: 'function-call',
  functionId: 'engine::echo',
  input: { text: 'hello, world.' },
  pendingApproval: true,
  createdAt: Date.now(),
}

const fcallPendingMulti: FCallType = {
  id: 'f0b',
  role: 'function-call',
  functionId: 'fs::write',
  input: {
    path: '/tmp/scratch/notes.md',
    contents: '# scratch\n\njust a draft.\n',
    overwrite: true,
  },
  pendingApproval: true,
  createdAt: Date.now(),
}

const fcallRunning: FCallType = {
  id: 'f1',
  role: 'function-call',
  functionId: 'engine::echo',
  input: { text: 'hello, world.' },
  running: true,
  createdAt: Date.now(),
}

const fcallDone: FCallType = {
  id: 'f2',
  role: 'function-call',
  functionId: 'engine::echo',
  input: { text: 'hello, world.' },
  output: { text: 'hello, world.' },
  durationMs: 132,
  createdAt: Date.now(),
}

const fcallDoneMulti: FCallType = {
  id: 'f3',
  role: 'function-call',
  functionId: 'directory::index',
  input: { path: '/repo/src', recursive: true, glob: '*.ts' },
  output: {
    files: 142,
    bytes: 318_204,
    elapsedMs: 410,
  },
  durationMs: 410,
  createdAt: Date.now(),
}

/* mirrors the multi-function-agent scenario: three sequential engine calls
   so the spec sheet matches what a real fan-out turn produces. */
const fcallGroupInFlight: FCallType[] = [
  {
    id: 'g1a',
    role: 'function-call',
    functionId: 'engine::list',
    input: {},
    output: { workers: ['worker-1', 'worker-3', 'worker-7'] },
    durationMs: 450,
    createdAt: Date.now(),
  },
  {
    id: 'g1b',
    role: 'function-call',
    functionId: 'engine::info',
    input: { id: 'worker-7' },
    running: true,
    createdAt: Date.now(),
  },
  {
    id: 'g1c',
    role: 'function-call',
    functionId: 'engine::echo',
    input: { workerId: 'worker-7', text: 'ping' },
    output: { text: 'ping' },
    durationMs: 350,
    createdAt: Date.now(),
  },
]

const fcallGroupDone: FCallType[] = [
  {
    id: 'g2a',
    role: 'function-call',
    functionId: 'engine::list',
    input: {},
    output: { workers: ['worker-1', 'worker-3', 'worker-7'] },
    durationMs: 450,
    createdAt: Date.now(),
  },
  {
    id: 'g2b',
    role: 'function-call',
    functionId: 'engine::info',
    input: { id: 'worker-7' },
    output: {
      id: 'worker-7',
      load: 0.12,
      version: '0.4.1',
      skills: ['echo', 'tokenize', 'embed'],
    },
    durationMs: 500,
    createdAt: Date.now(),
  },
  {
    id: 'g2c',
    role: 'function-call',
    functionId: 'engine::echo',
    input: { workerId: 'worker-7', text: 'ping' },
    output: { text: 'ping' },
    durationMs: 350,
    createdAt: Date.now(),
  },
]

export function MessageVariantsSection() {
  return (
    <Section
      id="message-variants"
      eyebrow="messages"
      title="message variants"
      description="every renderable kind, side-by-side. fixtures only — no live state."
    >
      <div className="grid grid-cols-1 @3xl:grid-cols-2 gap-6">
        <VariantCard label="user, plain">
          <Message message={userPlain} />
        </VariantCard>

        <VariantCard label="user, with attachments">
          <Message message={userWithAttachments} />
        </VariantCard>

        <VariantCard label="assistant, complete">
          <Message message={assistantComplete} />
        </VariantCard>

        <VariantCard label="assistant, streaming">
          <Message message={assistantStreaming} />
        </VariantCard>

        <VariantCard label="assistant, thinking (no content yet)">
          <Message message={assistantThinking} />
        </VariantCard>

        <VariantCard label="thought, streaming">
          <ThoughtMessage message={thoughtStreaming} />
        </VariantCard>

        <VariantCard label="thought, briefly (collapsed)">
          <ThoughtMessage message={thoughtBrief} />
        </VariantCard>

        <VariantCard label="thought, 2.3s (collapsed)">
          <ThoughtMessage message={thoughtLong} />
        </VariantCard>

        <VariantCard label="thought, expanded" className="@3xl:col-span-2">
          <ThoughtMessage message={thoughtLong} defaultOpen />
        </VariantCard>

        <VariantCard label="function-call, awaiting permission (single arg)">
          <FunctionCallMessage message={fcallPendingSingle} />
        </VariantCard>

        <VariantCard label="function-call, awaiting permission (multi arg)">
          <FunctionCallMessage message={fcallPendingMulti} />
        </VariantCard>

        <VariantCard label="function-call, running">
          <FunctionCallMessage message={fcallRunning} />
        </VariantCard>

        <VariantCard label="function-call, done (collapsed)">
          <FunctionCallMessage message={fcallDone} />
        </VariantCard>

        <VariantCard label="function-call, done · single-field (expanded)">
          <FunctionCallMessage message={fcallDone} defaultOpen />
        </VariantCard>

        <VariantCard label="function-call, done · multi-field (expanded)">
          <FunctionCallMessage message={fcallDoneMulti} defaultOpen />
        </VariantCard>

        <VariantCard label="function-call group, in-flight (3 functions, 2nd running)">
          <FunctionCallGroup messages={fcallGroupInFlight} />
        </VariantCard>

        <VariantCard label="function-call group, done (3 functions, collapsed)">
          <FunctionCallGroup messages={fcallGroupDone} />
        </VariantCard>

        <VariantCard label="function-call group, done (3 functions, expanded)" className="@3xl:col-span-2">
          <FunctionCallGroup messages={fcallGroupDone} defaultOpen />
        </VariantCard>
      </div>
    </Section>
  )
}
