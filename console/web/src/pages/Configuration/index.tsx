import { ModeToggle } from '@/components/ui/ModeToggle'
import {
  type ConfigurationTab,
  useConfigurationRoute,
} from '@/hooks/use-hash-route'
import type { Theme } from '@/hooks/use-theme'
import { ConsoleSettingsTab } from './tabs/ConsoleSettingsTab'
import { WorkersTab } from './tabs/WorkersTab'
import { useUnsavedGuard } from './tabs/WorkersTab/useUnsavedGuard'

interface ConfigurationProps {
  theme: Theme
  onThemeChange: (next: Theme) => void
}

const TAB_OPTIONS: { value: ConfigurationTab; label: string }[] = [
  { value: 'console', label: 'console settings' },
  { value: 'workers', label: 'workers' },
]

/**
 * Configuration page shell. Owns the tab strip and dispatches the active
 * tab to its matching surface. Each tab is a self-contained component so
 * we never share state across tabs without going through the URL.
 *
 * The tab selection lives in the hash (`#/configuration/<tab>`), not in
 * local state, so deep links and the browser back button work cleanly.
 *
 * The shell also hosts the unsaved-changes guard for the workers tab so
 * a single dirty bit can intercept tab switches (handled here), in-tab
 * selection changes (handled by `WorkersTab`), and browser close
 * (handled inside `useUnsavedGuard`). Console settings save inline per
 * field and don't need the guard.
 */
export function Configuration({ theme, onThemeChange }: ConfigurationProps) {
  const [route, navigate] = useConfigurationRoute()
  const guard = useUnsavedGuard()

  function handleTabChange(tab: ConfigurationTab) {
    guard.tryNavigate(() => navigate({ tab }))
  }

  function handleWorkerSelect(workerId: string | null) {
    guard.tryNavigate(() => navigate({ tab: 'workers', workerId }))
  }

  return (
    <main
      className="configuration-surface font-sans flex-1 overflow-hidden flex flex-col min-h-0"
      aria-label="configuration"
    >
      <div className="px-6 pt-8 pb-4 shrink-0">
        <header className="pb-4">
          <h1 className="font-sans text-[20px] text-ink lowercase tracking-[-0.01em]">
            configuration
          </h1>
        </header>
        <ModeToggle<ConfigurationTab>
          value={route.tab}
          onChange={handleTabChange}
          options={TAB_OPTIONS}
          aria-label="configuration tabs"
        />
      </div>
      <div className="flex-1 min-h-0 flex flex-col">
        {route.tab === 'workers' ? (
          <WorkersTab
            selectedId={route.workerId}
            onSelect={handleWorkerSelect}
            onDirtyChange={guard.setDirty}
          />
        ) : (
          <ConsoleSettingsTab theme={theme} onThemeChange={onThemeChange} />
        )}
      </div>
    </main>
  )
}
