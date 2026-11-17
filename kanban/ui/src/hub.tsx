import {
  ShadcnButton,
  ShadcnDropdownMenu,
  ShadcnDropdownMenuContent,
  ShadcnDropdownMenuGroup,
  ShadcnDropdownMenuItem,
  ShadcnDropdownMenuLabel,
  ShadcnDropdownMenuRadioGroup,
  ShadcnDropdownMenuRadioItem,
  ShadcnDropdownMenuSeparator,
  ShadcnDropdownMenuTrigger,
  ShadcnTabs,
  ShadcnTabsList,
  ShadcnTabsTrigger,
} from '@iii-dev/console-ui'
import { ChevronsUpDown, Columns3, LayoutGrid, Milestone, Network, Plus, SquareKanban } from 'lucide-react'
import { useState } from 'react'
import { FolderGlyph } from './plan/parts'
import type { ProjectView } from './shared'

/**
 * The Kanban page is one place for a project: pick the project in the header (or every project), then look at it
 * three ways — its Overview (progress, releases, details), its Board, or its Tree of tickets and agents. The
 * choice is kept per pane, and switching views keeps the project, the filters and the scroll of each view.
 */
export type HubView = 'overview' | 'board' | 'tree'

export const HUB_VIEWS: readonly HubView[] = ['overview', 'board', 'tree']

export const coerceView = (value: unknown, fallback: HubView): HubView =>
  HUB_VIEWS.includes(value as HubView) ? (value as HubView) : fallback

/** "Every project" in the switcher's radio group (a project id can never be empty). */
const ALL = ''

/** The header's title: the project the page is on, and the menu that changes it. */
export function ProjectSwitcher({
  projects,
  value,
  totals,
  onChange,
  onNewProject,
  onOpenReleases,
}: {
  projects: readonly ProjectView[]
  /** The project on screen; null = every project. */
  value: ProjectView | null
  /** Tickets on the whole board, for the "All projects" row. */
  totals: number
  onChange: (id: string | null) => void
  onNewProject: () => void
  /** Absent when the console has no panes to open the Releases page in. */
  onOpenReleases?: () => void
}) {
  // Controlled: a radio row keeps a Base UI menu open, and picking a project should close it.
  const [open, setOpen] = useState(false)
  const active = projects.filter((project) => project.status === 'active')
  const archived = projects.filter((project) => project.status === 'archived')
  const row = (project: ProjectView) => (
    <ShadcnDropdownMenuRadioItem className="kb-menu__row" key={project.id} value={project.id}>
      <FolderGlyph color={project.color} />
      <span className="kb-menu__label">{project.name}</span>
      <span className="kb-menu__hint">
        {project.done}/{project.total}
      </span>
    </ShadcnDropdownMenuRadioItem>
  )
  return (
    <ShadcnDropdownMenu onOpenChange={setOpen} open={open}>
      <ShadcnDropdownMenuTrigger
        render={
          <ShadcnButton
            aria-label={`Project: ${value?.name ?? 'All projects'}. Change project`}
            className="kb-hub-switcher"
            size="sm"
            type="button"
            variant="ghost"
          >
            {value ? <FolderGlyph color={value.color} /> : <SquareKanban size={16} />}
            <span className="kb-hub-switcher__name">{value?.name ?? 'All projects'}</span>
            <ChevronsUpDown className="kb-hub-switcher__chevron" size={16} />
          </ShadcnButton>
        }
      />
      <ShadcnDropdownMenuContent align="start" className="kb-menu kb-hub-switcher__menu">
        <ShadcnDropdownMenuRadioGroup
          onValueChange={(next: string) => {
            setOpen(false)
            onChange(next === ALL ? null : next)
          }}
          value={value?.id ?? ALL}
        >
          <ShadcnDropdownMenuGroup>
            <ShadcnDropdownMenuRadioItem className="kb-menu__row" value={ALL}>
              <SquareKanban size={16} />
              <span className="kb-menu__label">All projects</span>
              <span className="kb-menu__hint">{totals}</span>
            </ShadcnDropdownMenuRadioItem>
          </ShadcnDropdownMenuGroup>
          {active.length > 0 ? (
            <ShadcnDropdownMenuGroup>
              <ShadcnDropdownMenuLabel>Projects</ShadcnDropdownMenuLabel>
              {active.map(row)}
            </ShadcnDropdownMenuGroup>
          ) : null}
          {archived.length > 0 ? (
            <ShadcnDropdownMenuGroup>
              <ShadcnDropdownMenuLabel>Archived</ShadcnDropdownMenuLabel>
              {archived.map(row)}
            </ShadcnDropdownMenuGroup>
          ) : null}
        </ShadcnDropdownMenuRadioGroup>
        <ShadcnDropdownMenuSeparator />
        <ShadcnDropdownMenuGroup>
          <ShadcnDropdownMenuItem className="kb-menu__row" onClick={onNewProject}>
            <Plus size={16} />
            New project…
          </ShadcnDropdownMenuItem>
          {onOpenReleases ? (
            <ShadcnDropdownMenuItem className="kb-menu__row" onClick={onOpenReleases}>
              <Milestone size={16} />
              Releases
            </ShadcnDropdownMenuItem>
          ) : null}
        </ShadcnDropdownMenuGroup>
      </ShadcnDropdownMenuContent>
    </ShadcnDropdownMenu>
  )
}

const VIEW_META: Record<HubView, { label: string; icon: typeof Columns3 }> = {
  overview: { label: 'Overview', icon: LayoutGrid },
  board: { label: 'Board', icon: Columns3 },
  tree: { label: 'Tree', icon: Network },
}

/** Overview · Board · Tree, the header's line tabs. Icon-only when the pane is narrow. */
export function HubTabs({
  value,
  onChange,
  narrow,
}: {
  value: HubView
  onChange: (view: HubView) => void
  narrow: boolean
}) {
  return (
    <ShadcnTabs
      className="kb-hub-tabs"
      onValueChange={(next: string) => onChange(coerceView(next, value))}
      value={value}
    >
      <ShadcnTabsList aria-label="View" variant="line">
        {HUB_VIEWS.map((view) => {
          const { label, icon: Icon } = VIEW_META[view]
          return (
            <ShadcnTabsTrigger
              aria-label={narrow ? label : undefined}
              key={view}
              title={narrow ? label : undefined}
              value={view}
            >
              <Icon size={16} />
              {narrow ? null : label}
            </ShadcnTabsTrigger>
          )
        })}
      </ShadcnTabsList>
    </ShadcnTabs>
  )
}
