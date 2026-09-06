/**
 * The tab strip: Chrome's, not the console's line tabs. Tabs sit on the
 * window chrome; the active one is cut from the toolbar's own surface and
 * welded to it with inverted corners. Each tab shows its favicon slot, as
 * much of the page title as fits (fading out, never ellipsized), and a close
 * ✕ that appears on hover and on the active tab. A sleeping tab (page
 * closed, tab kept) dims and swaps the globe for a moon; an incognito tab is
 * dark with the hat-and-glasses glyph. Middle-click closes, arrows step the
 * selection, `+` opens a tab.
 */

import type { KeyboardEvent, MouseEvent } from 'react'
import type { BrowserSessionInfo } from '../lib/browser'
import { cn } from '../lib/cn'
import { Globe, Incognito, Moon, Plus, X } from '../lib/icons'

interface TabStripProps {
  tabs: BrowserSessionInfo[]
  selectedId: string | null
  starting: boolean
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

export function TabStrip({
  tabs,
  selectedId,
  starting,
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
                {tab.incognito ? (
                  <Incognito size={16} />
                ) : asleep ? (
                  <Moon size={14} />
                ) : (
                  <Globe size={15} />
                )}
              </span>
              <span className="br-ui-tab-title">{label}</span>
              <button
                type="button"
                className="br-ui-tab-close"
                aria-label={`close ${label}`}
                tabIndex={active ? 0 : -1}
                onClick={(event) => {
                  event.stopPropagation()
                  onClose(tab.session_id)
                }}
              >
                <X size={13} aria-hidden />
              </button>
            </div>
          )
        })}
        <button
          type="button"
          className="br-ui-tab-new"
          onClick={onNew}
          disabled={starting}
          aria-label={starting ? 'opening a tab' : 'new tab'}
          title="New tab"
        >
          <Plus size={16} aria-hidden />
        </button>
      </div>
    </div>
  )
}
