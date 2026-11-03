import { Component, type RefObject } from 'react'
import { REDUCED_MOTION_QUERY } from '@/hooks/use-media-query'

/**
 * The first send of a new chat: the welcome hero fades up and away while the
 * same mounted composer glides from the centre of the pane to its dock.
 *
 * Both halves need the layout from *before* React commits the change — the
 * hero unmounts in that commit, and the composer has already moved by the
 * time any effect runs — so this is a class for `getSnapshotBeforeUpdate`,
 * which React calls ahead of every DOM mutation in the commit. The
 * optimistic user row lands after an await in `submit`, so a View
 * Transition wrapped around the send would miss it.
 *
 * Only the send itself plays it: switching to another chat, or a welcome
 * that ends for any other reason, snaps as before.
 */

interface WelcomeHandoffProps {
  /** `.chat-conversation-body`: holds the hero and the composer footer. */
  bodyRef: RefObject<HTMLElement | null>
  welcome: boolean
  /** A turn has begun (a user row exists, or the agent is thinking). */
  sent: boolean
  conversationId: string
}

interface Snapshot {
  footer: HTMLElement
  footerTop: number
  hero: HTMLElement | null
  heroRect: DOMRect | null
}

// Strong ease-out: the composer leaves at once and settles into the dock.
const GLIDE_EASE = 'cubic-bezier(0.32, 0.72, 0, 1)'
const GLIDE_MS = 320
const EXIT_EASE = 'ease-out'
const EXIT_MS = 150

export class WelcomeHandoff extends Component<WelcomeHandoffProps> {
  getSnapshotBeforeUpdate(prev: WelcomeHandoffProps): Snapshot | null {
    const { bodyRef, welcome, sent, conversationId } = this.props
    if (!prev.welcome || welcome || !sent) return null
    if (prev.conversationId !== conversationId) return null
    const body = bodyRef.current
    const footer = body?.querySelector<HTMLElement>(':scope > footer')
    if (!body || !footer || typeof footer.animate !== 'function') return null
    const hero = body.querySelector<HTMLElement>('.chat-welcome-heading')
    return {
      footer,
      footerTop: footer.getBoundingClientRect().top,
      hero: hero ? (hero.cloneNode(true) as HTMLElement) : null,
      heroRect: hero?.getBoundingClientRect() ?? null,
    }
  }

  componentDidUpdate(
    _prev: WelcomeHandoffProps,
    _state: unknown,
    snapshot: Snapshot | null,
  ) {
    const body = this.props.bodyRef.current
    if (!snapshot || !body) return
    const reduced = window.matchMedia?.(REDUCED_MOTION_QUERY).matches === true

    const { footer, footerTop, hero, heroRect } = snapshot
    // Reduced motion keeps the cross-fade below and drops the travel.
    const dy = footerTop - footer.getBoundingClientRect().top
    if (!reduced && Math.abs(dy) > 1) {
      footer.animate(
        [{ transform: `translateY(${dy}px)` }, { transform: 'none' }],
        { duration: GLIDE_MS, easing: GLIDE_EASE },
      )
    }

    if (hero && heroRect) {
      // A static ghost of the hero, placed where it was: inside the body so
      // the pane's container queries still size its type the same.
      const bodyRect = body.getBoundingClientRect()
      hero.setAttribute('aria-hidden', 'true')
      hero.inert = true
      Object.assign(hero.style, {
        position: 'absolute',
        left: `${heroRect.left - bodyRect.left}px`,
        top: `${heroRect.top - bodyRect.top}px`,
        width: `${heroRect.width}px`,
        height: `${heroRect.height}px`,
        overflow: 'hidden',
        pointerEvents: 'none',
        zIndex: '0',
      })
      const restorePosition = body.style.position
      if (getComputedStyle(body).position === 'static') {
        body.style.position = 'relative'
      }
      body.appendChild(hero)
      const exit = hero.animate(
        reduced
          ? [{ opacity: 1 }, { opacity: 0 }]
          : [
              { opacity: 1, filter: 'blur(0)', transform: 'none' },
              {
                opacity: 0,
                filter: 'blur(4px)',
                transform: 'translateY(-12px)',
              },
            ],
        { duration: EXIT_MS, easing: EXIT_EASE, fill: 'forwards' },
      )
      const remove = () => {
        hero.remove()
        body.style.position = restorePosition
      }
      exit.addEventListener('finish', remove)
      exit.addEventListener('cancel', remove)
    }
  }

  render() {
    return null
  }
}
