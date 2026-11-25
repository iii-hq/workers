import {
  ActionLine,
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  IconButton,
  Input,
  Select,
  SettingsField,
  SettingsList,
  StatusPanel,
  Switch,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import { Check, Copy, Globe, Laptop, Link2 } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import type { ArtifactApi, ArtifactRecord, ShareLimits, ShareView } from '../lib/api'
import { isWaitingForUrl, WAITING_POLL_MS } from '../lib/refresh'
import { versionOptions } from './VersionSelect'

/** Lifetimes offered, filtered by the configured maximum. */
const LIFETIMES = [1, 6, 24, 72, 168, 336, 720]
const MIN_PASSCODE = 6

function lifetimeLabel(hours: number): string {
  if (hours < 24) return hours === 1 ? '1 hour' : `${hours} hours`
  const days = hours / 24
  return days === 1 ? '1 day' : `${days} days`
}

/** The offered lifetimes: every standard one up to the maximum, plus the
    configured default and maximum themselves. */
export function lifetimeOptions(limits: ShareLimits): number[] {
  const set = new Set(LIFETIMES.filter((h) => h <= limits.max_share_hours))
  set.add(limits.default_share_hours)
  set.add(limits.max_share_hours)
  return [...set].sort((a, b) => a - b)
}

export function CopyLink({ url, label }: { url: string; label: string }) {
  const { state, copy } = useCopyFlash(url)
  return (
    <div className="artifact-ui-link">
      <code className="artifact-ui-link-url">{url}</code>
      <IconButton variant="ghost" label={state === 'copied' ? 'Copied' : `Copy ${label}`} onClick={copy}>
        {state === 'copied' ? <Check /> : <Copy />}
      </IconButton>
    </div>
  )
}

/** The links of one share, with what to do when the public one is missing. */
export function ShareLinks({ share }: { share: ShareView }) {
  return (
    <div className="artifact-ui-links">
      {share.public_url ? (
        <ActionLine icon={<Globe />} tone="accent">
          <CopyLink url={share.public_url} label="public link" />
        </ActionLine>
      ) : null}
      <ActionLine icon={<Laptop />} tone="ink">
        <CopyLink url={share.local_url} label="local link" />
      </ActionLine>
      {share.public && !share.public_url ? (
        <StatusPanel
          variant={share.tunnel_error ? 'warn' : 'info'}
          headline={
            share.tunnel_error
              ? 'No public link yet'
              : share.tunnel_status === 'warming'
                ? 'Public link warming up'
                : 'The tunnel is starting'
          }
          detail={
            share.tunnel_error ??
            (share.tunnel_status === 'warming'
              ? 'The new address is still reaching public DNS, usually within a minute. The link appears here on its own.'
              : 'The public link appears here once the tunnel is ready.')
          }
        />
      ) : null}
    </div>
  )
}

interface ShareDialogProps {
  api: ArtifactApi
  artifact: ArtifactRecord
  /** Version to preselect; the latest by default. */
  version?: number
  open: boolean
  onOpenChange(open: boolean): void
  onShared?(share: ShareView): void
}

/** Pick a version, a lifetime, the audience and optional protection, then
    create the link. */
export function ShareDialog({ api, artifact, version, open, onOpenChange, onShared }: ShareDialogProps) {
  const [chosenVersion, setChosenVersion] = useState(String(version ?? artifact.latest_version))
  const [limits, setLimits] = useState<ShareLimits | null>(null)
  const [hours, setHours] = useState<string | undefined>(undefined)
  const [isPublic, setPublic] = useState(true)
  const [passcode, setPasscode] = useState('')
  const [once, setOnce] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [share, setShare] = useState<ShareView | null>(null)
  /** Bumped on every open: a request only touches the dialog session that
      started it. */
  const session = useRef(0)

  // The version to preselect, read when the dialog opens and not after: a
  // new version arriving while it is open must not wipe the draft (and with
  // it a passcode the user typed).
  const initialVersion = useRef(version ?? artifact.latest_version)
  initialVersion.current = version ?? artifact.latest_version

  useEffect(() => {
    if (!open) return
    session.current += 1
    const current = session.current
    setChosenVersion(String(initialVersion.current))
    setShare(null)
    setError(null)
    setBusy(false)
    setPasscode('')
    setOnce(false)
    api.shares(artifact.id).then(
      (res) => {
        if (session.current !== current) return
        setLimits(res.limits)
        setHours(String(res.limits.default_share_hours))
      },
      (err) => session.current === current && setError(errorMessage(err)),
    )
  }, [open, artifact.id, api])

  // A link made before its address resolved has no public URL yet: keep
  // re-reading this share until it does, or until the dialog closes.
  useEffect(() => {
    if (!share || !isWaitingForUrl(share)) return
    const current = session.current
    const timer = window.setTimeout(() => {
      api.shares(artifact.id).then(
        (res) => {
          if (session.current !== current) return
          const fresh = res.shares.find((s) => s.token === share.token)
          // A same-content copy still re-arms this effect for the next poll.
          setShare(fresh ? { ...fresh } : { ...share })
        },
        () => session.current === current && setShare({ ...share }),
      )
    }, WAITING_POLL_MS)
    return () => window.clearTimeout(timer)
  }, [share, artifact.id, api])

  const passcodeTooShort = passcode.length > 0 && passcode.length < MIN_PASSCODE

  const create = async () => {
    const current = session.current
    setBusy(true)
    setError(null)
    try {
      const created = await api.share({
        id: artifact.id,
        version: Number(chosenVersion),
        hours: hours ? Number(hours) : undefined,
        public: isPublic,
        passcode: passcode || undefined,
        once,
      })
      // The link exists either way: let the list refresh even if this
      // dialog session is gone.
      onShared?.(created)
      if (session.current === current) setShare(created)
    } catch (err) {
      if (session.current === current) setError(errorMessage(err))
    } finally {
      if (session.current === current) setBusy(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="artifact-ui-share-dialog">
        <DialogTitle>Share {artifact.title}</DialogTitle>
        <DialogDescription>
          Anyone with the link (and the passcode, if you set one) can view this version until it
          expires. Later edits do not change what the link shows.
        </DialogDescription>
        {share ? (
          <div className="artifact-ui-share-result">
            <ShareLinks share={share} />
            <p className="artifact-ui-muted">
              v{share.version} · expires {new Date(share.expires_at * 1000).toLocaleString()}
              {share.protected ? ' · passcode required: send it separately' : ''}
              {share.once ? ' · single view' : ''}
            </p>
            <div className="artifact-ui-dialog-actions">
              <Button variant="secondary" onClick={() => onOpenChange(false)}>
                Done
              </Button>
            </div>
          </div>
        ) : (
          <>
            <SettingsList>
              <SettingsField
                label="Version"
                controlSize="compact"
                renderControl={(props) => (
                  <Select {...props} value={chosenVersion} options={versionOptions(artifact)} onChange={setChosenVersion} />
                )}
              />
              <SettingsField
                label="Expires after"
                controlSize="compact"
                renderControl={(props) => (
                  <Select
                    {...props}
                    value={hours}
                    placeholder="Loading…"
                    disabled={!limits}
                    options={(limits ? lifetimeOptions(limits) : []).map((h) => ({
                      value: String(h),
                      label: lifetimeLabel(h),
                    }))}
                    onChange={setHours}
                  />
                )}
              />
              <SettingsField
                label="Public link"
                description="Publish through the quick-tunnel. Off: a link that only works on this machine."
                controlSize="fit"
                renderControl={(props) => (
                  <Switch {...props} checked={isPublic} onChange={(e) => setPublic(e.target.checked)} />
                )}
              />
              <SettingsField
                label="Passcode (optional)"
                description="At least 6 characters. Viewers must enter it; send it apart from the link."
                error={passcodeTooShort ? `Use at least ${MIN_PASSCODE} characters.` : undefined}
                controlSize="default"
                renderControl={(props) => (
                  <Input {...props} type="password" autoComplete="new-password" value={passcode} onChange={setPasscode} />
                )}
              />
              <SettingsField
                label="Single view"
                description="The link stops working after it is opened once. Chat apps that preview links can use up that view: add a passcode."
                controlSize="fit"
                renderControl={(props) => (
                  <Switch {...props} checked={once} onChange={(e) => setOnce(e.target.checked)} />
                )}
              />
            </SettingsList>
            {error ? <StatusPanel variant="alert" headline="Could not share" detail={error} /> : null}
            <div className="artifact-ui-dialog-actions">
              <Button variant="secondary" onClick={() => onOpenChange(false)} disabled={busy}>
                Cancel
              </Button>
              <Button
                variant="primary"
                onClick={create}
                disabled={busy || !limits || passcodeTooShort}
                aria-busy={busy}
              >
                <Link2 />
                {busy ? 'Creating link…' : 'Create link'}
              </Button>
            </div>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}
