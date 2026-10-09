import { SessionTreeDeletionError } from '../../src/lib/sessions/delete-tree'
import type { Conversation } from '../../src/types/chat'

const fixtureScenario = new URLSearchParams(location.search).get('scenario')
const a11yExample = ['a11y', 'longerror'].includes(fixtureScenario ?? '')
const title = `Selected chat — ${'a long descriptive title '.repeat(a11yExample ? 32 : 8)}`
const conversation: Conversation = {
  id: 'child2',
  title,
  parentId: 'parent',
  status: 'done',
  model: null,
  messages: [],
  hydrated: true,
  createdAt: 1,
  updatedAt: 1,
}
const staleParent = fixtureScenario === 'overlap-missing'
const parentConversation = {
  ...conversation,
  id: 'parent',
  parentId: undefined,
}
let calls = 0
export function useConversationsCtx() {
  return {
    conversations: staleParent
      ? [parentConversation]
      : [parentConversation, conversation],
    activeId: 'child2',
    active: conversation,
    watchConversation: () => () => {},
    createNew: () => {},
    select: () => {},
    rename: () => {},
    remove: async (
      _id: string,
      options?: {
        recover?: boolean
        reviewOperationId?: string
        force?: unknown
      },
    ) => {
      const query = new URLSearchParams(location.search)
      const scenario = query.get('scenario')
      // Frontend-only example data: no runtime calls or deletion evidence.
      const requestedCount = Number(query.get('blockers') ?? 15)
      const count =
        scenario === 'a11y'
          ? 25
          : [1, 24, 1000].includes(requestedCount)
            ? requestedCount
            : 15
      calls += 1
      if (calls === 1 && scenario === 'longerror')
        throw new Error(
          `${'Simulated failure detail requiring body scrolling. '.repeat(24)}End of simulated failure.`,
        )
      if (
        (calls === 1 && scenario === 'overlap') ||
        (staleParent && (calls === 1 || options?.reviewOperationId))
      )
        throw new SessionTreeDeletionError({
          operation_id: 'parent-op',
          attempt: 1,
          session_id: 'parent',
          status: 'failed',
          mode: 'normal',
          deleted_session_ids: [],
          remaining_session_ids: [],
          unconfirmed_session_ids: [],
          failure_code: 'overlapping_deletion',
          force_eligible: false,
          existing_deletion: { operation_id: 'child-op', session_id: 'child2' },
        })
      if (calls === 1 && scenario === 'preplan')
        throw new SessionTreeDeletionError({
          operation_id: 'op',
          attempt: 1,
          session_id: 'child2',
          status: 'failed',
          mode: 'normal',
          deleted_session_ids: [],
          remaining_session_ids: [],
          unconfirmed_session_ids: [],
          failure_code: 'failed',
          force_eligible: false,
          data_retained: false,
          blockers: [],
        })
      if (calls === 1 || (calls === 2 && scenario === 'overlap'))
        throw new SessionTreeDeletionError({
          operation_id: 'op',
          attempt: 1,
          session_id: 'child2',
          status: 'failed',
          mode: 'normal',
          data_retained: scenario === 'legacy' ? undefined : true,
          deleted_session_ids: [],
          remaining_session_ids:
            scenario === 'a11y'
              ? [
                  'child2',
                  ...Array.from(
                    { length: 24 },
                    (_, index) => `grandchild-${index + 1}`,
                  ),
                ]
              : ['child2', 'grandchild1'],
          unconfirmed_session_ids: scenario === 'legacy' ? undefined : [],
          force_eligible: true,
          failure_code: 'blocked',
          blockers: Array.from({ length: count }, (_, index) => ({
            kind:
              scenario === 'active'
                ? 'active_processing'
                : query.get('kinds') === 'mixed'
                  ? (
                      [
                        'active_processing',
                        'unconfirmed_cancellation',
                        'unknown_completion',
                      ] as const
                    )[index % 3]
                  : 'unknown_completion',
            session_id: `grandchild${query.has('blockers') ? Math.floor(index / 12) : 1}-${'s'.repeat(100)}`,
            function_id: 'browser::fetch',
            call_id: `call-${index}-${'c'.repeat(100)}`,
            started_at: 1,
          })),
        })
      if (calls === 2 && scenario === 'rejected')
        throw Object.assign(new Error('stale confirmation'), {
          code: 'harness/invalid_request',
        })
      if (calls === 2 && scenario === 'partial')
        throw new SessionTreeDeletionError({
          operation_id: 'op',
          attempt: 2,
          session_id: 'child2',
          status: 'failed',
          mode: 'force',
          data_retained: false,
          deleted_session_ids: ['grandchild1'],
          remaining_session_ids: ['child2'],
          unconfirmed_session_ids: ['child2'],
          force_eligible: false,
          failure_code: 'failed',
          blockers: [],
          error: 'Deletion dependency failed; completion is not confirmed.',
        })
      if (calls === 2 && scenario === 'network')
        throw new Error('Delete acknowledgement lost')
      await new Promise((resolve) => setTimeout(resolve, 250))
    },
    backend: {},
    modelOptions: [],
    catalogLoading: false,
    connectionState: 'connected',
    missingConversationIds: new Set<string>(),
  }
}
export function useConversationsCtxOptional() {
  return useConversationsCtx()
}
export async function getRemovalPreview(item?: Conversation) {
  return {
    id: item?.id ?? 'child2',
    title,
    parentId: 'parent',
    hasChildren: true,
    descendantCount: 1,
    hasRunningWork: a11yExample,
    empty: false,
  }
}
export function ConversationSidebar({
  onRemove,
}: {
  onRemove: (id: string) => void
}) {
  return (
    <button
      type="button"
      onClick={() =>
        onRemove(
          staleParent || fixtureScenario === 'overlap' ? 'parent' : 'child2',
        )
      }
    >
      Request removal
    </button>
  )
}
export function ChatView({ onBack }: { onBack?: () => void }) {
  return (
    <div>
      Isolated chat boundary
      {onBack ? (
        <button type="button" onClick={onBack}>
          Back to conversations
        </button>
      ) : null}
    </div>
  )
}
export function getIiiClient(): Promise<never> {
  return Promise.reject(new Error('Isolated network boundary'))
}
