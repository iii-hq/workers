/**
 * Browser diagnostics: what `browser::doctor` reports about the worker's
 * environment — which engine binary it launches, session capacity, what's allowed,
 * and any degraded capability with how to enable it. Read-only; opening it
 * never starts a browser.
 */

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  EmptyState,
  type Host,
  Skeleton,
  StatusDot,
  StatusPanel,
  Table,
  TableBody,
  TableCell,
  TableRow,
  TableViewport,
} from '@iii-dev/console-ui'
import { useEffect, useState } from 'react'
import { type BrowserDoctorInfo, readBrowserDoctor } from '../lib/browser'

interface DoctorDialogProps {
  host: Host
  open: boolean
  onOpenChange: (open: boolean) => void
}

export function DoctorDialog({ host, open, onOpenChange }: DoctorDialogProps) {
  const [info, setInfo] = useState<BrowserDoctorInfo | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    if (!open) return
    setError(null)
    setInfo(null)
    let stale = false
    void readBrowserDoctor(host.iii)
      .then((res) => {
        if (stale) return
        if (res) setInfo(res)
        else setError('diagnostics unavailable')
      })
      .catch(() => {
        if (!stale) setError('diagnostics unavailable')
      })
    return () => {
      stale = true
    }
  }, [open, host])

  const fact = (label: string, value: string) => (
    <TableRow>
      <TableCell className="br-ui-faint br-ui-td-name">{label}</TableCell>
      <TableCell className="br-ui-mono br-ui-break">{value}</TableCell>
    </TableRow>
  )
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="br-ui-doctor">
        <DialogTitle>Browser diagnostics</DialogTitle>
        <DialogDescription>
          What the worker would launch and what it allows. Read-only.
        </DialogDescription>
        {error ? (
          <StatusPanel variant="alert" headline={error} />
        ) : info ? (
          <div className="br-ui-doctor-body">
            <TableViewport>
              <Table density="compact">
                <TableBody>
                  {fact(
                    info.engine === 'lightpanda' ? 'Lightpanda' : 'Chromium',
                    info.chromium_version ?? 'unknown',
                  )}
                  {info.chromium_path ? fact('Path', info.chromium_path) : null}
                  {fact('Headless by default', info.headless_default ? 'yes' : 'no')}
                  {fact(
                    'Sessions',
                    `${info.active_sessions ?? 0} of ${info.max_sessions ?? 0}`,
                  )}
                  {info.allowed_schemes
                    ? fact('Allowed schemes', info.allowed_schemes.join(', '))
                    : null}
                </TableBody>
              </Table>
            </TableViewport>
            <div className="br-ui-doctor-caps">
              <span className="br-ui-doctor-cap">
                <StatusDot tone={info.attach_enabled ? 'accent' : 'ink'} />
                Attach {info.attach_enabled ? 'enabled' : 'off'}
              </span>
              <span className="br-ui-doctor-cap">
                <StatusDot tone={info.recording_available ? 'accent' : 'ink'} />
                Recording {info.recording_available ? 'available' : 'off'}
              </span>
            </div>
            {info.issues && info.issues.length > 0 ? (
              <ul className="br-ui-doctor-issues" aria-label="issues">
                {info.issues.map((issue) => (
                  <li key={issue.what}>
                    <span className="br-ui-doctor-issue-what">
                      {issue.what}
                    </span>
                    <span className="br-ui-doctor-issue-how">
                      {issue.enable_how}
                    </span>
                  </li>
                ))}
              </ul>
            ) : (
              <EmptyState
                title="No degraded capabilities"
                description="Everything the worker can offer is available."
              />
            )}
          </div>
        ) : (
          <div className="br-ui-doctor-body" aria-busy aria-label="Reading…">
            <Skeleton className="br-ui-skel" />
            <Skeleton className="br-ui-skel" />
            <Skeleton className="br-ui-skel" />
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}
