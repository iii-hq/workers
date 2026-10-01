import { Badge, Button, EmptyState, IconButton, List, ListItem, PageBody, PageMain, PageSidebar, SearchField, Skeleton, StatusPanel, type Host } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { ArrowLeft, ChevronDown, Info, MessageCircle, RefreshCw, UserRound, Users, WifiOff } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { identityLabel } from './inbox-model'
import { requestScope } from './model'

interface GroupOverview {
  id: string
  subject: string | null
  participant_count: number | null
  is_community: boolean
  parent_group_id: string | null
}

interface GroupParticipant {
  jid: string
  phone_number: string | null
  lid: string | null
  username: string | null
  participant_type: string
}

interface GroupDetail extends GroupOverview {
  description: string | null
  participants: GroupParticipant[]
  announcement: boolean
}

export interface GroupBrowserProps {
  host: Host
  sessionId: string
  connected: boolean
  narrow: boolean
  panelSide?: 'left' | 'right'
  onOpenMessages(chat: { chat_id: string; title: string; is_group: boolean }): void
}

function groupName(group: GroupOverview): string {
  return group.subject?.trim() || 'Unnamed group'
}

function participantRole(participant: GroupParticipant): string {
  if (participant.participant_type === 'superadmin') return 'Owner'
  if (participant.participant_type === 'admin') return 'Admin'
  return 'Member'
}

function participantIdentity(participant: GroupParticipant): string {
  const identifier = participant.phone_number || participant.jid
  return /^\+?\d+$/.test(identifier) ? `+${identifier.replace(/^\+/, '')}` : identityLabel(identifier)
}

export function GroupBrowser({ host, sessionId, connected, narrow, panelSide, onOpenMessages }: GroupBrowserProps) {
  const [groups, setGroups] = useState<GroupOverview[] | null>(null)
  const [selected, setSelected] = useState<string | null>(null)
  const [details, setDetails] = useState<Record<string, GroupDetail>>({})
  const [search, setSearch] = useState('')
  const [participantSearch, setParticipantSearch] = useState('')
  const [listLoading, setListLoading] = useState(connected)
  const [detailLoading, setDetailLoading] = useState(false)
  const [listError, setListError] = useState<string | null>(null)
  const [detailError, setDetailError] = useState<string | null>(null)
  const listScope = useRef(requestScope())
  const detailScope = useRef(requestScope())
  const selectedRef = useRef(selected)
  const connectedRef = useRef(connected)
  const backRef = useRef<HTMLButtonElement>(null)
  const rowRefs = useRef(new Map<string, HTMLButtonElement>())
  const lastSelected = useRef<string | null>(null)
  selectedRef.current = selected
  connectedRef.current = connected

  const refreshGroups = useCallback(async () => {
    if (!connected) return
    const current = listScope.current.begin()
    setListLoading(true)
    setListError(null)
    try {
      const result = await host.iii.trigger<GroupOverview[]>('whatsapp::groups::list', { session_id: sessionId })
      if (!current() || !connectedRef.current) return
      setGroups(result)
      setDetails((previous) => Object.fromEntries(result.flatMap((group) => previous[group.id] ? [[group.id, previous[group.id]]] : [])))
      if (selectedRef.current && !result.some((group) => group.id === selectedRef.current)) {
        detailScope.current.invalidate()
        selectedRef.current = null
        setSelected(null)
        setDetailError(null)
        setDetailLoading(false)
      }
    } catch (cause) {
      if (current() && connectedRef.current) setListError(errorMessage(cause))
    } finally {
      if (current()) setListLoading(false)
    }
  }, [host.iii, sessionId, connected])

  const refreshDetail = useCallback(async () => {
    if (!connected || !selected) return
    const groupId = selected
    const current = detailScope.current.begin()
    setDetailLoading(true)
    setDetailError(null)
    try {
      const result = await host.iii.trigger<GroupDetail>('whatsapp::groups::get', { session_id: sessionId, group_id: groupId })
      if (!current() || !connectedRef.current || selectedRef.current !== groupId) return
      setDetails((previous) => ({ ...previous, [groupId]: result }))
    } catch (cause) {
      if (current() && connectedRef.current && selectedRef.current === groupId) setDetailError(errorMessage(cause))
    } finally {
      if (current()) setDetailLoading(false)
    }
  }, [host.iii, sessionId, connected, selected])

  useEffect(() => {
    if (connected) void refreshGroups()
    else setListLoading(false)
    return () => listScope.current.invalidate()
  }, [connected, refreshGroups])

  useEffect(() => {
    setDetailError(null)
    setParticipantSearch('')
    if (connected && selected) void refreshDetail()
    else setDetailLoading(false)
    return () => detailScope.current.invalidate()
  }, [connected, selected, refreshDetail])

  useEffect(() => {
    if (narrow && selected) backRef.current?.focus()
    else if (!selected && lastSelected.current) rowRefs.current.get(lastSelected.current)?.focus()
  }, [narrow, selected])

  const select = (id: string | null) => {
    if (id === selected) return
    detailScope.current.invalidate()
    if (id) lastSelected.current = id
    selectedRef.current = id
    setSelected(id)
    setDetailError(null)
    setDetailLoading(false)
    setParticipantSearch('')
  }

  const query = search.trim().toLocaleLowerCase('en-US')
  const visibleGroups = (groups ?? []).filter((group) => `${group.subject ?? ''} ${group.id}`.toLocaleLowerCase('en-US').includes(query))
  const overview = groups?.find((group) => group.id === selected)
  const detail = selected ? details[selected] : undefined
  const group = detail ?? overview
  const peopleQuery = participantSearch.trim().toLocaleLowerCase('en-US')
  const participants = (detail?.participants ?? []).filter((participant) => [participant.jid, participant.phone_number, participant.lid, participant.username, participantRole(participant)].filter(Boolean).join(' ').toLocaleLowerCase('en-US').includes(peopleQuery))
  const participantCount = group?.participant_count ?? detail?.participants.length
  const disconnected = <div className="wa-ui-inline-status" data-tone="warn" role="status"><WifiOff size={16} aria-hidden="true" /><span>{groups ? 'Offline · showing saved group details' : 'Connect this account in Connection to browse groups.'}</span></div>

  return <PageBody side={panelSide} className="wa-ui-browser">
    {(!narrow || !selected) && <PageSidebar label="WhatsApp groups" header={<div className="wa-ui-sidebar-heading"><span>Groups</span><IconButton label={listLoading ? 'Refreshing groups' : 'Refresh groups'} onClick={() => void refreshGroups()} disabled={!connected || listLoading}><RefreshCw size={16} /></IconButton></div>} side={panelSide} narrow={narrow} defaultWidth={320} minWidth={240} maxWidth={400}>
      <div className="wa-ui-list-tools">
        <SearchField aria-label="Search groups by name or identifier" value={search} onChange={setSearch} placeholder="Search groups" />
        <span className="wa-ui-note">{groups ? `${visibleGroups.length.toLocaleString('en-US')} ${visibleGroups.length === 1 ? 'group' : 'groups'}` : 'Your account’s groups'}</span>
        {!connected && !selected && disconnected}
        {listError && <StatusPanel variant="alert" headline="Could not refresh groups" detail={listError} action={<Button variant="ghost" onClick={() => void refreshGroups()} disabled={!connected || listLoading}>Try again</Button>} />}
      </div>
      <div className="wa-ui-list" aria-busy={listLoading}>
        {listLoading && groups === null ? <Skeleton className="wa-ui-loading" /> : groups === null ? <EmptyState compact icon={Users} title={connected ? 'Groups unavailable' : 'Connect your account'} description={connected ? 'Refresh groups to try again.' : 'Groups are available while the account is connected.'} /> : visibleGroups.length === 0 ? <EmptyState compact icon={Users} title={query ? 'No groups found' : 'No groups available'} description={query ? 'Try another name or identifier.' : 'This account did not return any groups.'} /> : <List aria-label="Groups this account belongs to">{visibleGroups.map((item) => <ListItem key={item.id} className="wa-ui-chat-item" ref={(element) => { if (element) rowRefs.current.set(item.id, element); else rowRefs.current.delete(item.id) }} selected={selected === item.id}
          leading={<span className="wa-ui-avatar" data-group aria-hidden="true"><Users size={16} /></span>}
          label={<span className="wa-ui-chat-title">{groupName(item)}</span>}
          description={<span className="wa-ui-preview" title={item.id}>{item.participant_count !== null ? `${item.participant_count.toLocaleString('en-US')} ${item.participant_count === 1 ? 'member' : 'members'}` : item.id}{item.is_community ? ' · Community' : ''}</span>}
          onClick={() => select(item.id)} />)}</List>}
      </div>
    </PageSidebar>}
    {(!narrow || selected) && <PageMain className="wa-ui-reader">
      {selected ? <>
        <header className="wa-ui-detail-header wa-ui-compact-header">
          {narrow && <IconButton ref={backRef} label="Back to groups" onClick={() => select(null)}><ArrowLeft size={16} /></IconButton>}
          <span className="wa-ui-avatar wa-ui-avatar-header" data-group aria-hidden="true"><Users size={16} /></span>
          <div className="wa-ui-heading-copy">
            <h2 className="wa-ui-title">{group ? groupName(group) : 'Group'}</h2>
            <span className="wa-ui-note wa-ui-identity" title={selected}>{participantCount !== undefined ? `${participantCount.toLocaleString('en-US')} ${participantCount === 1 ? 'member' : 'members'} · ` : ''}{group?.is_community ? 'Community' : 'Group'}</span>
          </div>
          <div className="wa-ui-header-actions">
            <IconButton label="Refresh group details" onClick={() => void refreshDetail()} disabled={!connected || detailLoading}><RefreshCw size={16} /></IconButton>
            {narrow ? <IconButton label="Open group messages" onClick={() => onOpenMessages({ chat_id: selected, title: group ? groupName(group) : selected, is_group: true })}><MessageCircle size={16} /></IconButton> : <Button onClick={() => onOpenMessages({ chat_id: selected, title: group ? groupName(group) : selected, is_group: true })}><MessageCircle size={16} aria-hidden="true" />Messages</Button>}
          </div>
        </header>
        <div className="wa-ui-detail-body" aria-busy={detailLoading}>
          {!connected && disconnected}
          {detailError && <StatusPanel variant="alert" headline="Could not refresh group details" detail={detailError} action={<Button variant="ghost" onClick={() => void refreshDetail()} disabled={!connected || detailLoading}>Try again</Button>} />}
          {detailLoading && !detail ? <Skeleton className="wa-ui-loading" /> : detail ? <div key={selected} className="wa-ui-group-sections">
            <details className="wa-ui-group-section">
              <summary><Info size={16} aria-hidden="true" /><span>About this group</span><ChevronDown size={16} aria-hidden="true" /></summary>
              <div className="wa-ui-group-section-content">
                {detail.announcement && <Badge>Only admins can send</Badge>}
                <p className="wa-ui-group-description">{detail.description?.trim() || 'This group has no description.'}</p>
                <p className="wa-ui-note wa-ui-mono wa-ui-identity">{selected}</p>
              </div>
            </details>
            <details className="wa-ui-group-section" open>
              <summary><Users size={16} aria-hidden="true" /><span>Members</span><span className="wa-ui-note">{detail.participants.length.toLocaleString('en-US')}</span><ChevronDown size={16} aria-hidden="true" /></summary>
              <div className="wa-ui-group-section-content">
                <SearchField aria-label="Search members by name, phone number or identifier" value={participantSearch} onChange={setParticipantSearch} placeholder="Search members" />
                {participants.length ? <List role="list" aria-label="Group members">{participants.map((participant) => {
                  const name = participant.username?.trim()
                  const identity = participantIdentity(participant)
                  const administrator = participant.participant_type === 'admin' || participant.participant_type === 'superadmin'
                  return <ListItem as="div" role="listitem" key={participant.jid} className="wa-ui-member-item" leading={<span className="wa-ui-avatar" aria-hidden="true">{name ? Array.from(name).slice(0, 1).join('').toLocaleUpperCase('en-US') : <UserRound size={16} />}</span>} label={<span className="wa-ui-chat-title" title={participant.jid}>{name || identity}</span>} description={name && name !== identity ? <span className="wa-ui-participant-meta wa-ui-mono" title={identity}>{identity}</span> : undefined} trailing={administrator ? <Badge>{participantRole(participant)}</Badge> : undefined} />
                })}</List> : <EmptyState compact title={peopleQuery ? 'No members found' : 'Member list unavailable'} description={peopleQuery ? 'Try another name, phone number or identifier.' : 'WhatsApp did not return members for this group.'} />}
              </div>
            </details>
          </div> : !detailLoading && <EmptyState compact title="Group details are not loaded" description={connected ? 'Refresh to view this group’s description and members.' : 'Reconnect to view this group’s description and members.'} />}
        </div>
      </> : <EmptyState icon={Users} title="Select a group" description="Choose a group to explore its members, read the description, or open its messages." />}
    </PageMain>}
  </PageBody>
}
