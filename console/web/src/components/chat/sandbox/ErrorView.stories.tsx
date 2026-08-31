import type { Meta, StoryObj } from '@storybook/react-vite'
import { FunctionTriggerCard } from '@/components/function-trigger/FunctionTriggerCard'
import type { FunctionTriggerMessage } from '@/types/chat'
import { SandboxErrorView } from './ErrorView'

const genericFunctionError: FunctionTriggerMessage = {
  id: 'error-view-story-generic',
  role: 'function-trigger',
  functionId: 'sandbox::exec',
  input: { command: 'run the audit' },
  output: {
    error: {
      kind: 'function_error',
      message: 'sandbox/depth_exceeded: nesting depth 3 exceeds max_depth 2',
    },
  },
  createdAt: Date.now(),
}

const meta = {
  title: 'Chat/Sandbox/ErrorView',
  component: SandboxErrorView,
  parameters: { layout: 'padded' },
  args: {
    display: {
      variant: 'wire',
      error: {
        type: 'sandbox_error',
        code: 'S200',
        message: 'Command timed out after 100ms.',
        docs_url: 'https://docs.example/s200',
        retryable: true,
        fix_note: 'Increase timeout_ms or simplify the command.',
        fix: {
          stdout: 'Downloaded 38 of 120 records before the timeout.\n',
          stderr: '',
          exit_code: null,
          timed_out: true,
          duration_ms: 100,
          success: false,
        },
      },
    },
  },
} satisfies Meta<typeof SandboxErrorView>

export default meta
type Story = StoryObj<typeof meta>

export const RetryableTimeout: Story = {
  name: 'retryable timeout',
}

export const InvocationDenied: Story = {
  name: 'invocation denied',
  args: {
    display: {
      variant: 'invocation',
      error: {
        title: 'Gate unavailable',
        message:
          'The request could not continue because the approval gate is unavailable.',
        functionId: 'sandbox::fs::write',
        deniedBy: 'gate_unavailable',
        detailText: 'trigger_failed: approval gate unreachable',
      },
    },
  },
}

export const DispatchPolicy: Story = {
  name: 'dispatch policy denied',
  args: {
    display: {
      variant: 'dispatch-denied',
      error: {
        functionId: 'web::fetch',
        namespace: 'web',
        message:
          'function web::fetch is not permitted by this agent’s dispatch policy',
      },
    },
  },
}

export const InFunctionCard: Story = {
  name: 'inside function card',
  render: () => (
    <FunctionTriggerCard message={genericFunctionError} defaultOpen />
  ),
}
