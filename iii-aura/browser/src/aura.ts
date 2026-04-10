/**
 * iii Aura — browser-side worker.
 *
 * Connects to the iii engine via iii-browser-sdk and registers UI handler
 * functions that the Python inference worker can invoke. All communication
 * flows through iii primitives: trigger, createChannel, and state.
 */

import { registerWorker, TriggerAction } from 'iii-browser-sdk'
import type { ChannelReader, ISdk } from 'iii-browser-sdk'

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

const III_URL = (window as any).__III_URL__ ?? 'ws://localhost:49135'

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

let iii: ISdk
let sessionId: string | null = null
let appState: 'loading' | 'listening' | 'processing' | 'speaking' = 'loading'
let mediaStream: MediaStream | null = null
let audioCtx: AudioContext | null = null
let analyser: AnalyserNode | null = null
let micSource: MediaStreamAudioSourceNode | null = null
let cameraEnabled = true
let ignoreIncomingAudio = false
let speakingStartedAt = 0

// Streaming playback state
let streamSampleRate = 24000
let streamNextTime = 0
let streamSources: AudioBufferSourceNode[] = []

// VAD reference
let myvad: any = null

// DOM refs (populated by init)
let videoEl: HTMLVideoElement
let messagesEl: HTMLDivElement
let statusEl: HTMLSpanElement
let stateDotEl: HTMLDivElement
let stateTextEl: HTMLSpanElement
let cameraToggleEl: HTMLButtonElement
let viewportWrapEl: HTMLDivElement
let waveformCanvas: HTMLCanvasElement
let waveformCtx: CanvasRenderingContext2D

const BAR_COUNT = 40
const BAR_GAP = 3
let ambientPhase = 0
const BARGE_IN_GRACE_MS = 800

// ---------------------------------------------------------------------------
// iii connection + function registration
// ---------------------------------------------------------------------------

async function connectIII() {
  iii = registerWorker(III_URL)

  // Register browser-side functions the backend can invoke
  iii.registerFunction(
    'ui::aura::transcript',
    async (data: { text: string; transcription?: string; llm_time: number }) => {
      // Update the latest user bubble with the real transcription
      if (data.transcription) {
        const userMsgs = messagesEl.querySelectorAll('.msg.user')
        const lastUserMsg = userMsgs[userMsgs.length - 1]
        if (lastUserMsg) {
          const meta = lastUserMsg.querySelector('.meta')
          const metaClone = meta ? meta.cloneNode(true) : null
          lastUserMsg.textContent = data.transcription
          if (metaClone) lastUserMsg.appendChild(metaClone)
        }
      }
      addMessage('assistant', data.text, `LLM ${data.llm_time}s`)
      return null
    },
  )

  iii.registerFunction(
    'ui::aura::playback',
    async (data: { reader: ChannelReader; sample_rate: number; sentence_count: number }) => {
      if (ignoreIncomingAudio) return null

      streamSampleRate = data.sample_rate || 24000
      startStreamPlayback()

      const reader = data.reader

      reader.onBinary((pcmBytes: Uint8Array) => {
        if (ignoreIncomingAudio) return
        const int16 = new Int16Array(pcmBytes.buffer, pcmBytes.byteOffset, pcmBytes.byteLength / 2)
        const float32 = new Float32Array(int16.length)
        for (let i = 0; i < int16.length; i++) float32[i] = int16[i] / 32768
        queueAudioChunk(float32)
      })

      reader.onMessage((msg: string) => {
        try {
          const parsed = JSON.parse(msg)
          if (parsed.type === 'audio_end') {
            if (ignoreIncomingAudio) {
              ignoreIncomingAudio = false
              stopPlayback()
              setState('listening')
              return
            }
            const meta = messagesEl.querySelector('.msg.assistant:last-child .meta')
            if (meta) meta.textContent += ` · TTS ${parsed.tts_time}s`
          }
        } catch { /* ignore */ }
      })

      return null
    },
  )

  iii.onFunctionsAvailable((fns: Array<{ function_id: string }>) => {
    console.log(`iii connected — ${fns.length} functions available`)
    setStatus('connected', 'Connected')
    if (appState === 'loading') openSession()
  })
}

// ---------------------------------------------------------------------------
// Session management
// ---------------------------------------------------------------------------

async function openSession() {
  try {
    const result = await iii.trigger<Record<string, never>, { session_id: string }>({
      function_id: 'aura::session::open',
      payload: {},
    })
    sessionId = result.session_id
    console.log('Session opened', sessionId)
    setState('listening')
  } catch (err) {
    console.error('Failed to open session', err)
    setStatus('disconnected', 'Error')
  }
}

// ---------------------------------------------------------------------------
// Speech turn: capture audio + optional camera frame → channel → trigger
// ---------------------------------------------------------------------------

async function sendTurn(audioSamples: Float32Array) {
  if (!sessionId) return

  setState('processing')
  setStatus('processing', 'Processing')
  const hasImage = cameraEnabled
  addMessage('user', '<span class="loading-dots"><span></span><span></span><span></span></span>', hasImage ? 'with camera' : '', true)

  // Convert float32@16kHz to WAV bytes
  const wavBytes = float32ToWav(audioSamples)

  // Capture camera frame
  let imageB64: string | null = null
  if (hasImage) imageB64 = captureFrame()

  // Create a channel for sending audio+metadata to the backend
  const channel = await iii.createChannel()

  // Send metadata (including optional image) as a text message first
  channel.writer.sendMessage(JSON.stringify({ has_image: hasImage, image: imageB64 }))

  // Send audio binary
  channel.writer.sendBinary(new Uint8Array(wavBytes))
  channel.writer.close()

  // Trigger the ingest function (sync — will return when inference is done)
  try {
    await iii.trigger({
      function_id: 'aura::ingest::turn',
      payload: {
        session_id: sessionId,
        reader: channel.readerRef,
        has_image: hasImage,
      },
    })
  } catch (err) {
    console.error('Ingest failed', err)
    setState('listening')
    setStatus('connected', 'Connected')
  }
}

// ---------------------------------------------------------------------------
// Barge-in / interrupt
// ---------------------------------------------------------------------------

function sendInterrupt() {
  if (!sessionId) return
  iii.trigger({
    function_id: 'aura::interrupt',
    payload: { session_id: sessionId },
    action: TriggerAction.Void(),
  })
}

// ---------------------------------------------------------------------------
// Audio helpers
// ---------------------------------------------------------------------------

function float32ToWav(samples: Float32Array): ArrayBuffer {
  const buf = new ArrayBuffer(44 + samples.length * 2)
  const v = new DataView(buf)
  const w = (o: number, s: string) => { for (let i = 0; i < s.length; i++) v.setUint8(o + i, s.charCodeAt(i)) }
  w(0, 'RIFF'); v.setUint32(4, 36 + samples.length * 2, true); w(8, 'WAVE'); w(12, 'fmt ')
  v.setUint32(16, 16, true); v.setUint16(20, 1, true); v.setUint16(22, 1, true)
  v.setUint32(24, 16000, true); v.setUint32(28, 32000, true); v.setUint16(32, 2, true)
  v.setUint16(34, 16, true); w(36, 'data'); v.setUint32(40, samples.length * 2, true)
  for (let i = 0; i < samples.length; i++) {
    const s = Math.max(-1, Math.min(1, samples[i]))
    v.setInt16(44 + i * 2, s < 0 ? s * 0x8000 : s * 0x7fff, true)
  }
  return buf
}

function captureFrame(): string | null {
  if (!cameraEnabled || !videoEl.videoWidth) return null
  const canvas = document.createElement('canvas')
  const scale = 320 / videoEl.videoWidth
  canvas.width = 320
  canvas.height = videoEl.videoHeight * scale
  canvas.getContext('2d')!.drawImage(videoEl, 0, 0, canvas.width, canvas.height)
  return canvas.toDataURL('image/jpeg', 0.7).split(',')[1]
}

function ensureAudioCtx() {
  if (!audioCtx) {
    audioCtx = new AudioContext()
    analyser = audioCtx.createAnalyser()
    analyser.fftSize = 256
    analyser.smoothingTimeConstant = 0.75
  }
}

function startStreamPlayback() {
  stopPlayback()
  ensureAudioCtx()
  if (audioCtx!.state === 'suspended') audioCtx!.resume()
  streamNextTime = audioCtx!.currentTime + 0.05
  speakingStartedAt = Date.now()
  setState('speaking')
}

function queueAudioChunk(float32: Float32Array) {
  ensureAudioCtx()
  const audioBuffer = audioCtx!.createBuffer(1, float32.length, streamSampleRate)
  audioBuffer.getChannelData(0).set(float32)

  const source = audioCtx!.createBufferSource()
  source.buffer = audioBuffer
  source.connect(audioCtx!.destination)
  source.connect(analyser!)

  const startAt = Math.max(streamNextTime, audioCtx!.currentTime)
  source.start(startAt)
  streamNextTime = startAt + audioBuffer.duration

  streamSources.push(source)
  source.onended = () => {
    const idx = streamSources.indexOf(source)
    if (idx !== -1) streamSources.splice(idx, 1)
    if (streamSources.length === 0 && appState === 'speaking') {
      setState('listening')
      setStatus('connected', 'Connected')
    }
  }
}

function stopPlayback() {
  for (const src of streamSources) { try { src.stop() } catch { /* ignore */ } }
  streamSources = []
  streamNextTime = 0
}

// ---------------------------------------------------------------------------
// Camera
// ---------------------------------------------------------------------------

async function startCamera() {
  try {
    mediaStream = await navigator.mediaDevices.getUserMedia({
      video: { width: 640, height: 480, facingMode: 'user' },
      audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true },
    })
    videoEl.srcObject = mediaStream
    return
  } catch { /* fallback below */ }

  const results = await Promise.allSettled([
    navigator.mediaDevices.getUserMedia({ video: { width: 640, height: 480, facingMode: 'user' } }),
    navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true } }),
  ])
  mediaStream = new MediaStream()
  for (const r of results) {
    if (r.status === 'fulfilled') r.value.getTracks().forEach(t => mediaStream!.addTrack(t))
  }
  if (mediaStream.getVideoTracks().length) videoEl.srcObject = mediaStream
}

// ---------------------------------------------------------------------------
// VAD handlers
// ---------------------------------------------------------------------------

function handleSpeechStart() {
  if (appState === 'speaking') {
    if (Date.now() - speakingStartedAt < BARGE_IN_GRACE_MS) return
    stopPlayback()
    ignoreIncomingAudio = true
    sendInterrupt()
    setState('listening')
    console.log('Barge-in: interrupted playback')
  }
}

function handleSpeechEnd(audio: Float32Array) {
  if (appState !== 'listening') return
  sendTurn(audio)
}

// ---------------------------------------------------------------------------
// UI helpers
// ---------------------------------------------------------------------------

function setState(s: typeof appState) {
  appState = s
  stateDotEl.className = `dot ${s}`
  const labels: Record<string, string> = { loading: 'Loading...', listening: 'Listening', processing: 'Thinking...', speaking: 'Speaking' }
  stateTextEl.textContent = labels[s] || s

  viewportWrapEl.className = `viewport-wrap ${s}`
  if (s !== 'speaking') {
    viewportWrapEl.style.boxShadow = ''
    const glow = viewportWrapEl.querySelector('.viewport-glow') as HTMLElement | null
    if (glow) glow.style.boxShadow = ''
  }

  const stateVars: Record<string, [string, string]> = {
    listening: ['#4ade80', 'rgba(74,222,128,0.12)'],
    processing: ['#f59e0b', 'rgba(245,158,11,0.12)'],
    speaking: ['#818cf8', 'rgba(129,140,248,0.12)'],
    loading: ['#3a3d46', 'rgba(58,61,70,0.12)'],
  }
  const [glow, glowDim] = stateVars[s] || stateVars.loading
  document.documentElement.style.setProperty('--glow', glow)
  document.documentElement.style.setProperty('--glow-dim', glowDim)

  if (s === 'speaking') requestAnimationFrame(updateSpeakingGlow)
  if (myvad) myvad.setOptions({ positiveSpeechThreshold: s === 'speaking' ? 0.92 : 0.5 })

  if (s === 'listening' && mediaStream && audioCtx && analyser) {
    if (!micSource) micSource = audioCtx.createMediaStreamSource(mediaStream)
    try { micSource.connect(analyser) } catch { /* ignore */ }
  } else if (micSource && s !== 'listening') {
    try { micSource.disconnect(analyser!) } catch { /* ignore */ }
  }
}

function setStatus(cls: string, text: string) {
  statusEl.className = `status-pill ${cls}`
  statusEl.textContent = text
}

function addMessage(role: string, text: string, meta?: string, html = false) {
  const div = document.createElement('div')
  div.className = `msg ${role}`
  if (html) {
    div.innerHTML = text
  } else {
    div.textContent = text
  }
  if (meta) {
    const metaEl = document.createElement('div')
    metaEl.className = 'meta'
    metaEl.textContent = meta
    div.appendChild(metaEl)
  }
  messagesEl.appendChild(div)
  messagesEl.scrollTop = messagesEl.scrollHeight
}

// ---------------------------------------------------------------------------
// Waveform + speaking glow
// ---------------------------------------------------------------------------

function initWaveformCanvas() {
  const dpr = window.devicePixelRatio || 1
  const rect = waveformCanvas.getBoundingClientRect()
  waveformCanvas.width = rect.width * dpr
  waveformCanvas.height = rect.height * dpr
  waveformCtx.scale(dpr, dpr)
}

function drawWaveform() {
  const w = waveformCanvas.getBoundingClientRect().width
  const h = waveformCanvas.getBoundingClientRect().height
  waveformCtx.clearRect(0, 0, w, h)

  const barWidth = (w - (BAR_COUNT - 1) * BAR_GAP) / BAR_COUNT
  const color = { listening: '#4ade80', processing: '#f59e0b', speaking: '#818cf8', loading: '#3a3d46' }[appState] || '#3a3d46'
  waveformCtx.fillStyle = color

  let dataArray: Uint8Array<ArrayBuffer> | null = null
  if (analyser) {
    dataArray = new Uint8Array(analyser.frequencyBinCount)
    analyser.getByteFrequencyData(dataArray)
  }

  for (let i = 0; i < BAR_COUNT; i++) {
    let amplitude = 0
    if (dataArray) {
      const binIndex = Math.floor((i / BAR_COUNT) * dataArray.length * 0.6)
      amplitude = dataArray[binIndex] / 255
    }
    if (!dataArray || amplitude < 0.02) {
      ambientPhase += 0.0001
      const drift = Math.sin(ambientPhase * 3 + i * 0.4) * 0.5 + 0.5
      amplitude = 0.03 + drift * 0.04
    }

    const barH = Math.max(2, amplitude * (h - 4))
    const x = i * (barWidth + BAR_GAP)
    const y = (h - barH) / 2
    waveformCtx.globalAlpha = 0.3 + amplitude * 0.7
    waveformCtx.beginPath()
    ;(waveformCtx as any).roundRect(x, y, barWidth, barH, Math.min(barWidth / 2, barH / 2, 3))
    waveformCtx.fill()
  }
  waveformCtx.globalAlpha = 1
  requestAnimationFrame(drawWaveform)
}

function updateSpeakingGlow() {
  if (appState !== 'speaking' || !analyser) return
  const data = new Uint8Array(analyser.frequencyBinCount)
  analyser.getByteFrequencyData(data)
  let sum = 0
  for (let i = 0; i < data.length; i++) sum += data[i]
  const avg = sum / data.length / 255
  const intensity = 0.3 + avg * 0.7
  const spread = 20 + avg * 60
  const inner = 15 + avg * 25
  const glow = viewportWrapEl.querySelector('.viewport-glow') as HTMLElement | null
  if (glow) glow.style.boxShadow = `0 0 ${spread}px ${spread * 0.4}px rgba(129,140,248,${intensity * 0.25})`
  viewportWrapEl.style.boxShadow = `inset 0 0 ${inner}px rgba(129,140,248,${intensity * 0.15}), 0 0 ${inner}px rgba(129,140,248,${intensity * 0.2})`
  requestAnimationFrame(updateSpeakingGlow)
}

// ---------------------------------------------------------------------------
// Init
// ---------------------------------------------------------------------------

export async function init() {
  videoEl = document.getElementById('video') as HTMLVideoElement
  messagesEl = document.getElementById('messages') as HTMLDivElement
  statusEl = document.getElementById('status') as HTMLSpanElement
  stateDotEl = document.getElementById('stateDot') as HTMLDivElement
  stateTextEl = document.getElementById('stateText') as HTMLSpanElement
  cameraToggleEl = document.getElementById('cameraToggle') as HTMLButtonElement
  viewportWrapEl = document.getElementById('viewportWrap') as HTMLDivElement
  waveformCanvas = document.getElementById('waveform') as HTMLCanvasElement
  waveformCtx = waveformCanvas.getContext('2d')!

  initWaveformCanvas()
  window.addEventListener('resize', initWaveformCanvas)

  await startCamera()
  await connectIII()

  // Initialize Silero VAD from CDN
  const vad = (window as any).vad as any
  myvad = await vad.MicVAD.new({
    getStream: async () => new MediaStream(mediaStream!.getAudioTracks()),
    positiveSpeechThreshold: 0.5,
    negativeSpeechThreshold: 0.25,
    redemptionMs: 600,
    minSpeechMs: 300,
    preSpeechPadMs: 300,
    onSpeechStart: handleSpeechStart,
    onSpeechEnd: handleSpeechEnd,
    onVADMisfire: () => console.log('VAD misfire (too short)'),
    onnxWASMBasePath: 'https://cdn.jsdelivr.net/npm/onnxruntime-web@1.22.0/dist/',
    baseAssetPath: 'https://cdn.jsdelivr.net/npm/@ricky0123/vad-web@0.0.29/dist/',
  })
  myvad.start()

  const initAudio = () => {
    ensureAudioCtx()
    if (audioCtx!.state === 'suspended') audioCtx!.resume()
    document.removeEventListener('click', initAudio)
    document.removeEventListener('keydown', initAudio)
  }
  document.addEventListener('click', initAudio)
  document.addEventListener('keydown', initAudio)
  ensureAudioCtx()

  cameraToggleEl.addEventListener('click', () => {
    cameraEnabled = !cameraEnabled
    cameraToggleEl.classList.toggle('active', cameraEnabled)
    cameraToggleEl.textContent = cameraEnabled ? 'Camera On' : 'Camera Off'
    videoEl.style.opacity = cameraEnabled ? '1' : '0.3'
  })

  drawWaveform()
  console.log('Aura initialized')
}
