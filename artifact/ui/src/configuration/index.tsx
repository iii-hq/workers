/**
 * Settings form for the `artifact` configuration entry. Mirrors
 * `src/config.rs` field for field; the host owns dirty tracking, validation
 * and save.
 */

import {
  type ConfigFormProps,
  Input,
  type JsonValue,
  SettingsField,
  SettingsList,
  SettingsSection,
} from '@iii-dev/console-ui'

type JsonObject = { [key: string]: JsonValue }

interface FieldSpec {
  key: string
  label: string
  description: string
  numeric: boolean
}

const SHARING: FieldSpec[] = [
  {
    key: 'listen_addr',
    label: 'Public listener',
    description: 'Loopback ip:port serving public shares only. Point the quick-tunnel target here. Takes effect on restart.',
    numeric: false,
  },
  {
    key: 'local_listen_addr',
    label: 'Local listener',
    description: 'Loopback ip:port serving every share, for previews on this machine. Never tunnel it. Takes effect on restart.',
    numeric: false,
  },
  {
    key: 'tunnel_id',
    label: 'Quick-tunnel target',
    description: 'The operator-configured target public links are published through.',
    numeric: false,
  },
  {
    key: 'default_share_hours',
    label: 'Default lifetime (hours)',
    description: 'Used when a share names no lifetime.',
    numeric: true,
  },
  {
    key: 'max_share_hours',
    label: 'Longest lifetime (hours)',
    description: 'The most a share may ask for, up to 720.',
    numeric: true,
  },
  {
    key: 'tunnel_ready_timeout_ms',
    label: 'Tunnel wait (ms)',
    description: 'How long a share waits for the tunnel and its DNS before answering without a public link.',
    numeric: true,
  },
  {
    key: 'tunnel_grace_minutes',
    label: 'Keep tunnel after last link (minutes)',
    description: 'A new share within this window reuses the same address. 0 closes the tunnel at once.',
    numeric: true,
  },
  {
    key: 'dns_check_resolver',
    label: 'Public DNS for the check',
    description: 'ip:port asked whether a new tunnel address resolves before it is handed out. Empty disables it. Takes effect on restart.',
    numeric: false,
  },
]

const LIMITS: FieldSpec[] = [
  {
    key: 'max_content_bytes',
    label: 'Largest version (bytes)',
    description: 'Content size cap for one version.',
    numeric: true,
  },
  { key: 'max_versions', label: 'Versions per artifact', description: 'Updates past it are refused.', numeric: true },
  { key: 'max_list', label: 'List page size', description: 'Most rows one list returns.', numeric: true },
]

function asObject(value: JsonValue): JsonObject {
  return value && typeof value === 'object' && !Array.isArray(value) ? { ...value } : {}
}

export function ArtifactConfigForm({ value, onChange, errors }: ConfigFormProps) {
  const config = asObject(value)

  const field = (spec: FieldSpec) => (
    <SettingsField
      key={spec.key}
      field={spec.key}
      label={spec.label}
      description={spec.description}
      error={errors?.get(`/${spec.key}`)}
      controlSize={spec.numeric ? 'compact' : 'default'}
      renderControl={(props) => (
        <Input
          {...props}
          inputMode={spec.numeric ? 'numeric' : undefined}
          value={config[spec.key] === undefined || config[spec.key] === null ? '' : String(config[spec.key])}
          onChange={(next) => {
            const trimmed = next.trim()
            const parsed: JsonValue = spec.numeric && /^\d+$/.test(trimmed) ? Number(trimmed) : next
            onChange({ ...config, [spec.key]: parsed })
          }}
        />
      )}
    />
  )

  return (
    <>
      <SettingsSection title="Sharing" description="Where links are served and how long they live.">
        <SettingsList>{SHARING.map(field)}</SettingsList>
      </SettingsSection>
      <SettingsSection title="Limits">
        <SettingsList>{LIMITS.map(field)}</SettingsList>
      </SettingsSection>
    </>
  )
}
