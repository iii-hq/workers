// Vite's CSS modules import these helpers even when HMR is disabled. Keep its
// real transformed CSS while removing its development WebSocket transport.
const styles = new Map()

export function updateStyle(id, content) {
  let style = styles.get(id)
  if (!style) {
    style = document.createElement('style')
    style.dataset.viteDevId = id
    document.head.append(style)
    styles.set(id, style)
  }
  style.textContent = content
}

export function removeStyle(id) {
  styles.get(id)?.remove()
  styles.delete(id)
}

export function createHotContext() {
  return { data: {}, accept() {}, prune() {}, dispose() {}, on() {}, off() {} }
}

export function injectQuery(url, query) {
  return `${url}${url.includes('?') ? '&' : '?'}${query}`
}

export class ErrorOverlay extends HTMLElement {}
