import {
  ActionLine,
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  IconButton,
  Select,
  SettingsField,
  SettingsList,
  StatusPanel,
  Switch,
} from '@iii-dev/console-ui'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import { errorMessage } from '@iii-dev/console-ui/format'
import { Check, Copy, Globe, Laptop, Link2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import type { ArtifactApi, ArtifactRecord, ShareView } from '../lib/api'
import { versionOptions } from './VersionSelect'

const HOURS = ['1', '6', '24', '72'] as const
type Hours = (typeof HOURS)[number]

const HOUR_LABELS: Record<Hours, string> = {
  '1': '1 hour',
  '6': '6 hours',
  '24': '1 day',
  '72': '3 days',
}

export function CopyLink({ url, label }: { url: string; label: string }) {
  const { state, copy } = useCopyFlash(url)
  return (
    <div className="artifact-ui-link">
      <code className="artifact-ui-link-url">{url}</code>
      <IconButton
        variant="ghost"
        label={state === 'copied' ? 'Copied' : `Copy ${label}`}
        onClick={copy}
      >
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
              ? 'The new address is still reaching public DNS, usually within a minute. Reopen the share list to get the link.'
              : 'The public link shows in the share list once the tunnel is ready.')
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

/** Pick a version, a lifetime and the audience, then create the link. */
export function ShareDialog({ api, artifact, version, open, onOpenChange, onShared }: ShareDialogProps) {
  const [chosenVersion, setChosenVersion] = useState(String(version ?? artifact.latest_version))
  const [hours, setHours] = useState<Hours>('24')
  const [isPublic, setPublic] = useState(true)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [share, setShare] = useState<ShareView | null>(null)

  useEffect(() => {
    if (!open) return
    setChosenVersion(String(version ?? artifact.latest_version))
    setShare(null)
    setError(null)
  }, [open, version, artifact.latest_version])

  const create = async () => {
    setBusy(true)
    setError(null)
    try {
      const created = await api.share({
        id: artifact.id,
        version: Number(chosenVersion),
        hours: Number(hours),
        public: isPublic,
      })
      setShare(created)
      onShared?.(created)
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  const versions = versionOptions(artifact)

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="artifact-ui-share-dialog">
        <DialogTitle>Share {artifact.title}</DialogTitle>
        <DialogDescription>
          Anyone with the link can view this version until it expires. Later edits do not change
          what the link shows.
        </DialogDescription>
        {share ? (
          <div className="artifact-ui-share-result">
            <ShareLinks share={share} />
            <p className="artifact-ui-muted">
              v{share.version} · expires {new Date(share.expires_at * 1000).toLocaleString()}
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
                  <Select {...props} value={chosenVersion} options={versions} onChange={setChosenVersion} />
                )}
              />
              <SettingsField
                label="Expires after"
                controlSize="compact"
                renderControl={(props) => (
                  <Select
                    {...props}
                    value={hours}
                    options={HOURS.map((h) => ({ value: h, label: HOUR_LABELS[h] }))}
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
            </SettingsList>
            {error ? <StatusPanel variant="alert" headline="Could not share" detail={error} /> : null}
            <div className="artifact-ui-dialog-actions">
              <Button variant="secondary" onClick={() => onOpenChange(false)} disabled={busy}>
                Cancel
              </Button>
              <Button variant="primary" onClick={create} disabled={busy} aria-busy={busy}>
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
