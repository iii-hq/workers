/**
 * Releases and projects, to read (D1, design §2): each with its status and
 * progress; a tap shows its tickets on the board (a filter on it) and goes
 * back to the board. Planning them stays in the console.
 */

import {
  Badge,
  Button,
  createList,
  createSignal,
  EmptyState,
  Icon,
  iii,
  List,
  ListRow,
  onMount,
  Page,
  Show,
  Spinner,
  StatusPanel,
  Text,
} from '@iii-dev/native-ui'
import { kanban, type Project, type Release } from '../api'
import { EMPTY_FILTERS } from '../filters'
import { hueOf, shortDate } from '../format'
import { nameOf, online, readable, setFilters, toast } from '../state'

interface PlanRow {
  key: string
  /** 0 an entry, 1 a section header. */
  kind: 0 | 1
  title: string
  subtitle: string
  icon: string
  hue: string
  badge: string
  badgeVariant: string
  hasBadge: boolean
}

const header = (title: string): PlanRow => ({
  key: `h:${title}`,
  kind: 1,
  title,
  subtitle: '',
  icon: '',
  hue: 'none',
  badge: '',
  badgeVariant: 'default',
  hasBadge: false,
})

const progress = (done: number, total: number) => (total ? `${done} of ${total} done` : 'No tickets')

export function releaseRows(list: readonly Release[]): PlanRow[] {
  const row = (r: Release): PlanRow => ({
    key: r.id,
    kind: 0,
    title: r.name,
    subtitle: [shortDate(r.target_at), progress(r.done, r.total)].filter(Boolean).join(' · '),
    icon: 'rocket',
    hue: 'none',
    badge: r.released ? 'Released' : r.status === 'active' ? 'Active' : 'Planned',
    badgeVariant: r.released ? 'ok' : r.status === 'active' ? 'accent' : 'default',
    hasBadge: true,
  })
  const open = list.filter((r) => !r.released)
  const shipped = list.filter((r) => r.released)
  return [...open.map(row), ...(shipped.length ? [header('Released'), ...shipped.map(row)] : [])]
}

export function projectRows(list: readonly Project[]): PlanRow[] {
  const row = (p: Project): PlanRow => ({
    key: p.id,
    kind: 0,
    title: p.name,
    subtitle: [p.lead ? nameOf(p.lead) : '', progress(p.done, p.total)].filter(Boolean).join(' · '),
    icon: 'folder',
    hue: hueOf(p.color),
    badge: 'Archived',
    badgeVariant: 'default',
    hasBadge: p.archived,
  })
  const active = list.filter((p) => !p.archived)
  const archived = list.filter((p) => p.archived)
  return [...active.map(row), ...(archived.length ? [header('Archived'), ...archived.map(row)] : [])]
}

function PlanPage(props: { kind: 'release' | 'project'; title: string; read: () => Promise<PlanRow[]> }) {
  const rows = createList<PlanRow>({ key: 'key', kind: 'kind' })
  const [loaded, setLoaded] = createSignal(false)
  const [error, setError] = createSignal<string | null>(null)
  const load = async () => {
    try {
      rows.set(await props.read())
      setError(null)
    } catch (err) {
      setError(readable(err))
    } finally {
      setLoaded(true)
    }
  }
  onMount(() => void load())

  const show = (item: PlanRow) => {
    if (item.kind !== 0) return
    setFilters({ ...EMPTY_FILTERS, [props.kind === 'release' ? 'releases' : 'projects']: [item.key] })
    iii.nav.popToRoot()
    toast(`Showing ${item.title}`)
  }

  return (
    <Page
      testTag={`${props.kind}s`}
      title={props.title}
      top={
        <Show when={error()}>
          {(message: () => string) => (
            <StatusPanel
              testTag="plan-error"
              variant="alert"
              headline={`Couldn't read the ${props.kind}s`}
              detail={message()}
              padding={[8, 12]}
              action={
                <Button small enabled={online()} onPress={() => void load()}>
                  Retry
                </Button>
              }
            />
          )}
        </Show>
      }
    >
      <List
        testTag="plan-list"
        weight={1}
        source={rows}
        templates={[
          (row) => (
            <ListRow
              title={row.title}
              subtitle={row.subtitle}
              onPress={show}
              leading={<Icon name={row.icon} hue={row.hue} size={18} tone="muted" />}
              trailing={<Badge text={row.badge} variant={row.badgeVariant} visible={row.hasBadge} />}
            />
          ),
          (row) => <Text role="micro" tone="muted" padding={[16, 16, 4, 16]} text={row.title} />,
        ]}
        empty={
          <Show when={loaded()} fallback={<Spinner padding={24} />}>
            <Show when={!error()}>
              <EmptyState
                icon={props.kind === 'release' ? 'rocket' : 'folder'}
                title={`No ${props.kind}s yet`}
                description={`${props.title} are planned in the console.`}
              />
            </Show>
          </Show>
        }
      />
    </Page>
  )
}

export function ReleasesPage() {
  return <PlanPage kind="release" title="Releases" read={() => kanban.releases().then(releaseRows)} />
}

export function ProjectsPage() {
  return <PlanPage kind="project" title="Projects" read={() => kanban.projects().then(projectRows)} />
}
