/**
 * Downloads for the selected session: what a browser's downloads panel
 * shows. Seeded from `browser::downloads::list`, refreshed on the
 * session-filtered `browser::download-changed` event. Each row can go to the
 * chat, save to disk, or be removed.
 */

import { type Host, IconButton } from '@iii-dev/console-ui'
import { useCallback, useEffect, useState } from 'react'
import {
  BROWSER_DOWNLOAD_CHANGED_TRIGGER,
  type BrowserDownload,
  downloadFile,
  errorMessage,
  formatTime,
  listBrowserDownloads,
  readBrowserDownload,
  removeBrowserDownload,
} from '../lib/browser'
import { useBrowserSessionEvent } from '../lib/events'
import { formatSize } from '../lib/format'
import { Download, MessageSquarePlus, X } from '../lib/icons'

const DOWNLOADS_FEED_FN = 'iii::browser-ui::downloads-feed'

interface DownloadsPanelProps {
  host: Host
  sessionId: string
  enabled: boolean
}

export function DownloadsPanel({
  host,
  sessionId,
  enabled,
}: DownloadsPanelProps) {
  const [downloads, setDownloads] = useState<BrowserDownload[]>([])
  const [error, setError] = useState<string | null>(null)
  const refresh = useCallback(() => {
    void listBrowserDownloads(host.iii, sessionId)
      .then(setDownloads)
      .catch((e: unknown) => setError(errorMessage(e)))
  }, [host, sessionId])
  useEffect(() => {
    setError(null)
    refresh()
  }, [refresh])
  useBrowserSessionEvent({
    host,
    enabled,
    triggerType: BROWSER_DOWNLOAD_CHANGED_TRIGGER,
    sessionId,
    fnId: DOWNLOADS_FEED_FN,
    onEvent: () => refresh(),
  })

  const canSendToChat = typeof host.chat?.compose === 'function'
  const sendToChat = useCallback(
    (guid: string) => {
      void readBrowserDownload(host.iii, sessionId, guid).then((result) => {
        if (result) host.chat?.compose?.({ files: [result.file] })
      })
    },
    [host, sessionId],
  )
  const save = useCallback(
    (guid: string) => {
      void readBrowserDownload(host.iii, sessionId, guid).then((result) => {
        if (result) downloadFile(result.file)
      })
    },
    [host, sessionId],
  )
  const remove = useCallback(
    (guid: string) => {
      void removeBrowserDownload(host.iii, sessionId, guid).then(refresh)
    },
    [host, sessionId, refresh],
  )

  if (error) {
    return <p className="br-ui-downloads-empty">downloads failed: {error}</p>
  }
  if (downloads.length === 0) {
    return (
      <p className="br-ui-downloads-empty">
        Nothing downloaded in this session yet.
      </p>
    )
  }
  return (
    <ul className="br-ui-downloads" aria-label="downloads">
      {downloads.map((d) => {
        const done = d.state === 'completed'
        const pct =
          d.total_bytes > 0
            ? Math.round((d.received_bytes / d.total_bytes) * 100)
            : 0
        return (
          <li key={d.guid} className="br-ui-download-row">
            <Download size={16} aria-hidden className="br-ui-download-icon" />
            <div className="br-ui-download-main">
              <span className="br-ui-download-name" title={d.file_name}>
                {d.file_name}
              </span>
              <span className="br-ui-download-meta">
                {d.state === 'in_progress'
                  ? `Downloading… ${pct}%`
                  : d.state === 'canceled'
                    ? 'Canceled'
                    : formatSize(d.received_bytes)}
                <span aria-hidden> · </span>
                {formatTime(Math.floor(d.started_ms / 1000))}
              </span>
              {d.state === 'in_progress' ? (
                <span className="br-ui-download-bar" aria-hidden>
                  <span
                    className="br-ui-download-bar-fill"
                    style={{ width: `${pct}%` }}
                  />
                </span>
              ) : null}
            </div>
            <span className="br-ui-download-actions">
              <IconButton
                label={`send ${d.file_name} to chat`}
                onClick={() => sendToChat(d.guid)}
                disabled={!done || !canSendToChat}
              >
                <MessageSquarePlus size={16} aria-hidden />
              </IconButton>
              <IconButton
                label={`save ${d.file_name}`}
                onClick={() => save(d.guid)}
                disabled={!done}
              >
                <Download size={16} aria-hidden />
              </IconButton>
              <IconButton
                label={`remove ${d.file_name}`}
                onClick={() => remove(d.guid)}
              >
                <X size={16} aria-hidden />
              </IconButton>
            </span>
          </li>
        )
      })}
    </ul>
  )
}

export function useDownloadCount(
  host: Host,
  sessionId: string,
  enabled: boolean,
): number {
  const [count, setCount] = useState(0)
  const refresh = useCallback(() => {
    void listBrowserDownloads(host.iii, sessionId)
      .then((list) => setCount(list.length))
      .catch(() => {})
  }, [host, sessionId])
  useEffect(() => {
    setCount(0)
    refresh()
  }, [refresh])
  useBrowserSessionEvent({
    host,
    enabled,
    triggerType: BROWSER_DOWNLOAD_CHANGED_TRIGGER,
    sessionId,
    fnId: `${DOWNLOADS_FEED_FN}-count`,
    onEvent: () => refresh(),
  })
  return count
}
