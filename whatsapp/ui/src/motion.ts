import { useGSAP } from '@gsap/react'
import gsap from 'gsap'
import { ScrollTrigger } from 'gsap/ScrollTrigger'
import type { RefObject } from 'react'

gsap.registerPlugin(useGSAP, ScrollTrigger)

// Fontshare hosts the original font; optional loading never delays the UI.
let displayFont: Promise<FontFace> | undefined
export function loadDisplayFont() {
  if (typeof FontFace === 'undefined' || displayFont) return
  const face = new FontFace('Cabinet Grotesk', 'url(https://cdn.fontshare.com/wf/J2PZYZURCR7HNQKXCZ4VXYA3K5FFCCLT/PZSZLWHMBCE7FFGOUYDEU33FAESUMA3X/JFDL5FBAQ2WMYL3LGKSCZKAIFCS2UQ63.woff2)', { weight: '100 900', style: 'normal', display: 'swap' })
  document.fonts.add(face)
  displayFont = face.load()
  void displayFont.catch(() => {
    // An offline console retains its local system font and all functionality.
    document.fonts.delete(face)
  })
}

/** Scoped setup motion. Polling and message content never become animation triggers. */
export function useSetupMotion(ref: RefObject<HTMLElement | null>, identity = '') {
  useGSAP(() => {
    const root = ref.current
    if (!root) return
    let parent = root.parentElement
    while (parent && !/(auto|scroll)/.test(getComputedStyle(parent).overflowY)) parent = parent.parentElement
    const scroller = parent ?? undefined
    const media = gsap.matchMedia()
    media.add('(prefers-reduced-motion: no-preference)', () => {
      const triggers: ScrollTrigger[] = []
      root.querySelectorAll<HTMLElement>('.wa-ui-reveal-text').forEach((text) => {
        const animation = gsap.fromTo(text, { opacity: 0.55 }, {
          opacity: 1, ease: 'none',
          scrollTrigger: { trigger: text, scroller, start: 'top bottom', end: 'top 80%', scrub: true },
        })
        if (animation.scrollTrigger) triggers.push(animation.scrollTrigger)
      })
      root.querySelectorAll<HTMLElement>('.wa-ui-stack-card').forEach((card) => {
        const animation = gsap.fromTo(card, { y: 16, scale: 0.985, transformOrigin: 'center bottom' }, {
          y: 0, scale: 1, ease: 'none',
          scrollTrigger: { trigger: card, scroller, start: 'top bottom', end: 'top 85%', scrub: true },
        })
        if (animation.scrollTrigger) triggers.push(animation.scrollTrigger)
      })
      let frame = 0
      let active = true
      const refresh = () => {
        if (!active) return
        cancelAnimationFrame(frame)
        frame = requestAnimationFrame(() => triggers.forEach((trigger) => trigger.refresh()))
      }
      const observer = new ResizeObserver(refresh)
      observer.observe(root)
      if (scroller) observer.observe(scroller)
      void document.fonts.ready.then(refresh)
      return () => { active = false; observer.disconnect(); cancelAnimationFrame(frame) }
    })
    return () => media.revert()
  }, { scope: ref, dependencies: [identity], revertOnUpdate: true })
}
