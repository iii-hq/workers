/**
 * Demo-local copy of the chat empty state's hero. The product's `EmptyState`
 * keeps its own words; the landing demo wants this greeting, rendered inside
 * the transcript scroller (via `MessageList`'s `header` slot) so the replay
 * scrolls it away instead of unmounting it.
 */

import { Prompt } from '@/components/ui/Prompt'

export function DemoEmptyState() {
  return (
    <div className="flex max-w-[520px] flex-col gap-6">
      <div className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
        <Prompt symbol="$">new session</Prompt>
      </div>
      <h1 className="font-mono text-[28px] font-medium tracking-[-0.01em] text-ink lowercase">
        welcome to iii! 👋
      </h1>
      <div className="flex flex-col gap-2">
        <p className="font-mono text-[14px] leading-[1.7] text-ink-faint lowercase">
          You're in a 1:1 reproduction of our agent-oriented workspace where
          you can leverage the power of a full-featured agentic system:
        </p>
        <ul className="font-mono text-[13px] leading-[1.7] text-ink-faint flex flex-col gap-1">
          <li>· trigger functions via the function registry</li>
          <li>· spawn sub-agents for parallel/async work</li>
          <li>· register triggers for loop and graph control</li>
          <li>
            · query state, publish to queues, and do everything production
            systems do
          </li>
        </ul>
      </div>
    </div>
  )
}
