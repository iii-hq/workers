/**
 * A new ticket (design §2), a modal card: the title, the description, and
 * where it goes (column, priority, assignee, project, release); a subtask
 * also names its parent. Create leaves through `nav.pop({key, status})`.
 */

import {
  Button,
  Column,
  createSignal,
  iii,
  type JsonObject,
  ListRow,
  type NodeRef,
  Page,
  Scroll,
  Show,
  TextField,
  usePage,
} from '@iii-dev/native-ui'
import { kanban } from '../api'
import { priorityLabel, statusLabel } from '../format'
import { columns, meta, nameOf, online, projects, readable, releases, toast, wrote } from '../state'
import { pick, pickAgent } from './pick'

const text = (v: unknown) => (typeof v === 'string' && v ? v : null)

export function NewPage(props: { params: JsonObject }) {
  const page = usePage()
  const m = meta()
  const [title, setTitle] = createSignal('')
  const [description, setDescription] = createSignal('')
  const [status, setStatus] = createSignal(text(props.params.status) ?? m?.defaultStatus ?? '')
  // The first configured priority is the board's default.
  const [priority, setPriority] = createSignal(m?.priorities[0] ?? 'low')
  const [assignee, setAssignee] = createSignal<string | null>(null)
  const [project, setProject] = createSignal<string | null>(text(props.params.project))
  const [release, setRelease] = createSignal<string | null>(text(props.params.release))
  const [creating, setCreating] = createSignal(false)
  const parent = text(props.params.parent)
  let statusRow: NodeRef | undefined
  let priorityRow: NodeRef | undefined

  const chooseStatus = async () => {
    const cols = columns()
    const i = await iii.ui.actionMenu(statusRow ?? null, {
      title: 'Column',
      actions: cols.map((c) => ({ label: c.label, checked: c.id === status() })),
    })
    if (cols[i]) setStatus(cols[i].id)
  }
  const choosePriority = async () => {
    const values = meta()?.priorities ?? []
    const i = await iii.ui.actionMenu(priorityRow ?? null, {
      title: 'Priority',
      actions: values.map((p) => ({ label: priorityLabel(p), checked: p === priority() })),
    })
    if (values[i]) setPriority(values[i])
  }
  const chooseAssignee = async () => {
    const picked = await pickAgent(assignee())
    if (picked !== undefined) setAssignee(picked)
  }
  const chooseFacet = async (kind: 'project' | 'release') => {
    const picked = await pick({ kind, current: kind === 'project' ? project() : release() })
    if (picked) (kind === 'project' ? setProject : setRelease)(picked.value)
  }

  const create = async () => {
    const name = title().trim()
    if (!name || creating()) return
    if (!online()) return toast('The engine is not connected', 'error')
    setCreating(true)
    try {
      const payload: JsonObject = { title: name, status: status(), priority: priority() }
      if (description().trim()) payload.description = description()
      if (assignee()) payload.assignee = assignee()
      if (project()) payload.project = project()
      if (release()) payload.release = release()
      if (parent) payload.parent = parent
      const ticket = await kanban.create(payload)
      wrote(ticket, { entered: true })
      page.pop({ key: ticket.key, status: ticket.status })
    } catch (err) {
      toast(`Couldn't create the ticket: ${readable(err)}`, 'error')
    } finally {
      setCreating(false)
    }
  }

  return (
    <Page
      testTag="new"
      title={parent ? 'New subtask' : 'New ticket'}
      subtitle={parent ? `Subtask of ${parent}` : null}
      bottom={
        <Button
          testTag="create"
          kind="primary"
          padding={[8, 12]}
          loading={creating()}
          enabled={online() && title().trim().length > 0 && !creating()}
          onPress={() => void create()}
        >
          Create ticket
        </Button>
      }
    >
      <Scroll>
        <Column gap={8} padding={[8, 12]}>
          <TextField
            testTag="new-title"
            label="Title"
            value={title()}
            onChange={setTitle}
            autofocus
            imeAction="next"
            placeholder="What needs doing"
          />
          <TextField
            testTag="new-description"
            label="Description"
            value={description()}
            onChange={setDescription}
            lines={4}
            placeholder="Markdown: what, why, and how to check it"
          />
          <ListRow
            testTag="new-status"
            ref={(node) => {
              statusRow = node
            }}
            icon="columns-2"
            title="Column"
            subtitle={statusLabel(columns(), status())}
            chevron
            onPress={() => void chooseStatus()}
          />
          <ListRow
            testTag="new-priority"
            ref={(node) => {
              priorityRow = node
            }}
            icon="flag"
            title="Priority"
            subtitle={priorityLabel(priority())}
            chevron
            onPress={() => void choosePriority()}
          />
          <ListRow
            testTag="new-assignee"
            icon={assignee() ? 'bot' : 'circle-dashed'}
            title="Assignee"
            subtitle={assignee() ? nameOf(assignee() as string) : 'Unassigned'}
            chevron
            onPress={() => void chooseAssignee()}
          />
          <ListRow
            testTag="new-project"
            icon="folder"
            title="Project"
            subtitle={projects().find((p) => p.id === project())?.name ?? 'No project'}
            chevron
            onPress={() => void chooseFacet('project')}
          />
          <ListRow
            testTag="new-release"
            icon="rocket"
            title="Release"
            subtitle={releases().find((r) => r.id === release())?.name ?? 'No release'}
            chevron
            onPress={() => void chooseFacet('release')}
          />
          <Show when={parent}>
            <ListRow icon="corner-down-right" title="Parent" subtitle={parent ?? ''} enabled={false} />
          </Show>
        </Column>
      </Scroll>
    </Page>
  )
}
