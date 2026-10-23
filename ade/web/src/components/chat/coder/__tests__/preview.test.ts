import { describe, expect, it } from 'vitest'
import type { FunctionTriggerMessage } from '@/types/chat'
import { CoderToolView } from '../index'

function pending(input: unknown): FunctionTriggerMessage {
  return {
    functionId: 'coder::update-file',
    input,
    pendingApproval: true,
  } as unknown as FunctionTriggerMessage
}

describe('CoderToolView.tryRenderPreview', () => {
  it('returns null for a request the preview cannot parse, so the raw request shows', () => {
    expect(
      CoderToolView.tryRenderPreview(
        pending({
          files: [
            {
              path: 'a',
              ops: [{ from_line: 1, to_line: 2, new_content: 'x' }],
            },
          ],
        }),
      ),
    ).toBeNull()
    expect(
      CoderToolView.tryRenderPreview(pending({ path: 'a', ops: [] })),
    ).toBeNull()
  })

  it('renders the preview for an op whose missing `op` the worker infers', () => {
    expect(
      CoderToolView.tryRenderPreview(
        pending({
          files: [{ path: 'a', ops: [{ from_line: 1, to_line: 40 }] }],
        }),
      ),
    ).not.toBeNull()
  })
})
