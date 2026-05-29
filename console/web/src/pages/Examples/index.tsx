import { Prompt } from '@/components/ui/Prompt'
import { ColorTokensSection } from './sections/color-tokens'
import { ComposerVariantsSection } from './sections/composer-variants'
import { CustomFunctionViewsSection } from './sections/custom-function-views'
import { LoadingStatesSection } from './sections/loading-states'
import { MessageVariantsSection } from './sections/message-variants'
import { PrimitivesSection } from './sections/primitives'
import { TypographySection } from './sections/typography'

const TOC: { id: string; label: string }[] = [
  { id: 'composer-variants', label: 'composer' },
  { id: 'message-variants', label: 'messages' },
  { id: 'custom-function-views', label: 'function views' },
  { id: 'loading-states', label: 'loading' },
  { id: 'primitives', label: 'primitives' },
  { id: 'typography', label: 'typography' },
  { id: 'color-tokens', label: 'color' },
]

export function Examples() {
  return (
    <section
      className="flex-1 overflow-y-auto @container"
      style={{ containerType: 'inline-size' }}
    >
      <div className="px-9 py-10 max-w-[1200px] mx-auto flex flex-col gap-12">
        <header className="flex flex-col gap-3">
          <div className="font-mono text-[11px] uppercase tracking-[0.18em] text-ink-faint">
            <Prompt symbol="$">examples</Prompt>
          </div>
          <h1 className="font-mono text-[28px] font-medium tracking-[-0.01em] text-ink lowercase">
            spec sheet.
          </h1>
          <p className="font-mono text-[14px] leading-[1.7] text-ink-faint max-w-[60ch] lowercase">
            every composer, message kind, loading state, primitive, type token,
            and color in one place. use it to sanity-check visual changes before
            shipping them.
          </p>
          <nav className="flex flex-wrap gap-x-4 gap-y-2 pt-3 border-t border-rule-2">
            {TOC.map((t) => (
              <a
                key={t.id}
                href={`#${t.id}`}
                className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint hover:text-ink transition-colors"
              >
                {t.label}
              </a>
            ))}
          </nav>
        </header>

        <ComposerVariantsSection />
        <MessageVariantsSection />
        <CustomFunctionViewsSection />
        <LoadingStatesSection />
        <PrimitivesSection />
        <TypographySection />
        <ColorTokensSection />

        <footer className="border-t border-rule-2 pt-6 pb-12">
          <div className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-ghost">
            <Prompt symbol=">">end of sheet.</Prompt>
          </div>
        </footer>
      </div>
    </section>
  )
}
