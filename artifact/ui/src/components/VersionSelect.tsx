import { Select } from '@iii-dev/console-ui'
import type { ArtifactRecord } from '../lib/api'

/** `v2 (latest)` labels, newest first, with each version's note. */
export function versionOptions(record: ArtifactRecord) {
  return [...record.versions].reverse().map((v) => ({
    value: String(v.version),
    label: `v${v.version}${v.version === record.latest_version ? ' (latest)' : ''}${v.note ? ` · ${v.note}` : ''}`,
  }))
}

/** A visible version picker: the preview follows the chosen version. */
export function VersionSelect({
  record,
  value,
  onChange,
}: {
  record: ArtifactRecord
  value: number
  onChange(version: number): void
}) {
  return (
    <Select
      aria-label="Version"
      className="artifact-ui-version"
      value={String(value)}
      options={versionOptions(record)}
      onChange={(next) => onChange(Number(next))}
      sheetTitle="Version"
    />
  )
}
