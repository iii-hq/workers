const SESSION_KEY = 'iii-workspace-mobile-panel'

export type MobilePanelIndexes = Record<string, number>

/** The panel each workspace was showing on a phone, per browser tab. */
export function loadMobilePanelIndexes(): MobilePanelIndexes {
  if (typeof window === 'undefined') return {}
  try {
    const raw = window.sessionStorage.getItem(SESSION_KEY)
    if (!raw) return {}
    const parsed = JSON.parse(raw) as unknown
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed))
      return {}
    const out: MobilePanelIndexes = {}
    for (const [tabId, index] of Object.entries(parsed)) {
      if (typeof index === 'number' && Number.isInteger(index) && index >= 0) {
        out[tabId] = index
      }
    }
    return out
  } catch {
    return {}
  }
}

export function persistMobilePanelIndexes(indexes: MobilePanelIndexes): void {
  if (typeof window === 'undefined') return
  try {
    window.sessionStorage.setItem(SESSION_KEY, JSON.stringify(indexes))
  } catch {
    // best-effort
  }
}
