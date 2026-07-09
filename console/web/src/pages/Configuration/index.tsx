import type { Theme } from '@/hooks/use-theme'
import { ConsoleSettingsTab } from './tabs/ConsoleSettingsTab'

interface ConfigurationProps {
  theme: Theme
  onThemeChange: (next: Theme) => void
}

/**
 * Console-level settings shell. Worker configuration now lives on the Workers
 * page as row actions, with old worker-configuration hashes routed there for
 * compatibility.
 */
export function Configuration({ theme, onThemeChange }: ConfigurationProps) {
  return (
    <main
      className="configuration-surface font-sans flex-1 overflow-hidden flex flex-col min-h-0"
      aria-label="console settings"
    >
      <div className="px-6 pt-8 pb-4 shrink-0">
        <header className="pb-4">
          <h1 className="font-sans text-[20px] text-ink lowercase tracking-[-0.01em]">
            console settings
          </h1>
        </header>
      </div>
      <div className="flex-1 min-h-0 flex flex-col">
        <ConsoleSettingsTab theme={theme} onThemeChange={onThemeChange} />
      </div>
    </main>
  )
}
