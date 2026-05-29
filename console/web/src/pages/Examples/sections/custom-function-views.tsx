import { FunctionCallMessage } from '@/components/chat/FunctionCallMessage'
import type { FunctionCallMessage as FCallType } from '@/types/chat'
import { Section, VariantCard } from '../Section'
import { directoryFixtures } from './directory-fixtures'
import { engineFixtures } from './engine-fixtures'
import { webFixtures } from './web-fixtures'
import { workerFixtures } from './worker-fixtures'

/**
 * Namespace-specific function-call renderers, one card per fixture. Mirrors
 * the sandbox block in `message-variants` but groups the newer families
 * (directory / engine / web / worker) under their own headings so the spec
 * sheet stays scannable as more views land. Every card is `defaultOpen` so
 * the custom terminal/preview pane renders, not just the collapsed header.
 */
const FAMILIES: { family: string; fixtures: readonly FCallType[] }[] = [
  { family: 'directory', fixtures: directoryFixtures },
  { family: 'engine', fixtures: engineFixtures },
  { family: 'web', fixtures: webFixtures },
  { family: 'worker', fixtures: workerFixtures },
]

export function CustomFunctionViewsSection() {
  return (
    <Section
      id="custom-function-views"
      eyebrow="function views"
      title="custom function views"
      description="namespace-specific renderers for agent function calls. fixtures only — no live state. unmatched ids fall back to the default request/response json panes."
    >
      <div className="flex flex-col gap-10">
        {FAMILIES.map(({ family, fixtures }) => (
          <div key={family} className="flex flex-col gap-4">
            <h3 className="font-mono text-[11px] uppercase tracking-[0.12em] text-ink-faint border-b border-rule-2 pb-2">
              {family} · {fixtures.length} view
              {fixtures.length === 1 ? '' : 's'}
            </h3>
            <div className="grid grid-cols-1 @3xl:grid-cols-2 gap-6">
              {fixtures.map((fixture) => (
                <VariantCard
                  key={fixture.id}
                  label={`${family} · ${fixture.functionId}`}
                >
                  <FunctionCallMessage message={fixture} defaultOpen />
                </VariantCard>
              ))}
            </div>
          </div>
        ))}
      </div>
    </Section>
  )
}
