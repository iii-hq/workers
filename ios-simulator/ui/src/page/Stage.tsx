import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  EmptyState,
  type Host,
  IconButton,
  LiveRegion,
  type PageRenderProps,
  StatusBar,
  StatusPanel,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  Toolbar,
  useConfirm,
} from '@iii-dev/console-ui'
import { errorMessage, formatDuration } from '@iii-dev/console-ui/format'
import {
  Camera,
  ChevronLeft,
  Circle,
  CircleStop,
  Ellipsis,
  House,
  Lock,
  Power,
  Smartphone,
} from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { type Device, isBooted, runtimeLabel, type SimApi } from '../lib/api'
import { claimOnPage } from '../overlay/store'
import { createInputQueue, useLiveScreen } from './live'
import { MediaPanel } from './Media'
import { Phone } from './Phone'

type Announcement = { seq: number; text: string; urgency: 'polite' | 'assertive' }

export function Stage({
  host,
  api,
  device,
  commands,
  onBack,
  onDeleted,
}: {
  host: Host
  api: SimApi
  device: Device
  commands?: PageRenderProps['commands']
  onBack?: () => void
  onDeleted: () => void
}) {
  const booted = isBooted(device)
  const [tab, setTab] = useState('live')
  const [busy, setBusy] = useState<string | null>(null)
  const [problem, setProblem] = useState<string | null>(null)
  const [announcement, setAnnouncement] = useState<Announcement | null>(null)
  const [recordingSince, setRecordingSince] = useState<number | null>(device.recording ? Date.now() : null)
  const [now, setNow] = useState(Date.now())
  const { confirm, dialog } = useConfirm()
  const watching = booted && tab === 'live'
  const { screen, error: screenError, fps } = useLiveScreen(host, api, device.udid, watching)
  // This window already shows the phone: the live preview leaves it alone.
  useEffect(
    () => (watching ? claimOnPage(`${api.tenant}/${device.udid}`) : undefined),
    [watching, api.tenant, device.udid],
  )
  const input = useMemo(
    () => (booted ? createInputQueue(api, device.udid, setProblem) : null),
    [api, device.udid, booted],
  )
  const seq = useRef(0)
  const announce = (text: string, urgency: Announcement['urgency'] = 'polite') =>
    setAnnouncement({ seq: ++seq.current, text, urgency })

  // Recording state can change elsewhere (an agent, another tab).
  useEffect(() => {
    if (!device.recording) setRecordingSince(null)
    else setRecordingSince((since) => since ?? Date.now())
  }, [device.recording])
  useEffect(() => {
    if (recordingSince === null) return
    const t = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(t)
  }, [recordingSince])

  const act = async (label: string, run: () => Promise<unknown>, done?: string) => {
    setBusy(label)
    setProblem(null)
    try {
      await run()
      if (done) announce(done)
    } catch (err) {
      setProblem(errorMessage(err))
      announce(`${label} failed`, 'assertive')
    } finally {
      setBusy(null)
    }
  }

  const press = (button: string) => input?.run(() => api.button(device.udid, button))
  const remove = async () => {
    const ok = await confirm({
      title: `Delete ${device.name}?`,
      description: 'The simulator, its apps and data, and its screenshots and recordings are removed for good.',
      tone: 'danger',
      confirmLabel: 'Delete',
    })
    if (ok) await act('Delete', () => api.remove(device.udid), `${device.name} deleted`).then(onDeleted)
  }

  const recording = recordingSince !== null
  const screenshot = () => void act('Screenshot', () => api.screenshot(device.udid), 'Screenshot saved')
  const toggleRecording = () =>
    recording
      ? void act('Stop recording', () => api.recordStop(device.udid), 'Recording saved').then(() =>
          setRecordingSince(null),
        )
      : void act('Record', () => api.recordStart(device.udid), 'Recording').then(() => setRecordingSince(Date.now()))

  // Latest handlers for the palette rows, registered once per device.
  const live = useRef({ booted, busy, recording, screenshot, toggleRecording, press })
  live.current = { booted, busy, recording, screenshot, toggleRecording, press }
  useEffect(() => {
    const ready = () => live.current.booted && live.current.busy === null
    return commands?.register([
      { id: 'screenshot', title: 'Take screenshot', enabled: ready, run: () => live.current.screenshot() },
      {
        id: 'record',
        title: 'Start or stop recording',
        keywords: ['video', 'capture'],
        enabled: ready,
        run: () => live.current.toggleRecording(),
      },
      { id: 'home', title: 'Press Home', enabled: () => live.current.booted, run: () => live.current.press('home') },
    ])
  }, [commands])

  return (
    <div className="ios-ui-stage">
      <Toolbar
        aria-label="Simulator controls"
        className="ios-ui-toolbar"
        end={
          <>
            {booted ? (
              <>
                <IconButton label="Home" onClick={() => press('home')}>
                  <House size={16} aria-hidden />
                </IconButton>
                <IconButton label="Lock" onClick={() => press('lock')}>
                  <Lock size={16} aria-hidden />
                </IconButton>
                <IconButton label="Screenshot" disabled={busy !== null} onClick={screenshot}>
                  <Camera size={16} aria-hidden />
                </IconButton>
                <IconButton
                  label={recording ? 'Stop recording' : 'Record screen'}
                  aria-pressed={recording}
                  disabled={busy !== null}
                  data-recording={recording ? 'true' : undefined}
                  className="ios-ui-record"
                  onClick={toggleRecording}
                >
                  {recording ? <CircleStop size={16} aria-hidden /> : <Circle size={16} aria-hidden />}
                </IconButton>
              </>
            ) : null}
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <IconButton label="More actions">
                  <Ellipsis size={16} aria-hidden />
                </IconButton>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                {booted ? (
                  <DropdownMenuItem
                    onSelect={() => void act('Shut down', () => api.shutdown(device.udid), `${device.name} shut down`)}
                  >
                    Shut down
                  </DropdownMenuItem>
                ) : (
                  <DropdownMenuItem
                    onSelect={() => void act('Erase', () => api.erase(device.udid), `${device.name} erased`)}
                  >
                    Erase content and settings
                  </DropdownMenuItem>
                )}
                <DropdownMenuSeparator />
                <DropdownMenuItem onSelect={() => void remove()}>
                  Delete simulator
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </>
        }
      >
        {onBack ? (
          <IconButton label="Back to simulators" onClick={onBack}>
            <ChevronLeft size={16} aria-hidden />
          </IconButton>
        ) : null}
        <span className="ios-ui-title" title={device.udid}>
          {device.name}
        </span>
        <span className="ios-ui-crumb">{runtimeLabel(device.runtime)}</span>
      </Toolbar>

      {problem ? (
        <StatusPanel
          variant="alert"
          role="alert"
          className="ios-ui-problem"
          headline={problem}
          action={
            <Button variant="ghost" size="sm" onClick={() => setProblem(null)}>
              Dismiss
            </Button>
          }
        />
      ) : null}

      <Tabs value={tab} onValueChange={setTab} className="ios-ui-tabs">
        <TabsList variant="line" className="ios-ui-tablist">
          <TabsTrigger value="live" icon={<Smartphone size={16} aria-hidden />}>
            Live
          </TabsTrigger>
          <TabsTrigger value="media" icon={<Camera size={16} aria-hidden />}>
            Media
          </TabsTrigger>
        </TabsList>
        <TabsContent value="live" className="ios-ui-tab">
          {booted ? (
            <Phone
              screen={screen}
              placeholder={screenError ? `Live view unavailable: ${screenError}` : 'Connecting…'}
              input={input}
              onButton={(button, phase) => input?.run(() => api.button(device.udid, button, phase))}
              onText={(text) => input?.run(() => api.type(device.udid, text))}
              onKeys={(keys) => input?.run(() => api.key(device.udid, keys))}
            />
          ) : (
            <EmptyState
              icon={Power}
              title={device.state === 'Shutdown' ? `${device.name} is shut down` : `${device.name} is ${device.state}`}
              description="Boot it to watch and drive it here. It runs headless: no Simulator.app window."
              action={
                device.state === 'Shutdown'
                  ? {
                      label: busy === 'Boot' ? 'Booting…' : 'Boot',
                      onClick: () => void act('Boot', () => api.boot(device.udid), `${device.name} booted`),
                    }
                  : undefined
              }
            />
          )}
        </TabsContent>
        <TabsContent value="media" className="ios-ui-tab">
          <MediaPanel host={host} api={api} udid={device.udid} onError={setProblem} />
        </TabsContent>
      </Tabs>

      <StatusBar
        as="footer"
        className="ios-ui-status"
        end={booted ? <span>Option-drag pinches · Shift+Esc leaves the screen</span> : null}
      >
        {screen ? (
          <span className="ios-ui-fact">
            {screen.deviceWidth}×{screen.deviceHeight}
          </span>
        ) : null}
        {booted && tab === 'live' ? <span className="ios-ui-fact">{fps} fps</span> : null}
        {recording ? (
          <span className="ios-ui-fact" data-tone="alert">
            REC {formatDuration(Math.max(0, now - (recordingSince ?? now)))}
          </span>
        ) : null}
      </StatusBar>
      <LiveRegion announcement={announcement} />
      {dialog}
    </div>
  )
}
