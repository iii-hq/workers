import type { ConfigFormProps, Host, JsonValue } from '@iii-dev/console-ui'
import { Input, Select, SettingsField, SettingsList, SettingsRow, SettingsSection } from '@iii-dev/console-ui'
import { useEffect, useState } from 'react'
import { expiresIn } from './model'
import type { Status } from './types'

const DEFAULTS = {
  cloud_url: 'https://cloud.iii.dev',
  iii_bin: 'iii',
  merge_method: 'squash',
  postgres_image: 'pgvector/pgvector:pg17',
  run_timeout_s: 900,
  data_dir: 'data/iii-cloud',
} as const

type Cfg = Record<string, JsonValue>
const asCfg = (v: JsonValue): Cfg => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Cfg) : {})

export function createConfigForm(host: Host) {
  return function IiiCloudConfigForm({ value, onChange, errors }: ConfigFormProps) {
    const cfg = asCfg(value)
    const [status, setStatus] = useState<Status | null>(null)
    useEffect(() => {
      let live = true
      host.iii
        .call<Status>('iii-cloud::status', {})
        .then((s) => live && setStatus(s))
        .catch(() => undefined)
      return () => {
        live = false
      }
    }, [])
    // Preserve unknown siblings; show defaults without writing them.
    const set = (key: string, next: JsonValue | undefined) => {
      const out = { ...cfg }
      if (next === undefined) delete out[key]
      else out[key] = next
      onChange(out)
    }
    const text = (key: keyof typeof DEFAULTS) =>
      typeof cfg[key] === 'string' ? (cfg[key] as string) : String(DEFAULTS[key])
    const c = status?.connection
    const signedIn =
      c?.status === 'connected'
        ? `Signed in as ${c.user?.email ?? 'you'} · ${c.method === 'api_key' ? 'API key' : 'browser'}${c.expires_at ? ` · expires ${expiresIn(c)}` : ''}`
        : 'Not signed in'
    return (
      <>
        <SettingsSection title="iii cloud" description="Where the worker signs in and which iii it runs here.">
          <SettingsList>
            <SettingsField
              id="iii-cloud-url"
              field="cloud_url"
              label="Cloud URL"
              description="The console. A change signs you out: the token belongs to one cloud."
              error={errors?.get('/cloud_url')}
              renderControl={(p) => <Input {...p} value={text('cloud_url')} onChange={(v) => set('cloud_url', v)} />}
            />
            <SettingsField
              id="iii-cloud-bin"
              field="iii_bin"
              label="iii binary"
              description="The iii the run here uses. It must report the cloud's engine version."
              meta={
                status
                  ? `detected ${status.worker.iii_version ?? 'none'} · cloud engine ${status.worker.cloud_engine ?? 'unknown'}`
                  : undefined
              }
              error={errors?.get('/iii_bin')}
              renderControl={(p) => (
                <Input {...p} className="iii-cloud-mono" value={text('iii_bin')} onChange={(v) => set('iii_bin', v)} />
              )}
            />
            <SettingsRow label="Account" description={signedIn} />
          </SettingsList>
        </SettingsSection>
        <SettingsSection title="Deploy and run here">
          <SettingsList>
            <SettingsField
              id="iii-cloud-merge"
              field="merge_method"
              label="Merge method"
              description="How Deploy merges the pull request."
              error={errors?.get('/merge_method')}
              renderControl={(p) => (
                <Select
                  {...p}
                  value={text('merge_method')}
                  options={[
                    { value: 'squash', label: 'Squash' },
                    { value: 'merge', label: 'Merge commit' },
                    { value: 'rebase', label: 'Rebase' },
                  ]}
                  onChange={(v) => set('merge_method', v)}
                />
              )}
            />
            <SettingsField
              id="iii-cloud-pg"
              field="postgres_image"
              label="Postgres image"
              description="The local Postgres for a database resource."
              error={errors?.get('/postgres_image')}
              renderControl={(p) => (
                <Input
                  {...p}
                  className="iii-cloud-mono"
                  value={text('postgres_image')}
                  onChange={(v) => set('postgres_image', v)}
                />
              )}
            />
            <SettingsField
              id="iii-cloud-timeout"
              field="run_timeout_s"
              label="Run timeout"
              description="Seconds for the whole run here, from install to healthcheck."
              error={errors?.get('/run_timeout_s')}
              renderControl={(p) => (
                <Input
                  {...p}
                  inputMode="numeric"
                  value={
                    typeof cfg.run_timeout_s === 'number'
                      ? String(cfg.run_timeout_s)
                      : cfg.run_timeout_s === undefined
                        ? String(DEFAULTS.run_timeout_s)
                        : ''
                  }
                  onChange={(v) =>
                    set('run_timeout_s', v.replace(/\D/g, '') === '' ? undefined : Number(v.replace(/\D/g, '')))
                  }
                />
              )}
            />
            <SettingsField
              id="iii-cloud-data"
              field="data_dir"
              label="Data directory"
              description="Records and run logs, relative to the compose directory."
              error={errors?.get('/data_dir')}
              renderControl={(p) => (
                <Input
                  {...p}
                  className="iii-cloud-mono"
                  value={text('data_dir')}
                  onChange={(v) => set('data_dir', v)}
                />
              )}
            />
          </SettingsList>
        </SettingsSection>
      </>
    )
  }
}
