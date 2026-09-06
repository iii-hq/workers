import type { Meta, StoryObj } from '@storybook/react-vite'
import { registrationFromCall } from '@/components/trigger-activity/model'
import type {
  AssistantMessage,
  Attachment,
  SystemMessage,
  UserMessage,
} from '@/types/chat'
import { Message } from './Message'

const sampleAttachments: Attachment[] = [
  { id: 'a1', name: 'spec.md', size: 4_312, type: 'text/markdown' },
  { id: 'a2', name: 'screenshot.png', size: 281_402, type: 'image/png' },
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

const spawnTask: UserMessage = {
  id: 'spawn-1',
  role: 'user',
  content:
    'Research the best approach for rendering Mermaid diagrams entirely inside one self-contained HTML file. Compare CDN integration with vendoring and recommend a practical implementation with caveats.',
  spawn: true,
  createdAt: Date.now(),
}

const assistantComplete: AssistantMessage = {
  id: 'a1',
  role: 'assistant',
  model: 'anthropic::claude-opus-4-7',
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
  content:
    'sure — a btree-backed index gives you both `O(log n)` lookups and cheap range',
  streaming: true,
  createdAt: Date.now(),
}

const assistantThinking: AssistantMessage = {
  id: 'a3',
  role: 'assistant',
  model: 'anthropic::claude-opus-4-7',
  content: '',
  streaming: true,
  createdAt: Date.now(),
}

const triggerFiredCall: SystemMessage = {
  id: 's_trig1',
  role: 'system',
  kind: 'trigger-fired',
  tone: 'info',
  content: 'orders watch · called example::check_completion',
  trigger: {
    subscription_id: 'sub_00000000000000000000000000000001',
    trigger_id: '00000000-0000-4000-8000-000000000001',
    target: 'example::check_completion',
    label: 'orders watch',
    action: 'order record changed',
    once: false,
    retired: false,
    fired_at: 1785948999879,
    payload: {
      event: {
        db: 'primary',
        table: 'orders',
        op: 'update',
        affected_rows: 1,
        at: 1785948999879,
      },
    },
  },
  createdAt: 1785948999879,
}

const triggerFiredOnce: SystemMessage = {
  id: 'e_trigfired_sub_story_1',
  role: 'system',
  kind: 'trigger-fired',
  tone: 'info',
  content: 'daily report · notified this chat · once consumed',
  trigger: {
    subscription_id: 'sub_story',
    trigger_id: 'trigger-story',
    trigger_type: 'cron',
    config: { expression: '0 30 9 * * *' },
    target: 'harness::send',
    label: 'daily report',
    action: 'daily report became ready',
    once: true,
    retired: true,
    fires: 1,
    fired_at: 1_785_948_999_879,
    outcome: 'delivered',
    retirement_reason: 'once_consumed',
  },
  createdAt: 1_785_948_999_879,
}

const triggerNotification: UserMessage = {
  id: 'e_fire_sub_story_1',
  role: 'user',
  content: '[notification] daily report: {"report_id":"rpt-42","rows":128}',
  notification: true,
  triggerBindingId: 'sub_story',
  createdAt: 1_785_948_999_879,
}

const triggerManuallyRemoved: SystemMessage = {
  id: 'e_trigexpired_sub_manual',
  role: 'system',
  kind: 'trigger-fired',
  tone: 'info',
  content: 'orders watch · binding manually removed',
  trigger: {
    subscription_id: 'sub_manual',
    trigger_type: 'database::row-changed',
    config: { database: 'primary', table: 'orders' },
    target: 'orders::reindex',
    label: 'orders watch',
    once: false,
    retired: true,
    fires: 4,
    fired_at: 1_785_949_099_879,
    outcome: 'unregistered',
    retirement_reason: 'unregistered',
  },
  createdAt: 1_785_949_099_879,
}

const HARNESS_DIR = '/Users/sergio/Documents/workspaces/iii/workers/harness'
const WORKERS_DIR = '/Users/sergio/Documents/workspaces/iii/workers'

const workingDirChanged: SystemMessage = {
  id: 's_wd_changed',
  role: 'system',
  kind: 'working-dir',
  tone: 'info',
  content: `working directory changed to ${HARNESS_DIR} — applies to the messages that follow`,
  scope: { path: HARNESS_DIR, previousPath: WORKERS_DIR, cause: 'selected' },
  createdAt: 1_785_949_000_000,
}

const workingDirRecovered: SystemMessage = {
  id: 's_wd_recovered',
  role: 'system',
  kind: 'working-dir',
  tone: 'info',
  content: `working directory changed to ${WORKERS_DIR} because /private/tmp/iii-harness-9f3c is no longer available — applies to the messages that follow`,
  scope: {
    path: WORKERS_DIR,
    previousPath: '/private/tmp/iii-harness-9f3c',
    cause: 'recovered',
  },
  createdAt: 1_785_949_000_000,
}

const workingDirUnavailable: SystemMessage = {
  id: 's_wd_unavailable',
  role: 'system',
  kind: 'working-dir',
  tone: 'info',
  content:
    'working directory /private/tmp/iii-harness-9f3c is no longer available; this session is now unscoped — applies to the messages that follow',
  scope: {
    path: null,
    previousPath: '/private/tmp/iii-harness-9f3c',
    cause: 'unavailable',
  },
  createdAt: 1_785_949_000_000,
}

const failureCredentials: SystemMessage = {
  id: 'e_t1_error',
  role: 'system',
  kind: 'turn-failure',
  tone: 'error',
  content: 'The provider authentication needs attention.',
  failure: {
    summary: 'The provider authentication needs attention.',
    retryable: false,
    phase: 'execution',
  },
  nextActions: [
    'Update the provider credentials in LLM Router settings.',
    'Retry the turn after the credentials are updated.',
  ],
  technicalDetails: {
    code: 'router/provider_auth_expired',
    class: 'llm.auth_expired',
    detail:
      'remote error (router/provider_auth_expired): 401 invalid_api_key — Incorrect API key provided: sk-proj-****Xk2. You can find your API key at https://platform.openai.com/account/api-keys.',
    provider: 'openai',
    model: 'gpt-5.4',
  },
  createdAt: 1_785_949_000_000,
}

const failureBilling: SystemMessage = {
  id: 'e_t2_error',
  role: 'system',
  kind: 'turn-failure',
  tone: 'error',
  content: 'The provider rejected this request.',
  failure: {
    summary: 'The provider rejected this request.',
    retryable: false,
    phase: 'execution',
  },
  nextActions: [
    'Review the selected model and provider settings, then try again.',
  ],
  technicalDetails: {
    code: 'router/provider_rejected',
    class: 'llm.permanent',
    detail:
      'anthropic messages: 400 invalid_request_error — Your credit balance is too low to access the Anthropic API. Please go to Plans & Billing to upgrade or purchase credits.',
    provider: 'anthropic',
    model: 'claude-opus-4-7',
  },
  createdAt: 1_785_949_000_000,
}

const failureConnection: SystemMessage = {
  id: 'e_t3_error',
  role: 'system',
  kind: 'turn-failure',
  tone: 'error',
  content:
    'The provider disconnected before completing the response. A partial response was preserved in this conversation and may be incomplete. Automatic recovery stopped after 1 of 1 attempts.',
  failure: {
    summary: 'The provider disconnected before completing the response.',
    retryable: true,
    partialResultAvailable: true,
    recoveryAttempted: 1,
    recoveryMaxAttempts: 1,
    phase: 'execution',
  },
  nextActions: ['Retry the turn to continue.'],
  technicalDetails: {
    code: 'router/stream_incomplete',
    class: 'llm.transient',
    detail: 'stream ended without a terminal frame',
    provider: 'zai',
    model: 'glm-5',
  },
  createdAt: 1_785_949_000_000,
}

const failureInternal: SystemMessage = {
  id: 'e_t4_error',
  role: 'system',
  kind: 'turn-failure',
  tone: 'error',
  content: 'The turn could not be completed.',
  failure: { summary: 'The turn could not be completed.', retryable: false },
  nextActions: [
    'Inspect the failure details.',
    'Retry only after correcting the dependency or request.',
  ],
  technicalDetails: {
    code: 'harness.turn_internal',
    class: 'llm.permanent',
    detail: 'state::put_turn failed: store unavailable (S503)',
    provider: 'openai',
    model: 'gpt-5.4',
  },
  createdAt: 1_785_949_000_000,
}

const noticeInfo: SystemMessage = {
  id: 's_notice_info',
  role: 'system',
  kind: 'notice',
  tone: 'info',
  content: 'worktree feature/notices landed onto main (9f3c2a1b)',
  createdAt: 1_785_949_000_000,
}

const noticeWarn: SystemMessage = {
  id: 's_notice_warn',
  role: 'system',
  kind: 'notice',
  tone: 'warn',
  content: 'could not attach spec.pdf — file exceeds the 20 MB limit',
  createdAt: 1_785_949_000_000,
}

const noticeError: SystemMessage = {
  id: 's_notice_error',
  role: 'system',
  kind: 'notice',
  tone: 'error',
  content:
    'could not unregister the trigger — subscription sub_00000000000000000000000000000001 not found',
  createdAt: 1_785_949_000_000,
}

const noticeCompacting: SystemMessage = {
  id: 's_notice_compacting',
  role: 'system',
  tone: 'info',
  content: 'compacting session…',
  createdAt: 1_785_949_000_000,
}

const meta = {
  title: 'Chat/Message',
  component: Message,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof Message>

export default meta
type Story = StoryObj<typeof meta>

export const UserPlain: Story = {
  name: 'user, plain',
  args: { message: userPlain },
}

export const UserWithAttachments: Story = {
  name: 'user, with attachments',
  args: { message: userWithAttachments },
}

export const SpawnTask: Story = {
  name: 'spawn, sub-agent task',
  args: {
    message: spawnTask,
    spawnContext: {
      title: 'Researcher',
      model: 'codex/gpt-5.6-luna',
      appearance: { name: 'Researcher', icon: 'search', color: 'purple' },
    },
  },
}

export const AssistantComplete: Story = {
  name: 'assistant, complete',
  args: { message: assistantComplete },
}

export const AssistantStreaming: Story = {
  name: 'assistant, streaming',
  args: { message: assistantStreaming },
}

export const AssistantThinking: Story = {
  name: 'assistant, thinking (no content yet)',
  args: { message: assistantThinking },
}

export const TriggerFiredWithPayload: Story = {
  name: 'system, trigger fired · ƒ-call with payload',
  args: { message: triggerFiredCall },
}

export const TriggerFiredOnceConsumed: Story = {
  name: 'system, trigger fired · once consumed',
  args: {
    message: triggerFiredOnce,
    triggerNotification,
    registration: registrationFromCall({
      id: 'register-story',
      subscriptionId: 'sub_story',
      effectiveOnce: true,
      input: {
        trigger_type: 'cron',
        config: { expression: '0 30 9 * * *' },
        label: 'daily report',
        metadata: { action: 'daily report became ready' },
      },
    }),
  },
}

export const TriggerBindingManuallyRemoved: Story = {
  name: 'system, trigger binding · manually removed',
  args: { message: triggerManuallyRemoved },
}

export const WorkingDirChanged: Story = {
  name: 'system, working directory · changed',
  args: { message: workingDirChanged },
}

export const WorkingDirRecovered: Story = {
  name: 'system, working directory · recovered to default',
  args: { message: workingDirRecovered },
}

export const WorkingDirUnavailable: Story = {
  name: 'system, working directory · unavailable',
  args: { message: workingDirUnavailable },
}

export const TurnFailureCredentials: Story = {
  name: 'system, turn failure · credentials rejected (user)',
  args: { message: failureCredentials },
}

export const TurnFailureBilling: Story = {
  name: 'system, turn failure · credit exhausted (user)',
  args: { message: failureBilling },
}

export const TurnFailureConnection: Story = {
  name: 'system, turn failure · connection dropped (transient)',
  args: { message: failureConnection },
}

export const TurnFailureInternal: Story = {
  name: 'system, turn failure · iii internal',
  args: { message: failureInternal },
}

export const NoticeInfo: Story = {
  name: 'system, notice · info',
  args: { message: noticeInfo },
}

export const NoticeWarn: Story = {
  name: 'system, notice · warn',
  args: { message: noticeWarn },
}

export const NoticeError: Story = {
  name: 'system, notice · error',
  args: { message: noticeError },
}

/** Every system presentation in one transcript-shaped stack. */
export const SystemNoticesStack: Story = {
  name: 'system, all presentations · stack',
  args: { message: noticeCompacting },
  render: () => (
    <div className="flex flex-col gap-y-8">
      <Message message={userPlain} />
      <Message message={workingDirChanged} />
      <Message message={noticeCompacting} />
      <Message message={noticeInfo} />
      <Message message={noticeWarn} />
      <Message message={noticeError} />
      <Message message={failureConnection} />
      <Message message={failureCredentials} />
      <Message message={failureBilling} />
      <Message message={failureInternal} />
      <Message message={workingDirRecovered} />
      <Message message={workingDirUnavailable} />
    </div>
  ),
}
