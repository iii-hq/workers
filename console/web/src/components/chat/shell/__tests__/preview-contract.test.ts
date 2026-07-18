import { describe, expect, it } from 'vitest'
import type { FunctionCallMessage } from '@/types/chat'
import { ShellToolView } from '../index'

function pendingCall(functionId: string, input: unknown): FunctionCallMessage {
  return {
    id: 'm1',
    role: 'function-call',
    functionId,
    input,
    pendingApproval: true,
    createdAt: 0,
  }
}

/* FunctionCallCard falls back to its raw request pane only when
   tryRenderPreview returns null. An element that renders nothing would
   leave a held call with no visible arguments at all (MOT-4101), so the
   parse decision must happen here, not inside the preview component. */
describe('ShellToolView.tryRenderPreview null contract', () => {
  it('returns an element only when the request parses', () => {
    expect(
      ShellToolView.tryRenderPreview(
        pendingCall('shell::exec', { command: 'ls -la' }),
      ),
    ).not.toBeNull()
    expect(
      ShellToolView.tryRenderPreview(
        pendingCall('shell::kill', { job_id: 'job-1' }),
      ),
    ).not.toBeNull()
  })

  it('returns null for empty or unparseable requests', () => {
    expect(
      ShellToolView.tryRenderPreview(pendingCall('shell::exec', {})),
    ).toBeNull()
    // A raw string (unrecovered stringified payload) must not parse.
    expect(
      ShellToolView.tryRenderPreview(pendingCall('shell::exec', 'ls -la')),
    ).toBeNull()
    // argv-as-command is rejected by the schema (server rejects it too).
    expect(
      ShellToolView.tryRenderPreview(
        pendingCall('shell::exec', { command: ['ls', '-la'] }),
      ),
    ).toBeNull()
    expect(
      ShellToolView.tryRenderPreview(pendingCall('shell::exec_bg', {})),
    ).toBeNull()
    expect(
      ShellToolView.tryRenderPreview(pendingCall('shell::kill', {})),
    ).toBeNull()
  })
})
