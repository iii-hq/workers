/**
 * The board filters (design §2, §8.2): the text, when, and chips with their
 * counts for assignee, priority, labels, project and release, applied as
 * they change. The first filter reads the whole board, so the counts and the
 * lanes see every card.
 */

import {
  Button,
  Chip,
  Column,
  createMemo,
  createSignal,
  For,
  type Hue,
  iii,
  Page,
  Row,
  Scroll,
  Segmented,
  Show,
  Spinner,
  StatusPanel,
  Text,
  TextField,
} from '@iii-dev/native-ui'
import {
  activeCount,
  allLabels,
  EMPTY_FILTERS,
  type FacetKey,
  type Filters,
  facetCounts,
  matches,
  toggled,
  UPDATED_WINDOWS,
} from '../filters'
import { hueOf, priorityLabel } from '../format'
import {
  agents,
  boardRows,
  complete,
  filters,
  loadError,
  matchContext,
  meta,
  nameOf,
  online,
  projects,
  readAll,
  readingAll,
  releases,
  setFilters,
} from '../state'

/** Labels shown before "Show all". */
const LABELS_SHOWN = 30

interface Option {
  value: string
  label: string
  hue?: Hue
}

export function FiltersPage() {
  readAll()
  const [text, setText] = createSignal(filters().q)
  const [allLabelsShown, setAllLabelsShown] = createSignal(false)
  let timer: number | undefined

  const update = (patch: Partial<Filters>) => setFilters({ ...filters(), ...patch })
  const typed = (value: string) => {
    setText(value)
    clearTimeout(timer)
    timer = setTimeout(() => update({ q: value }), 300)
  }

  const rows = () => boardRows()
  const shown = createMemo(() => {
    const f = filters()
    const ctx = matchContext()
    return rows().filter((r) => matches(r, f, ctx)).length
  })
  const counts = (key: FacetKey) => facetCounts(rows(), filters(), matchContext(), key)

  /** The options of a facet: the ones some card has, and the ones chosen. */
  const options = (key: Exclude<FacetKey, 'updated'>, all: Option[]) => {
    const n = counts(key)
    const chosen = filters()[key]
    return all
      .filter((o) => (n.get(o.value) ?? 0) > 0 || chosen.includes(o.value))
      .map((o) => ({ ...o, n: n.get(o.value) ?? 0 }))
  }

  const assigneeOptions = createMemo(() => {
    const ids = new Set([...agents().map((a) => a.id), ...rows().flatMap((r) => (r.assignee ? [r.assignee] : []))])
    return options('assignees', [
      { value: '', label: 'Unassigned' },
      ...[...ids].map((id) => ({ value: id, label: nameOf(id), hue: hueOf(agents().find((a) => a.id === id)?.color) })),
    ])
  })
  const priorityOptions = createMemo(() =>
    options(
      'priorities',
      [...(meta()?.priorities ?? [])].reverse().map((p) => ({ value: p, label: priorityLabel(p) })),
    ),
  )
  const allLabelOptions = createMemo(() =>
    options(
      'labels',
      allLabels(rows()).map((l) => ({ value: l, label: l })),
    ),
  )
  const labelOptions = () => (allLabelsShown() ? allLabelOptions() : allLabelOptions().slice(0, LABELS_SHOWN))
  const projectOptions = createMemo(() =>
    options('projects', [
      { value: '', label: 'No project' },
      ...projects().map((p) => ({ value: p.id, label: p.name, hue: hueOf(p.color) })),
    ]),
  )
  const releaseOptions = createMemo(() =>
    options('releases', [
      { value: '', label: 'No release' },
      ...releases().map((r) => ({ value: r.id, label: r.name })),
    ]),
  )

  const facet = (title: string, key: Exclude<FacetKey, 'updated'>, list: () => (Option & { n: number })[]) => (
    <Show when={list().length > 0}>
      <Column gap={6}>
        <Text role="micro" tone="muted">
          {title}
        </Text>
        <Row gap={6} wrap>
          <For each={list()}>
            {(o) => (
              <Chip
                text={`${o.label} ${o.n}`}
                hue={o.hue ?? 'none'}
                selected={filters()[key].includes(o.value)}
                onPress={() => update({ [key]: toggled(filters()[key], o.value) } as Partial<Filters>)}
              />
            )}
          </For>
        </Row>
      </Column>
    </Show>
  )

  return (
    <Page
      testTag="filters-page"
      title="Filters"
      subtitle={complete() ? `${shown()} of ${rows().length} tickets` : null}
      top={
        <TextField
          testTag="filter-text"
          padding={[8, 12]}
          value={text()}
          onChange={typed}
          placeholder="Key or title"
          leadingIcon="search"
          keyboard="search"
          imeAction="search"
          clearable
        />
      }
      bottom={
        <Row gap={8} padding={[8, 12]}>
          <Button
            testTag="clear-filters"
            weight={1}
            enabled={activeCount(filters()) > 0}
            onPress={() => {
              setText('')
              setFilters(EMPTY_FILTERS)
            }}
          >
            Clear all
          </Button>
          <Button testTag="show-tickets" weight={1} kind="primary" onPress={() => iii.nav.popToRoot()}>
            {complete() ? `Show ${shown()} ${shown() === 1 ? 'ticket' : 'tickets'}` : 'Show tickets'}
          </Button>
        </Row>
      }
    >
      <Show
        when={complete()}
        fallback={
          <Show
            when={!readingAll() && loadError()}
            fallback={
              <Column gap={8} padding={24} align="center">
                <Spinner />
                <Text role="small" tone="muted">
                  Reading the whole board…
                </Text>
              </Column>
            }
          >
            {(message: () => string) => (
              <StatusPanel
                testTag="filters-error"
                variant="alert"
                headline="Couldn't read the whole board"
                detail={message()}
                padding={[8, 12]}
                action={
                  <Button small enabled={online()} onPress={readAll}>
                    Retry
                  </Button>
                }
              />
            )}
          </Show>
        }
      >
        <Scroll>
          <Column gap={16} padding={[12, 16]}>
            <Column gap={6}>
              <Text role="micro" tone="muted">
                Updated
              </Text>
              <Segmented
                testTag="filter-updated"
                compact
                options={UPDATED_WINDOWS.map((w) => w.label)}
                selected={UPDATED_WINDOWS.findIndex((w) => w.id === filters().updated)}
                onSelect={(i: number) => update({ updated: UPDATED_WINDOWS[i]?.id ?? 'any' })}
              />
            </Column>
            {facet('Assignee', 'assignees', assigneeOptions)}
            {facet('Priority', 'priorities', priorityOptions)}
            {facet('Labels', 'labels', labelOptions)}
            <Show when={filters().labels.length > 1}>
              <Segmented
                testTag="label-mode"
                compact
                options={['Any of them', 'All of them']}
                selected={filters().labelMode === 'all' ? 1 : 0}
                onSelect={(i: number) => update({ labelMode: i === 1 ? 'all' : 'any' })}
              />
            </Show>
            <Show when={!allLabelsShown() && allLabelOptions().length > LABELS_SHOWN}>
              <Button small kind="ghost" onPress={() => setAllLabelsShown(true)}>
                Show all labels
              </Button>
            </Show>
            {facet('Project', 'projects', projectOptions)}
            {facet('Release', 'releases', releaseOptions)}
          </Column>
        </Scroll>
      </Show>
    </Page>
  )
}
