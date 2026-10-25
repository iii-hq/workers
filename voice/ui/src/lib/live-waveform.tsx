/**
 * The scrolling waveform the dictation bar draws while you speak: one thin
 * bar per update, newest on the right, older ones sliding left and fading
 * at the edges; silence is a row of dots. Adapted from the `LiveWaveform`
 * component of ElevenLabs UI (https://github.com/elevenlabs/ui, MIT,
 * Copyright (c) 2025 Eleven Labs Inc.), scrolling mode only. Unlike the
 * original it opens no microphone of its own: it reads the analyser the
 * dictation capture already taps (`getAnalyser`), so there is one stream
 * and one permission prompt. `processing` swaps the live bars for a slow
 * wave while the transcript is being finished.
 */

import { useEffect, useRef } from 'react'

export interface LiveWaveformProps {
  /** Sample the microphone and scroll. */
  active: boolean
  /** Ease the bars into a slow wave: listening ended and the text is on its way. */
  processing?: boolean
  /** The analyser to sample; null until the microphone is open, which draws silence. */
  getAnalyser: () => AnalyserNode | null
  barWidth?: number
  barGap?: number
  barRadius?: number
  /** The shortest bar, drawn for silence; equal to `barWidth` it is a dot. */
  barHeight?: number
  fadeEdges?: boolean
  fadeWidth?: number
  sensitivity?: number
  /** Milliseconds between samples, so also the scroll speed: one bar step per update. */
  updateRate?: number
  className?: string
}

/** Loudness of the speech band (bins 5% to 40% of the spectrum), 0 to 1. */
export function analyserLevel(analyser: AnalyserNode, sensitivity: number, data: Uint8Array<ArrayBuffer>): number {
  analyser.getByteFrequencyData(data)
  const start = Math.floor(data.length * 0.05)
  const end = Math.floor(data.length * 0.4)
  if (end <= start) return 0
  let sum = 0
  for (let i = start; i < end; i++) sum += data[i]
  return Math.min(1, (sum / (end - start) / 255) * sensitivity)
}

export function LiveWaveform({
  active,
  processing = false,
  getAnalyser,
  barWidth = 2,
  barGap = 2,
  barRadius = 1,
  barHeight: baseBarHeight = 2,
  fadeEdges = true,
  fadeWidth = 48,
  sensitivity = 1,
  updateRate = 50,
  className,
}: LiveWaveformProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const historyRef = useRef<number[]>([])
  const lastActiveDataRef = useRef<number[]>([])
  const sizeRef = useRef({ width: 0, height: 0 })
  const gradientRef = useRef<CanvasGradient | null>(null)
  const drawRef = useRef<() => void>(() => {})
  const getAnalyserRef = useRef(getAnalyser)
  getAnalyserRef.current = getAnalyser

  const step = barWidth + barGap
  const barCount = () => Math.max(0, Math.floor(sizeRef.current.width / step))

  drawRef.current = () => {
    const canvas = canvasRef.current
    const ctx = canvas?.getContext('2d')
    if (!canvas || !ctx) return
    const { width, height } = sizeRef.current
    ctx.clearRect(0, 0, width, height)
    ctx.fillStyle = getComputedStyle(canvas).color || '#888'
    const centerY = height / 2
    const history = historyRef.current
    const count = barCount()
    for (let i = 0; i < count && i < history.length; i++) {
      const value = history[history.length - 1 - i] ?? 0
      const x = width - (i + 1) * step
      const h = Math.max(baseBarHeight, value * height * 0.8)
      ctx.globalAlpha = 0.4 + value * 0.6
      ctx.beginPath()
      ctx.roundRect(x, centerY - h / 2, barWidth, h, barRadius)
      ctx.fill()
    }
    ctx.globalAlpha = 1
    if (fadeEdges && fadeWidth > 0 && width > 0) {
      if (!gradientRef.current) {
        const gradient = ctx.createLinearGradient(0, 0, width, 0)
        const fade = Math.min(0.3, fadeWidth / width)
        gradient.addColorStop(0, 'rgba(255,255,255,1)')
        gradient.addColorStop(fade, 'rgba(255,255,255,0)')
        gradient.addColorStop(1, 'rgba(255,255,255,0)')
        gradientRef.current = gradient
      }
      // destination-out erases where the gradient is opaque: the oldest bars dissolve.
      ctx.globalCompositeOperation = 'destination-out'
      ctx.fillStyle = gradientRef.current
      ctx.fillRect(0, 0, width, height)
      ctx.globalCompositeOperation = 'source-over'
    }
  }

  useEffect(() => {
    const canvas = canvasRef.current
    const container = containerRef.current
    if (!canvas || !container) return
    const observer = new ResizeObserver(() => {
      const rect = container.getBoundingClientRect()
      const dpr = window.devicePixelRatio || 1
      canvas.width = rect.width * dpr
      canvas.height = rect.height * dpr
      canvas.getContext('2d')?.scale(dpr, dpr)
      sizeRef.current = { width: rect.width, height: rect.height }
      gradientRef.current = null
      drawRef.current()
    })
    observer.observe(container)
    return () => observer.disconnect()
  }, [])

  // Listening: sample, push, scroll. The row starts full of dots so the
  // line spans the bar from the first frame and speech scrolls in from the right.
  useEffect(() => {
    if (!active) return
    historyRef.current = new Array(barCount() + 1).fill(0)
    let data: Uint8Array<ArrayBuffer> | null = null
    let last = 0
    let raf = 0
    const tick = (now: number) => {
      raf = requestAnimationFrame(tick)
      if (now - last < updateRate) return
      last = now
      const analyser = getAnalyserRef.current()
      let value = 0
      if (analyser) {
        if (!data || data.length !== analyser.frequencyBinCount) data = new Uint8Array(analyser.frequencyBinCount)
        value = analyserLevel(analyser, sensitivity, data)
      }
      const history = historyRef.current
      history.push(value)
      const max = barCount() + 1
      if (history.length > max) history.splice(0, history.length - max)
      lastActiveDataRef.current = history.slice()
      drawRef.current()
    }
    raf = requestAnimationFrame(tick)
    return () => cancelAnimationFrame(raf)
  }, [active, updateRate, sensitivity, step])

  // Finishing: the last live bars ease into a travelling wave.
  useEffect(() => {
    if (!processing || active) return
    let time = 0
    let progress = 0
    let raf = 0
    const animate = () => {
      time += 0.03
      progress = Math.min(1, progress + 0.02)
      const count = barCount()
      const last = lastActiveDataRef.current
      const next: number[] = []
      for (let i = 0; i < count; i++) {
        const position = (i - count / 2) / (count / 2)
        const centerWeight = 1 - Math.abs(position) * 0.4
        const wave =
          Math.sin(time * 1.5 + i * 0.15) * 0.25 + Math.sin(time * 0.8 - i * 0.1) * 0.2 + Math.cos(time * 2 + i * 0.05) * 0.15
        let value = (0.2 + wave) * centerWeight
        if (last.length > 0 && progress < 1) {
          const prior = last[Math.floor((i / count) * last.length)] ?? 0
          value = prior * (1 - progress) + value * progress
        }
        next.push(Math.max(0.05, Math.min(1, value)))
      }
      historyRef.current = next
      drawRef.current()
      raf = requestAnimationFrame(animate)
    }
    raf = requestAnimationFrame(animate)
    return () => cancelAnimationFrame(raf)
  }, [processing, active, step])

  return (
    <div ref={containerRef} className={className} role="img" aria-label={processing ? 'Finishing the transcript' : 'Microphone waveform'}>
      <canvas ref={canvasRef} aria-hidden="true" />
    </div>
  )
}
