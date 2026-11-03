import { Button, Dialog, DialogContent, DialogDescription, DialogTitle, Input, StatusPanel } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useEffect, useRef, useState } from 'react'
import type { Client, GroupSummary, InvestigationManyResult, InvestigationProfile } from '../api'
import { CONTEXT_MAX_BYTES, MEMBER_EVIDENCE_MAX_BYTES, groupInvestigationProblem, hypothesisBytes, intentSignature } from './group-investigation.js'
import { ProfilePicker } from './ProfilePicker'

export function InvestigateMany({ api, groups, onCancel, onStarted }: {
  api: Client
  groups: GroupSummary[]
  onCancel: () => void
  onStarted: (result: InvestigationManyResult) => void
}) {
  const [profiles, setProfiles] = useState<InvestigationProfile[]>([])
  const [loading, setLoading] = useState(true)
  const [catalogError, setCatalogError] = useState('')
  const [load, setLoad] = useState(0)
  const [profile, setProfile] = useState('')
  const [hypothesis, setHypothesis] = useState('')
  const [error, setError] = useState('')
  const [pending, setPending] = useState(false)
  const [pickerOpen, setPickerOpen] = useState(false)
  const gate = useRef(false)
  const intent = useRef({ signature: '', id: '' })
  useEffect(() => {
    let live = true
    setLoading(true)
    setCatalogError('')
    api.profiles().then(({ agents }) => { if (live) setProfiles(agents) }).catch(cause => { if (live) setCatalogError(errorMessage(cause)) }).finally(() => { if (live) setLoading(false) })
    return () => { live = false }
  }, [api, load])
  const ids = groups.map(group => group.id)
  const problem = groupInvestigationProblem(ids, hypothesis)
  const available = profiles.some(entry => entry.id === profile && !entry.inheritance_error)
  const submit = async () => {
    if (gate.current || problem || !available || loading) return
    gate.current = true
    setPending(true)
    setError('')
    const signature = intentSignature(ids, profile, hypothesis)
    if (intent.current.signature !== signature) intent.current = { signature, id: crypto.randomUUID() }
    try {
      const result = await api.investigateMany(ids, profile, hypothesis || undefined, intent.current.id)
      onStarted(result)
    } catch (cause) {
      const detail = errorMessage(cause)
      setError(detail.includes('investigation_overlap') ? `Investigation overlap: no partial session was started. ${detail}` : detail)
    } finally {
      gate.current = false
      setPending(false)
    }
  }
  // Radix requests dismissal in document capture, before the picker's keydown.
  // Consume that request while the nested list is open; the next Escape closes
  // the dialog normally. Pending requests never dismiss either investigation.
  return <Dialog open onOpenChange={open => {
    if (open || gate.current) return
    if (pickerOpen) { setPickerOpen(false); return }
    onCancel()
  }}>
    <DialogContent className="sentinel-ui-dialog sentinel-ui-group-dialog" onPointerDownCapture={event => {
      if (pickerOpen && !(event.target as HTMLElement).closest('.sentinel-ui-profile-picker')) setPickerOpen(false)
    }}>
      <DialogTitle>Investigate in group</DialogTitle>
      <DialogDescription>Review 2–5 issues for one restricted, read-only investigation. Compare evidence without assuming a shared cause. No issues are merged, resolved or ignored.</DialogDescription>
      <ul className="sentinel-ui-group-review" aria-label="Issues to investigate">{groups.map(group => <li key={group.id}><strong>{group.title}</strong><span>{group.id} · {group.service_name} · {group.status}</span></li>)}</ul>
      <p className="sentinel-ui-quiet">Evidence excerpt: up to {MEMBER_EVIDENCE_MAX_BYTES} UTF-8 bytes per issue. The request context, including issue envelopes and hypothesis, is limited to {CONTEXT_MAX_BYTES} bytes. Missing evidence and excerpt cuts are reported explicitly by the investigation. Profile selection never grants additional tools or write access.</p>
      {catalogError ? <StatusPanel variant="alert" headline="Could not load installed profiles" detail={catalogError} action={<Button onClick={() => setLoad(value => value + 1)}>Retry profiles</Button>} /> : null}
      <div className="sentinel-ui-group-field"><span>Investigation profile</span><ProfilePicker profiles={profiles} value={profile} open={pickerOpen} onOpenChange={setPickerOpen} disabled={pending || loading || !!catalogError} loading={loading} onChange={value => { setProfile(value); setError('') }} /></div>
      {!loading && !catalogError && profiles.length === 0 ? <StatusPanel variant="warn" headline="No installed profiles available" detail="Install or repair a profile outside this investigation flow, then retry." /> : null}
      <label className="sentinel-ui-group-field"><span>Shared-cause hypothesis (optional)</span><Input aria-label="Shared-cause hypothesis (optional)" aria-describedby="sentinel-hypothesis-limit" value={hypothesis} disabled={pending} onChange={value => { setHypothesis(value); setError('') }} /></label>
      <span id="sentinel-hypothesis-limit" className="sentinel-ui-quiet">{hypothesisBytes(hypothesis)} / 2000 UTF-8 bytes. This is a hypothesis, not a conclusion.</span>
      {problem ? <StatusPanel variant="warn" headline={problem} /> : null}
      {error ? <StatusPanel variant="alert" role="alert" headline="Could not start the group investigation" detail={error} /> : null}
      <div className="sentinel-ui-dialog-actions" aria-busy={pending}><Button variant="ghost" disabled={pending} onClick={onCancel}>Cancel</Button><Button disabled={pending || loading || !!catalogError || !!problem || !available} onClick={submit}>{pending ? 'Starting investigation…' : 'Start investigation'}</Button></div>
    </DialogContent>
  </Dialog>
}
