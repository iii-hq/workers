import { Badge, Button, EmptyState, IconButton, List, ListItem, PageBody, PageMain, PageSidebar, SearchField, Skeleton, StatusPanel, Toolbar, type Host } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { ArrowLeft, MessageCircle, RefreshCw, Users } from 'lucide-react'
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
  return group.subject?.trim() || 'Grupo sem nome'
}

function participantRole(participant: GroupParticipant): string {
  if (participant.participant_type === 'superadmin') return 'Administrador principal'
  if (participant.participant_type === 'admin') return 'Administrador'
  return 'Participante'
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

  const query = search.trim().toLocaleLowerCase('pt-BR')
  const visibleGroups = (groups ?? []).filter((group) => `${group.subject ?? ''} ${group.id}`.toLocaleLowerCase('pt-BR').includes(query))
  const overview = groups?.find((group) => group.id === selected)
  const detail = selected ? details[selected] : undefined
  const group = detail ?? overview
  const peopleQuery = participantSearch.trim().toLocaleLowerCase('pt-BR')
  const participants = (detail?.participants ?? []).filter((participant) => [participant.jid, participant.phone_number, participant.lid, participant.username, participantRole(participant)].filter(Boolean).join(' ').toLocaleLowerCase('pt-BR').includes(peopleQuery))
  const participantCount = group?.participant_count ?? detail?.participants.length
  const disconnected = <StatusPanel variant="warn" headline="Sessão desconectada" detail={groups ? 'Exibindo os últimos dados consultados. Reconecte a conta na aba Conexão para atualizar.' : 'Conecte a conta na aba Conexão para consultar os grupos.'} />

  return <PageBody side={panelSide} className="wa-ui-browser">
    {(!narrow || !selected) && <PageSidebar label="Grupos do WhatsApp" header="Grupos" side={panelSide} narrow={narrow} defaultWidth={300} minWidth={220} maxWidth={360}>
      <div className="wa-ui-list-tools">
        <SearchField aria-label="Buscar grupos por nome ou identificador" value={search} onChange={setSearch} placeholder="Buscar grupos" />
        <Button variant="ghost" onClick={() => void refreshGroups()} disabled={!connected || listLoading}><RefreshCw size={16} aria-hidden="true" />{listLoading ? 'Atualizando…' : 'Atualizar grupos'}</Button>
        {!connected && !selected && disconnected}
        {listError && <StatusPanel variant="alert" headline="Não foi possível atualizar os grupos" detail={listError} action={<Button variant="ghost" onClick={() => void refreshGroups()} disabled={!connected || listLoading}>Tentar novamente</Button>} />}
      </div>
      <div className="wa-ui-list" aria-busy={listLoading}>
        {listLoading && groups === null ? <Skeleton className="wa-ui-loading" /> : groups === null ? <EmptyState compact icon={Users} title={connected ? 'Grupos indisponíveis' : 'Conecte a conta'} description={connected ? 'Use Atualizar grupos para tentar a consulta novamente.' : 'Os grupos ficam disponíveis quando a sessão está conectada.'} /> : visibleGroups.length === 0 ? <EmptyState compact icon={Users} title={query ? 'Nenhum grupo encontrado' : 'Nenhum grupo disponível'} description={query ? 'Tente outro nome ou identificador.' : 'Esta conta não retornou grupos dos quais participa.'} /> : <List aria-label="Grupos dos quais a conta participa">{visibleGroups.map((item) => <ListItem key={item.id} ref={(element) => { if (element) rowRefs.current.set(item.id, element); else rowRefs.current.delete(item.id) }} selected={selected === item.id} leading={<Users size={16} aria-hidden="true" />} label={groupName(item)} description={<span className="wa-ui-mono">{item.id}</span>} trailing={item.is_community ? <Badge>Comunidade</Badge> : item.participant_count !== null ? <span aria-label={`${item.participant_count} participantes`}>{item.participant_count}</span> : undefined} onClick={() => select(item.id)} />)}</List>}
      </div>
    </PageSidebar>}
    {(!narrow || selected) && <PageMain className="wa-ui-reader">
      {selected ? <>
        <Toolbar aria-label="Navegação do grupo" end={<IconButton label="Atualizar detalhes do grupo" onClick={() => void refreshDetail()} disabled={!connected || detailLoading}><RefreshCw size={16} /></IconButton>}>
          {narrow && <Button ref={backRef} variant="ghost" onClick={() => select(null)}><ArrowLeft size={16} aria-hidden="true" />Voltar aos grupos</Button>}
          {!narrow && <span className="wa-ui-note">Detalhes do grupo</span>}
        </Toolbar>
        <div className="wa-ui-detail-header">
          <div className="wa-ui-stack">
            <h3 className="wa-ui-title">{group ? groupName(group) : 'Grupo'}</h3>
            <p className="wa-ui-note wa-ui-mono wa-ui-identity">{selected}</p>
            {participantCount !== undefined && <p className="wa-ui-note">{participantCount} {participantCount === 1 ? 'participante' : 'participantes'}</p>}
          </div>
          <Button variant="ghost" onClick={() => onOpenMessages({ chat_id: selected, title: group ? groupName(group) : selected, is_group: true })}><MessageCircle size={16} aria-hidden="true" />Abrir mensagens</Button>
        </div>
        <div className="wa-ui-detail-body" aria-busy={detailLoading}>
          {!connected && disconnected}
          {detailError && <StatusPanel variant="alert" headline="Não foi possível atualizar os detalhes" detail={detailError} action={<Button variant="ghost" onClick={() => void refreshDetail()} disabled={!connected || detailLoading}>Tentar novamente</Button>} />}
          {detailLoading && !detail ? <Skeleton className="wa-ui-loading" /> : detail ? <div className="wa-ui-stack">
            {(detail.is_community || detail.announcement) && <div className="wa-ui-actions">{detail.is_community && <Badge>Comunidade</Badge>}{detail.announcement && <Badge>Apenas administradores enviam mensagens</Badge>}</div>}
            <section className="wa-ui-stack" aria-label="Descrição do grupo">
              <h4 className="wa-ui-title">Descrição</h4>
              <p className="wa-ui-group-description">{detail.description?.trim() || 'Este grupo não tem descrição.'}</p>
            </section>
            <section className="wa-ui-section" aria-label="Participantes do grupo">
              <h4 className="wa-ui-title">Participantes</h4>
              <SearchField aria-label="Buscar participantes por nome, telefone ou identificador" value={participantSearch} onChange={setParticipantSearch} placeholder="Buscar participantes" />
              {participants.length ? <List role="list" aria-label="Participantes">{participants.map((participant) => {
                const name = participant.username?.trim()
                const identity = participantIdentity(participant)
                const administrator = participant.participant_type === 'admin' || participant.participant_type === 'superadmin'
                return <ListItem as="div" role="listitem" key={participant.jid} label={<span title={participant.jid}>{name || identity}</span>} description={name || administrator ? <span className="wa-ui-participant-meta">{name && name !== identity && <span className="wa-ui-mono">{identity}</span>}{administrator && <Badge>{participantRole(participant)}</Badge>}</span> : undefined} />
              })}</List> : <EmptyState compact title={peopleQuery ? 'Nenhum participante encontrado' : 'Lista de participantes indisponível'} description={peopleQuery ? 'Tente outro nome, telefone ou identificador.' : 'O WhatsApp não retornou os participantes deste grupo.'} />}
            </section>
          </div> : !detailLoading && <EmptyState compact title="Detalhes ainda não consultados" description={connected ? 'Atualize os detalhes para consultar a descrição e os participantes.' : 'Reconecte a conta para consultar a descrição e os participantes.'} />}
        </div>
      </> : <EmptyState icon={Users} title="Explore os grupos da conta" description="Selecione um grupo para ver a descrição, consultar os participantes ou abrir as mensagens." />}
    </PageMain>}
  </PageBody>
}
