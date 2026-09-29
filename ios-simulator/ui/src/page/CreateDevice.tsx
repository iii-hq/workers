import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  Input,
  Select,
  SettingsField,
  SettingsList,
  StatusPanel,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useEffect, useMemo, useState } from 'react'
import type { Runtime, SimApi } from '../lib/api'

/** Pick a runtime and a device type (iPhones first), name it, create. */
export function CreateDeviceDialog({
  api,
  open,
  onOpenChange,
  onCreated,
}: {
  api: SimApi
  open: boolean
  onOpenChange: (open: boolean) => void
  onCreated: (udid: string) => void
}) {
  const [runtimes, setRuntimes] = useState<Runtime[] | null>(null)
  const [runtime, setRuntime] = useState<string>()
  const [deviceType, setDeviceType] = useState<string>()
  const [name, setName] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!open) return
    setError(null)
    let cancelled = false
    api
      .runtimes()
      .then((list) => {
        if (cancelled) return
        const ios = list.filter((r) => r.name.startsWith('iOS'))
        const sorted = [...ios, ...list.filter((r) => !ios.includes(r))]
        setRuntimes(sorted)
        setRuntime((current) => current ?? sorted[0]?.identifier)
      })
      .catch((err) => !cancelled && setError(errorMessage(err)))
    return () => {
      cancelled = true
    }
  }, [api, open])

  const types = useMemo(() => {
    const all = runtimes?.find((r) => r.identifier === runtime)?.device_types ?? []
    return [...all].sort((a, b) => Number(b.product_family === 'iPhone') - Number(a.product_family === 'iPhone'))
  }, [runtimes, runtime])

  useEffect(() => {
    if (!types.some((t) => t.identifier === deviceType)) setDeviceType(types[0]?.identifier)
  }, [types, deviceType])

  const typeName = types.find((t) => t.identifier === deviceType)?.name ?? ''

  const submit = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!deviceType) return
    setBusy(true)
    setError(null)
    try {
      const { device } = await api.create(name.trim() || typeName, deviceType, runtime)
      onOpenChange(false)
      setName('')
      onCreated(device.udid)
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="ios-ui-create">
        <DialogTitle>New simulator</DialogTitle>
        <DialogDescription>
          {api.tenant === 'default' ? 'Created on this Mac.' : `Created in tenant ${api.tenant}'s private device set.`}
        </DialogDescription>
        <form onSubmit={(e) => void submit(e)} className="ios-ui-create-form">
          <SettingsList>
            <SettingsField
              label="Runtime"
              layout="stacked"
              controlSize="full"
              renderControl={(props) => (
                <Select
                  {...props}
                  value={runtime}
                  options={(runtimes ?? []).map((r) => ({ value: r.identifier, label: r.name }))}
                  // The form's hidden select reports '' while options arrive.
                  onChange={(next) => next && setRuntime(next)}
                  placeholder={runtimes ? 'No runtimes installed' : 'Loading…'}
                  aria-label="Runtime"
                />
              )}
            />
            <SettingsField
              label="Device"
              layout="stacked"
              controlSize="full"
              renderControl={(props) => (
                <Select
                  {...props}
                  value={deviceType}
                  groups={['iPhone', 'iPad']
                    .map((family) => ({
                      label: family,
                      options: types
                        .filter((t) => t.product_family === family)
                        .map((t) => ({ value: t.identifier, label: t.name })),
                    }))
                    .filter((g) => g.options.length > 0)}
                  onChange={(next) => next && setDeviceType(next)}
                  aria-label="Device"
                />
              )}
            />
            <SettingsField
              label="Name"
              layout="stacked"
              controlSize="full"
              renderControl={(props) => (
                <Input {...props} value={name} onChange={setName} placeholder={typeName} maxLength={64} />
              )}
            />
          </SettingsList>
          {error ? <StatusPanel variant="alert" role="alert" headline={error} /> : null}
          <div className="ios-ui-create-actions">
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" variant="primary" disabled={busy || !deviceType}>
              {busy ? 'Creating…' : 'Create'}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  )
}
