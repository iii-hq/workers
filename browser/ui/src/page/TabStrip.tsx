/**
 * The tab strip: Chrome's, not the console's line tabs. Tabs sit on the
 * window chrome; the active one is cut from the toolbar's own surface and
 * welded to it with inverted corners. Each tab shows the page's own favicon
 * (a spinner while a load this pane started is in flight, a globe when the
 * page has none), as much of the page title as fits (fading out, never
 * ellipsized), and a close ✕ that appears on hover and on the active tab. A
 * sleeping tab (page closed, tab kept) dims and keeps its icon, a moon when
 * it has none; an incognito tab is dark with the hat-and-glasses glyph.
 * Middle-click closes, arrows step the selection, `+` opens a tab.
 */

import { IconButton } from '@iii-dev/console-ui'
import uiClasses from '@iii-dev/console-ui/ui-classes'
import { Globe, HatGlasses, LoaderCircle, Moon, Plus, X } from 'lucide-react'
import type { KeyboardEvent, MouseEvent, ReactNode } from 'react'
import { useState } from 'react'
import type { BrowserSessionInfo } from '../lib/browser'
import { cn } from '../lib/cn'

interface TabStripProps {
  tabs: BrowserSessionInfo[]
  selectedId: string | null
  starting: boolean
  /** Tabs with a load in flight (started from this pane). */
  loadingIds?: ReadonlySet<string>
  onSelect: (sessionId: string) => void
  onClose: (sessionId: string) => void
  onNew: () => void
}

function hostOf(url: string): string {
  try {
    return new URL(url).host
  } catch {
    return ''
  }
}

/** What the tab reads: the page title, else its host, else "New tab". */
export function tabLabel(tab: Pick<BrowserSessionInfo, 'title' | 'url'>): string {
  const title = tab.title?.trim()
  if (title) return title
  if (!tab.url || tab.url === 'about:blank') return 'New tab'
  return hostOf(tab.url) || tab.url
}

/** The page's favicon, falling back to `fallback` when it has none or the
 * image fails to load (an icon URL this browser can't reach). */
export function Favicon({
  src,
  fallback,
}: {
  src: string | undefined
  fallback: ReactNode
}) {
  const [failed, setFailed] = useState<string | null>(null)
  if (!src || failed === src) return <>{fallback}</>
  return (
    <img
      src={src}
      alt=""
      width={16}
      height={16}
      draggable={false}
      referrerPolicy="no-referrer"
      className="br-ui-favicon"
      onError={() => setFailed(src)}
    />
  )
}

export function TabStrip({
  tabs,
  selectedId,
  starting,
  loadingIds,
  onSelect,
  onClose,
  onNew,
}: TabStripProps) {
  // Roving selection: arrows move to the neighbouring tab, Delete closes.
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!selectedId) return
    const index = tabs.findIndex((tab) => tab.session_id === selectedId)
    if (index === -1) return
    if (event.key === 'ArrowRight' || event.key === 'ArrowLeft') {
      event.preventDefault()
      const next = tabs[index + (event.key === 'ArrowRight' ? 1 : -1)]
      if (next) {
        onSelect(next.session_id)
        event.currentTarget
          .querySelector<HTMLElement>(`[data-tab-id="${next.session_id}"]`)
          ?.focus()
      }
    } else if (event.key === 'Delete') {
      event.preventDefault()
      onClose(selectedId)
    }
  }

  return (
    <div
      className="br-ui-tabstrip"
      role="tablist"
      aria-label="browser tabs"
      onKeyDown={onKeyDown}
    >
      <div className="br-ui-tabstrip-scroll">
        {tabs.map((tab) => {
          const active = tab.session_id === selectedId
          const asleep = tab.active === false
          const loading = loadingIds?.has(tab.session_id) === true
          const label = tabLabel(tab)
          const hint = [
            label,
            tab.url,
            tab.incognito ? 'Incognito — nothing is saved' : null,
            asleep ? 'Asleep — select to load the page again' : null,
          ]
            .filter(Boolean)
            .join('\n')
          return (
            <div
              key={tab.session_id}
              role="tab"
              tabIndex={active ? 0 : -1}
              aria-selected={active}
              aria-busy={loading || undefined}
              data-tab-id={tab.session_id}
              title={hint}
              className={cn(
                'br-ui-tab',
                active && 'is-active',
                asleep && 'is-asleep',
                tab.incognito && 'is-incognito',
              )}
              onClick={() => onSelect(tab.session_id)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' || event.key === ' ') {
                  event.preventDefault()
                  onSelect(tab.session_id)
                }
              }}
              onMouseDown={(event: MouseEvent<HTMLDivElement>) => {
                // Middle click closes, like every browser.
                if (event.button === 1) {
                  event.preventDefault()
                  onClose(tab.session_id)
                }
              }}
            >
              <span className="br-ui-tab-icon" aria-hidden>
                {loading ? (
                  <LoaderCircle size={16} className={uiClasses.spin} />
                ) : tab.incognito ? (
                  <HatGlasses size={16} />
                ) : (
                  <Favicon
                    src={tab.favicon}
                    fallback={asleep ? <Moon size={16} /> : <Globe size={16} />}
                  />
                )}
              </span>
              <span className="br-ui-tab-title">{label}</span>
              <IconButton
                label={`close ${label}`}
                tooltip={false}
                className="br-ui-tab-close"
                tabIndex={active ? 0 : -1}
                onClick={(event) => {
                  event.stopPropagation()
                  onClose(tab.session_id)
                }}
              >
                <X size={16} aria-hidden />
              </IconButton>
            </div>
          )
        })}
        <IconButton
          label={starting ? 'opening a tab' : 'new tab'}
          tooltip="New tab"
          className="br-ui-tab-new"
          onClick={onNew}
          disabled={starting}
        >
          <Plus size={16} aria-hidden />
        </IconButton>
      </div>
    </div>
  )
}
