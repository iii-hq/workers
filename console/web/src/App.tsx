import { lazy, Suspense } from 'react'
import { ModeToggle } from '@/components/ui/ModeToggle'
import { Sheet } from '@/components/ui/Sheet'
import { Wordmark } from '@/components/ui/Wordmark'
import { useHashRoute, type View } from '@/hooks/use-hash-route'
import { type Theme, useTheme } from '@/hooks/use-theme'
import { Chat } from '@/pages/Chat'
import { Traces } from '@/pages/Traces'

const PLAYGROUND_ENABLED = !!import.meta.env.VITE_PLAYGROUND

/* Lazy + flag-guarded: when VITE_PLAYGROUND is empty at build time, both
   constants fold to null and Rolldown drops the dynamic-import targets along
   with every transitive module (mock backend, scenarios, examples sections). */
const Examples = PLAYGROUND_ENABLED
  ? lazy(() =>
      import('@/pages/Examples').then((m) => ({ default: m.Examples })),
    )
  : null

const Playground = PLAYGROUND_ENABLED
  ? lazy(() =>
      import('@/pages/Playground').then((m) => ({ default: m.Playground })),
    )
  : null

const VIEW_OPTIONS: { value: View; label: string }[] = PLAYGROUND_ENABLED
  ? [
      { value: 'chat', label: 'chat' },
      { value: 'traces', label: 'traces' },
      { value: 'playground', label: 'playground' },
      { value: 'examples', label: 'examples' },
    ]
  : [
      { value: 'chat', label: 'chat' },
      { value: 'traces', label: 'traces' },
    ]

export function App() {
  const [theme, setTheme] = useTheme()
  const [view, setView] = useHashRoute()

  return (
    <Sheet>
      <Header
        view={view}
        onViewChange={setView}
        theme={theme}
        onThemeChange={setTheme}
      />
      <Suspense fallback={<RouteFallback />}>
        {view === 'traces' ? (
          <Traces />
        ) : view === 'examples' && Examples ? (
          <Examples />
        ) : view === 'playground' && Playground ? (
          <Playground />
        ) : (
          <Chat />
        )}
      </Suspense>
    </Sheet>
  )
}

interface HeaderProps {
  view: View
  onViewChange: (next: View) => void
  theme: Theme
  onThemeChange: (next: Theme) => void
}

function Header({ view, onViewChange, theme, onThemeChange }: HeaderProps) {
  return (
    <header className="flex items-center justify-between px-6 h-12 border-b border-rule shrink-0">
      <div className="flex items-center gap-3">
        <Wordmark />
        <span className="font-mono text-[11px] uppercase tracking-[0.18em] text-ink-faint">
          {view}
        </span>
      </div>
      <div className="flex items-center gap-3">
        <ModeToggle<View>
          value={view}
          onChange={onViewChange}
          options={VIEW_OPTIONS}
        />
        <ModeToggle<Theme>
          value={theme}
          onChange={onThemeChange}
          options={[
            { value: 'light', label: 'light' },
            { value: 'dark', label: 'dark' },
          ]}
        />
      </div>
    </header>
  )
}

function RouteFallback() {
  return (
    <section className="flex-1 flex items-center justify-center">
      <div className="font-mono text-[12px] uppercase tracking-[0.18em] text-ink-ghost">
        loading…
      </div>
    </section>
  )
}
