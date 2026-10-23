/**
 * The pickers (design §2): one page for an agent, a ticket (a parent, or an
 * existing ticket to attach), a label, a project or a release. Pushed by the
 * ticket screen, the new ticket and the card menu; it leaves through
 * `nav.pop({value})`, where `null` is "none" and Back is no answer at all.
 */

import {
  createEffect,
  createList,
  createMemo,
  createSignal,
  EmptyState,
  Icon,
  iii,
  type JsonObject,
  List,
  ListRow,
  onMount,
  Page,
  Show,
  Spinner,
  Text,
  TextField,
  usePage,
} from '@iii-dev/native-ui'
import { kanban, type Release, type Summary } from '../api'
import { allLabels, fold } from '../filters'
import { hueOf, plural, shortDate, statusLabel } from '../format'
import { keyNumber, pickRefusal } from '../rules'
import { agents, columns, loadAgents, projects, readable, releases } from '../state'

export type PickKind = 'agent' | 'ticket' | 'label' | 'project' | 'release'

export interface PickParams {
  kind: PickKind
  /** The value now (an agent, project or release id), shown selected. */
  current?: string | null
  /** `ticket`: which tickets qualify. */
  mode?: 'parent' | 'attach'
  /** `ticket`: the ticket asking (never a choice). */
  self?: string
  /** `label`: the labels the ticket already has. */
  exclude?: string[]
}

/** Pushes the picker; resolves with the choice (`value: null` = none), or null after Back. */
export async function pick(params: PickParams): Promise<{ value: string | null } | null> {
  const result = await iii.nav.push<{ value: string | null }>('pick', params as unknown as JsonObject)
  if (!result || typeof result !== 'object') return null
  const value = (result as { value?: unknown }).value
  return { value: typeof value === 'string' ? value : null }
}

/** An agent id, null for Unassigned, undefined after Back. */
export async function pickAgent(current: string | null): Promise<string | null | undefined> {
  const picked = await pick({ kind: 'agent', current })
  return picked ? picked.value : undefined
}

interface PickRow {
  key: string
  /** 0 a choice, 1 a section header. */
  kind: 0 | 1
  title: string
  subtitle: string
  icon: string
  hue: string
  selected: boolean
  enabled: boolean
  /** What the choice returns; `` = none. */
  value: string
}

const TITLES: Record<PickKind, string> = {
  agent: 'Assignee',
  ticket: 'Ticket',
  label: 'Add label',
  project: 'Project',
  release: 'Release',
}

const choice = (fields: Partial<PickRow> & { key: string; title: string; value: string }): PickRow => ({
  kind: 0,
  subtitle: '',
  icon: 'circle',
  hue: 'none',
  selected: false,
  enabled: true,
  ...fields,
})
const header = (title: string): PickRow => choice({ key: `h:${title}`, kind: 1, title, value: '', enabled: false })

export function PickPage(props: { params: JsonObject }) {
  const page = usePage()
  const params = props.params as unknown as PickParams
  const kind = params.kind
  const current = typeof params.current === 'string' ? params.current : null
  const rows = createList<PickRow>({ key: 'key', kind: 'kind' })
  const [text, setText] = createSignal('')
  const [query, setQuery] = createSignal('')
  // Tickets and labels need the whole board: read here, the board on screen may hold only its first rows.
  const [tickets, setTickets] = createSignal<Summary[] | null>(null)
  const [error, setError] = createSignal<string | null>(null)
  let timer: number | undefined

  const needsBoard = kind === 'ticket' || kind === 'label'
  onMount(() => {
    if (kind === 'agent') void loadAgents()
    if (needsBoard) {
      kanban.board().then(
        (read) => setTickets(read.columns.flatMap((c) => c.rows)),
        (err: unknown) => setError(readable(err)),
      )
    }
  })

  const typed = (value: string) => {
    setText(value)
    clearTimeout(timer)
    timer = setTimeout(() => setQuery(value.trim()), 200)
  }
  const matchesQuery = (...texts: string[]) => {
    const q = fold(query())
    return !q || texts.some((t) => fold(t).includes(q))
  }

  const all = createMemo((): PickRow[] => {
    switch (kind) {
      case 'agent':
        return [
          choice({ key: 'none', title: 'Unassigned', icon: 'circle-dashed', value: '', selected: current === null }),
          ...agents().map((a) =>
            choice({
              key: a.id,
              title: a.name,
              subtitle: a.id,
              icon: 'bot',
              hue: hueOf(a.color),
              value: a.id,
              selected: a.id === current,
            }),
          ),
        ].filter((r) => matchesQuery(r.title, r.subtitle))
      case 'project': {
        // Active projects; the current one stays listed even when archived.
        const list = projects().filter((p) => !p.archived || p.id === current)
        return [
          choice({ key: 'none', title: 'No project', icon: 'circle-dashed', value: '', selected: current === null }),
          ...list.map((p) =>
            choice({
              key: p.id,
              title: p.archived ? `${p.name} (archived)` : p.name,
              subtitle: plural(p.total, 'ticket'),
              icon: 'folder',
              hue: hueOf(p.color),
              value: p.id,
              selected: p.id === current,
            }),
          ),
        ].filter((r) => matchesQuery(r.title))
      }
      case 'release': {
        const row = (r: Release) =>
          choice({
            key: r.id,
            title: r.name,
            subtitle: [shortDate(r.target_at), plural(r.total, 'ticket')].filter(Boolean).join(' · '),
            icon: 'rocket',
            value: r.id,
            selected: r.id === current,
          })
        const open = releases().filter((r) => !r.released && matchesQuery(r.name))
        const shipped = releases().filter((r) => r.released && matchesQuery(r.name))
        return [
          ...(matchesQuery('No release')
            ? [
                choice({
                  key: 'none',
                  title: 'No release',
                  icon: 'circle-dashed',
                  value: '',
                  selected: current === null,
                }),
              ]
            : []),
          ...open.map(row),
          ...(shipped.length ? [header('Released'), ...shipped.map(row)] : []),
        ]
      }
      case 'label': {
        const have = new Set((params.exclude ?? []).map((l) => l.toLowerCase()))
        const labels = allLabels(tickets() ?? []).filter((l) => !have.has(l.toLowerCase()) && matchesQuery(l))
        const typedLabel = query()
        const exists =
          typedLabel && allLabels(tickets() ?? []).some((l) => l.toLowerCase() === typedLabel.toLowerCase())
        return [
          ...(typedLabel && !exists && !have.has(typedLabel.toLowerCase())
            ? [
                choice({
                  key: `+${typedLabel}`,
                  title: `Create label "${typedLabel}"`,
                  icon: 'plus',
                  value: typedLabel,
                }),
              ]
            : []),
          ...labels.map((l) => choice({ key: l, title: l, icon: 'tag', value: l })),
        ]
      }
      case 'ticket': {
        const list = tickets() ?? []
        const keys = new Map(list.map((t) => [t.id, t.key]))
        const parents = new Set(list.filter((t) => t.parent_id).map((t) => t.parent_id as string))
        const self = typeof params.self === 'string' ? params.self : ''
        const mode = params.mode === 'attach' ? 'attach' : 'parent'
        return list
          .filter((t) => matchesQuery(t.key, t.title))
          .map((t) => ({ t, why: pickRefusal(t, self, mode, parents, keys) }))
          .filter(({ t }) => t.id !== self)
          .sort((a, b) => Number(!!a.why) - Number(!!b.why) || keyNumber(a.t.key) - keyNumber(b.t.key))
          .slice(0, 200)
          .map(({ t, why }) =>
            choice({
              key: t.id,
              title: `${t.key}  ${t.title}`,
              subtitle: why ? `Can't: ${why}` : statusLabel(columns(), t.status),
              icon: 'circle',
              value: mode === 'parent' ? t.key : t.id,
              enabled: !why,
            }),
          )
      }
    }
  })
  createEffect(() => rows.set(all()))

  const choose = (item: PickRow) => {
    if (!item.enabled || item.kind !== 0) return
    page.pop({ value: item.value || null })
  }
  const searchable = () => needsBoard || all().length > 8 || !!text()

  return (
    <Page
      testTag="pick"
      title={TITLES[kind] ?? 'Pick'}
      top={
        <Show when={searchable()}>
          <TextField
            testTag="pick-search"
            padding={[8, 12]}
            value={text()}
            onChange={typed}
            placeholder={kind === 'label' ? 'Find or create a label' : kind === 'ticket' ? 'Key or title' : 'Search'}
            leadingIcon="search"
            keyboard="search"
            imeAction={kind === 'label' ? 'done' : 'search'}
            autofocus={needsBoard}
            clearable
            onSubmit={() => {
              const first = all().find((r) => r.kind === 0 && r.enabled)
              if (kind === 'label' && first) choose(first)
            }}
          />
        </Show>
      }
    >
      <List
        testTag="pick-list"
        weight={1}
        source={rows}
        templates={[
          (row) => (
            <ListRow
              title={row.title}
              subtitle={row.subtitle}
              selected={row.selected}
              enabled={row.enabled}
              onPress={choose}
              leading={<Icon name={row.icon} hue={row.hue} size={18} tone="muted" />}
            />
          ),
          (row) => <Text role="micro" tone="muted" padding={[16, 16, 4, 16]} text={row.title} />,
        ]}
        empty={
          <Show when={!needsBoard || tickets() || error()} fallback={<Spinner padding={24} />}>
            <EmptyState
              testTag="pick-empty"
              icon={error() ? 'triangle-alert' : 'search'}
              title={error() ? "Couldn't read the board" : 'Nothing to pick'}
              description={error() ?? (query() ? 'Nothing matches the search.' : undefined)}
            />
          </Show>
        }
      />
    </Page>
  )
}
