import { Prompt } from '@/components/ui/Prompt'
import { cn } from '@/lib/utils'
import {
  type PlaygroundScenario,
  SCENARIO_GROUPS,
  SCENARIOS,
} from './scenarios'

interface ScenarioPickerProps {
  selectedId: string
  onSelect: (id: string) => void
}

export function ScenarioPicker({ selectedId, onSelect }: ScenarioPickerProps) {
  return (
    <aside className="w-[220px] shrink-0 border-r border-rule flex flex-col bg-bg min-h-0">
      <div className="px-3 py-3 border-b border-rule">
        <div className="font-mono text-[11px] uppercase tracking-[0.18em] text-ink-faint">
          <Prompt symbol="$">scenarios</Prompt>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto py-1">
        {SCENARIO_GROUPS.map((group) => {
          const items = SCENARIOS.filter((s) => s.group === group)
          if (items.length === 0) return null
          return (
            <div key={group} className="py-1">
              <div className="px-3 py-1.5 font-mono text-[10px] uppercase tracking-[0.18em] text-ink-ghost">
                {group}
              </div>
              <ul className="flex flex-col">
                {items.map((s) => (
                  <ScenarioRow
                    key={s.id}
                    scenario={s}
                    active={s.id === selectedId}
                    onSelect={() => onSelect(s.id)}
                  />
                ))}
              </ul>
            </div>
          )
        })}
      </div>
    </aside>
  )
}

interface ScenarioRowProps {
  scenario: PlaygroundScenario
  active: boolean
  onSelect: () => void
}

function ScenarioRow({ scenario, active, onSelect }: ScenarioRowProps) {
  return (
    <li>
      <button
        type="button"
        onClick={onSelect}
        aria-pressed={active}
        className={cn(
          'w-full text-left px-3 py-1.5 transition-colors group',
          'hover:bg-paper-2',
          active && 'bg-paper-2',
        )}
      >
        <div
          className={cn(
            'font-mono text-[12.5px] lowercase truncate',
            active ? 'text-ink' : 'text-ink-faint group-hover:text-ink',
          )}
        >
          <span
            aria-hidden
            className={cn(
              'mr-1.5 inline-block w-2',
              active ? 'text-accent' : 'text-transparent',
            )}
          >
            ▸
          </span>
          {scenario.label}
        </div>
        <div className="mt-0.5 ml-[18px] font-mono text-[11px] leading-[1.45] text-ink-ghost lowercase line-clamp-3">
          {scenario.description}
        </div>
      </button>
    </li>
  )
}
