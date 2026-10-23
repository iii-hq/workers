import {
  Button,
  type Host,
  Input,
  SegmentedControl,
  Select,
  StatusPanel,
  uiClasses,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { Box, Globe, LoaderCircle, Monitor } from 'lucide-react'
import { forwardRef, useEffect, useImperativeHandle, useState } from 'react'
import {
  type ComputerDisplay,
  listSandboxImages,
  type SandboxImage,
  type StartSessionInput,
} from '../lib/computer'

/**
 * The stage's start surface while no desktop is open: the three ways to get
 * one as a single choice, the one field that choice needs, and one button.
 * `native` drives this machine (with a display picker when the host reports
 * more than one), `sandbox` boots an image from iii-sandbox's catalog in a
 * microVM, `remote` connects to a desktop somebody else runs. The last choice
 * is remembered per browser.
 */

type Mode = 'native' | 'sandbox' | 'remote'

const MODES = [
  {
    value: 'native' as const,
    label: 'This machine',
    icon: <Monitor size={16} aria-hidden />,
  },
  {
    value: 'sandbox' as const,
    label: 'Sandbox',
    icon: <Box size={16} aria-hidden />,
  },
  {
    value: 'remote' as const,
    label: 'Remote',
    icon: <Globe size={16} aria-hidden />,
  },
]

const HELP: Record<Mode, string> = {
  native:
    'Drives the screen this worker runs on. Clicks and keys reach the real machine.',
  sandbox:
    'A fresh Linux desktop in a microVM. Stopping the session throws it away.',
  remote:
    "Connects to a desktop someone else runs, through its guest executor's WebSocket.",
}

const STORAGE_KEY = 'computer:launcher'

interface Remembered {
  mode: Mode
  image?: string
}

function recall(): Remembered {
  try {
    const saved = JSON.parse(
      localStorage.getItem(STORAGE_KEY) ?? 'null',
    ) as Partial<Remembered> | null
    if (saved && MODES.some((m) => m.value === saved.mode)) {
      return {
        mode: saved.mode as Mode,
        image: typeof saved.image === 'string' ? saved.image : undefined,
      }
    }
  } catch {
    // private window or blocked storage: start from the default
  }
  return { mode: 'native' }
}

function remember(value: Remembered) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(value))
  } catch {
    // not persisted; the choice still applies to this start
  }
}

/** The remembered image when iii-sandbox still lists it, else `desktop`, else the first. */
function pickImage(
  images: readonly SandboxImage[],
  wanted: string | undefined,
): string | undefined {
  const names = images.map((image) => image.name)
  if (wanted && names.includes(wanted)) return wanted
  return names.includes('desktop') ? 'desktop' : names[0]
}

interface LauncherProps {
  host: Host
  displays: ComputerDisplay[]
  starting: boolean
  onStart: (input: StartSessionInput) => void
}

/** A page-level command submits whatever the launcher currently has set. */
export interface LauncherHandle {
  submit(): void
}

export const Launcher = forwardRef<LauncherHandle, LauncherProps>(
  function Launcher({ host, displays, starting, onStart }, ref) {
    const [initial] = useState(recall)
    const [mode, setMode] = useState<Mode>(initial.mode)
    const [image, setImage] = useState<string | undefined>(initial.image)
    const [endpoint, setEndpoint] = useState('')
    const [monitor, setMonitor] = useState<string | undefined>(undefined)
    // null while loading; read each time Sandbox is picked, so an iii-sandbox
    // added meanwhile shows up without a reload.
    const [images, setImages] = useState<SandboxImage[] | null>(null)
    const [imagesError, setImagesError] = useState<string | null>(null)

    useEffect(() => {
      if (mode !== 'sandbox') return
      let cancelled = false
      setImages(null)
      setImagesError(null)
      listSandboxImages(host.iii).then(
        (found) => {
          if (cancelled) return
          setImages(found)
          setImage((current) => pickImage(found, current))
        },
        (error: unknown) => {
          if (cancelled) return
          setImages([])
          setImagesError(errorMessage(error))
        },
      )
      return () => {
        cancelled = true
      }
    }, [host, mode])

    const ready =
      !starting &&
      (mode === 'native' ||
        (mode === 'sandbox' &&
          images?.some((entry) => entry.name === image) === true) ||
        (mode === 'remote' && endpoint.trim() !== ''))

    const submit = () => {
      if (!ready) return
      remember({ mode, image })
      if (mode === 'sandbox' && image) onStart({ image })
      else if (mode === 'remote') onStart({ endpoint: endpoint.trim() })
      else
        onStart(
          monitor != null
            ? { native: true, monitor: Number(monitor) }
            : { native: true },
        )
    }

    useImperativeHandle(ref, () => ({ submit }))

    let field = null
    if (mode === 'native' && displays.length > 1) {
      field = (
        <Select
          value={monitor}
          options={displays.map((d) => ({
            value: String(d.index),
            label: `${d.name || `Display ${d.index}`}${d.primary ? ' (primary)' : ''}`,
            description: `${d.width}x${d.height}`,
          }))}
          onChange={setMonitor}
          allowEmpty
          emptyLabel="Display under the cursor"
          onClear={() => setMonitor(undefined)}
          placeholder="Display under the cursor"
          aria-label="Display"
          className="cp-ui-launch-field"
        />
      )
    } else if (mode === 'sandbox') {
      field =
        images !== null && images.length === 0 ? (
          <StatusPanel
            variant="warn"
            className="cp-ui-launch-panel"
            headline={
              imagesError ? 'Sandboxes need iii-sandbox' : 'No desktop images'
            }
            detail={
              imagesError
                ? "The engine's iii-sandbox worker is not running. Add it under engine.workers in worker-compose.yaml, with a desktop image in custom_images; the computer worker's README has the snippet."
                : 'iii-sandbox lists no desktop image. Add one under custom_images in its configuration, such as ghcr.io/trycua/linux:24.04-slim-20261013-36bc520.'
            }
          />
        ) : (
          <Select
            value={image}
            options={(images ?? []).map((entry) => ({
              value: entry.name,
              label: entry.name,
              description: entry.ociRef || undefined,
            }))}
            // Radix's hidden form select reports '' while the options load.
            onChange={(next) => {
              if (next) setImage(next)
            }}
            placeholder={
              images === null ? 'Looking for desktop images…' : 'Pick an image'
            }
            disabled={images === null}
            aria-busy={images === null}
            aria-label="Sandbox image"
            className="cp-ui-launch-field"
          />
        )
    } else if (mode === 'remote') {
      field = (
        <Input
          value={endpoint}
          onChange={setEndpoint}
          placeholder="ws://host:8000"
          preserveCase
          aria-label="Guest executor endpoint"
          className="cp-ui-launch-field"
        />
      )
    }

    return (
      <div className="cp-ui-launch">
        <Monitor size={28} aria-hidden className="cp-ui-launch-icon" />
        <h2 className="cp-ui-launch-title">Start a desktop</h2>
        <p className="cp-ui-launch-body">
          Desktops you or agents start appear in the session list, live.
        </p>
        <form
          className="cp-ui-launch-form"
          onSubmit={(event) => {
            event.preventDefault()
            submit()
          }}
        >
          <SegmentedControl<Mode>
            value={mode}
            onChange={setMode}
            options={MODES}
            variant="radio"
            aria-label="What to drive"
            className="cp-ui-launch-modes"
          />
          <p className="cp-ui-launch-help">{HELP[mode]}</p>
          {field}
          <Button
            type="submit"
            variant="primary"
            disabled={!ready}
            className="cp-ui-launch-go"
          >
            {starting ? (
              <LoaderCircle size={16} aria-hidden className={uiClasses.spin} />
            ) : null}
            {starting ? 'Starting…' : 'Start desktop'}
          </Button>
          {starting && mode === 'sandbox' ? (
            <p role="status" className="cp-ui-launch-note">
              Booting the microVM. The first boot of an image downloads it,
              which can take a minute or two.
            </p>
          ) : null}
        </form>
      </div>
    )
  },
)
