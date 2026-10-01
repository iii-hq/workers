import { errorMessage } from '@iii-dev/console-ui/format'
import { Plus, Trash2, Undo2, X } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { Badge } from '@/components/ui/Badge'
import { Button } from '@/components/ui/Button'
import { CodeEditor } from '@/components/ui/CodeEditor'
import { FileDiff } from '@/components/ui/FileDiff'
import { IconButton } from '@/components/ui/IconButton'
import { Input } from '@/components/ui/Input'
import { Select } from '@/components/ui/Select'
import {
  SettingsField,
  SettingsList,
  SettingsRow,
  SettingsSection,
} from '@/components/ui/Settings'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { Card, CardBody, CardHeader } from '@/components/ui/Surface'
import type {
  ComposeApi,
  ContainerEntry,
  DeclaredContainer,
} from './compose-api'
import type { Actions } from './index'
import {
  type EnvDraft,
  entryShape,
  entryYaml,
  MASK,
  type SettingsDraft,
  settingsPatch,
  shortPath,
} from './model'

const FILE = 'worker-compose.yaml'
const PREBUILT = /^\.\/target\/(debug|release)\//

export function SettingsTab({
  api,
  actions,
  name,
  declared,
  others,
  entry,
  draft,
  onLoaded,
  onDraft,
  onRemove,
}: {
  api: ComposeApi
  actions: Actions
  name: string
  declared: DeclaredContainer | null
  others: string[]
  entry: ContainerEntry | null
  draft: SettingsDraft | null
  onLoaded: (entry: ContainerEntry) => void
  onDraft: (draft: SettingsDraft) => void
  onRemove: () => void
}) {
  const [error, setError] = useState<string | null>(null)
  const [attempt, setAttempt] = useState(0)

  // Load once, and again after a save; the draft lives in the parent across tabs.
  const loaded = useRef(onLoaded)
  loaded.current = onLoaded
  const hasEntry = entry !== null
  useEffect(() => {
    if (hasEntry && attempt === 0) return
    let cancelled = false
    setError(null)
    api
      .container(name)
      .then((next) => !cancelled && loaded.current(next))
      .catch((cause) => !cancelled && setError(errorMessage(cause)))
    return () => {
      cancelled = true
    }
  }, [api, name, attempt, hasEntry])

  if (error) {
    return (
      <StatusPanel
        variant="alert"
        headline="The declaration could not be read"
        detail={error}
        action={
          <Button
            variant="pill"
            size="sm"
            onClick={() => setAttempt((n) => n + 1)}
          >
            Retry
          </Button>
        }
      />
    )
  }
  if (!entry || !draft) return <Skeleton className="wk-skeleton-block" />

  const { patch, changes } = settingsPatch(entry, draft)
  const set = (next: Partial<SettingsDraft>) => onDraft({ ...draft, ...next })
  const setEnv = (index: number, next: Partial<EnvDraft>) =>
    set({
      env: draft.env.map((row, i) => (i === index ? { ...row, ...next } : row)),
    })
  const cargo = `cargo run --locked --bin ${name}`
  const waitable = others.filter((other) => !draft.startAfter.includes(other))

  const save = async () => {
    if (await actions.track(`Saving ${name}`, () => api.edit(name, patch)))
      setAttempt((n) => n + 1)
  }

  return (
    <div className="wk-settings">
      <div className="wk-form">
        <SettingsSection
          title="Run command"
          description={
            declared?.source === 'path'
              ? `Runs in ${shortPath(declared.ref)}.`
              : 'Empty runs what the worker package declares.'
          }
        >
          <SettingsList>
            <SettingsField
              label="scripts.run"
              layout="stacked"
              controlSize="full"
              renderControl={(control) => (
                <Input
                  {...control}
                  className="wk-mono"
                  value={draft.run}
                  onChange={(run) => set({ run })}
                  spellCheck={false}
                />
              )}
            />
          </SettingsList>
          {PREBUILT.test(draft.run) ? (
            <Button
              variant="ghost"
              size="sm"
              className="wk-suggestion"
              onClick={() => set({ run: cargo })}
            >
              Build on start: <span className="wk-mono">{cargo}</span>
            </Button>
          ) : null}
        </SettingsSection>

        <SettingsSection
          title="Starts after"
          description={`Compose starts ${name} once these are ready, and stops it before them.`}
        >
          <SettingsList>
            {draft.startAfter.map((dep) => (
              <SettingsRow
                key={dep}
                label={<span className="wk-mono">{dep}</span>}
                action={
                  <IconButton
                    label={`Stop waiting for ${dep}`}
                    variant="ghost"
                    onClick={() =>
                      set({
                        startAfter: draft.startAfter.filter((d) => d !== dep),
                      })
                    }
                  >
                    <X />
                  </IconButton>
                }
              />
            ))}
            <SettingsRow
              label={
                draft.startAfter.length
                  ? 'Also wait for'
                  : 'Starts with the engine'
              }
              control={
                <Select
                  aria-label="Wait for a container"
                  placeholder="Wait for a container…"
                  // An action to pick from, not an empty field: ink, not the ghost placeholder grey.
                  className="data-[placeholder]:text-ink"
                  value={undefined}
                  options={waitable.map((other) => ({
                    value: other,
                    label: other,
                  }))}
                  onChange={(dep) =>
                    set({ startAfter: [...draft.startAfter, dep] })
                  }
                  disabled={!waitable.length}
                />
              }
            />
          </SettingsList>
        </SettingsSection>

        <SettingsSection
          title="Environment"
          description={`${entry.env_file.length ? `Also loads ${entry.env_file.join(', ')}. ` : ''}Values that look like credentials are write-only: replace them, they are never shown.`}
          action={
            <Button
              variant="ghost"
              size="sm"
              onClick={() =>
                set({
                  env: [
                    ...draft.env,
                    {
                      key: '',
                      value: '',
                      secret: false,
                      replacing: false,
                      removed: false,
                      isNew: true,
                    },
                  ],
                })
              }
            >
              <Plus />
              Add variable
            </Button>
          }
        >
          {draft.env.length ? (
            <SettingsList>
              {draft.env.map((row, index) => (
                <SettingsRow
                  key={row.isNew ? `new-${index}` : row.key}
                  data-removed={row.removed || undefined}
                  className="wk-env"
                  label={
                    row.isNew ? (
                      <Input
                        aria-label="Variable name"
                        className="wk-mono"
                        placeholder="NAME"
                        value={row.key}
                        onChange={(key) =>
                          setEnv(index, {
                            key: key.toUpperCase().replace(/[^A-Z0-9_]/g, ''),
                          })
                        }
                      />
                    ) : (
                      <span className="wk-mono wk-env-key">{row.key}</span>
                    )
                  }
                  control={
                    row.secret && !row.replacing ? (
                      <span className="wk-secret">
                        <span className="wk-mono wk-faint">{MASK}</span>
                        <Badge>secret</Badge>
                        <Button
                          variant="ghost"
                          size="sm"
                          disabled={row.removed}
                          onClick={() =>
                            setEnv(index, { replacing: true, value: '' })
                          }
                        >
                          Replace
                        </Button>
                      </span>
                    ) : (
                      <Input
                        aria-label={`${row.key || 'New variable'} value`}
                        className="wk-mono wk-value"
                        type={row.secret ? 'password' : 'text'}
                        placeholder={row.secret ? 'New value' : 'value'}
                        value={row.value}
                        disabled={row.removed}
                        onChange={(value) => setEnv(index, { value })}
                        autoComplete="off"
                        spellCheck={false}
                      />
                    )
                  }
                  action={
                    row.removed ? (
                      <IconButton
                        label={`Keep ${row.key}`}
                        variant="ghost"
                        onClick={() => setEnv(index, { removed: false })}
                      >
                        <Undo2 />
                      </IconButton>
                    ) : (
                      <IconButton
                        label={`Remove ${row.key || 'variable'}`}
                        variant="ghost"
                        className="wk-danger"
                        onClick={() =>
                          row.isNew
                            ? set({
                                env: draft.env.filter((_, i) => i !== index),
                              })
                            : setEnv(index, { removed: true })
                        }
                      >
                        <Trash2 />
                      </IconButton>
                    )
                  }
                />
              ))}
            </SettingsList>
          ) : null}
        </SettingsSection>

        <SettingsSection
          title="Config override"
          description="YAML merged over the worker's own configuration."
        >
          <div className="wk-code">
            <CodeEditor
              aria-label="config_override"
              language="yaml"
              value={draft.config}
              onChange={(config) => set({ config })}
              lineNumbers
            />
          </div>
        </SettingsSection>

        <SettingsSection title="Remove from project">
          <SettingsList>
            <SettingsRow
              label={`Remove ${name}`}
              description="Takes the entry and every start_after reference to it out of the compose file. Only this container stops."
              action={
                <Button
                  variant="ghost"
                  size="sm"
                  className="wk-danger"
                  disabled={actions.busy}
                  onClick={onRemove}
                >
                  Remove…
                </Button>
              }
            />
          </SettingsList>
        </SettingsSection>
      </div>

      <Card className="wk-preview">
        <CardHeader className="wk-card-header">
          Change preview
          <Badge variant={changes ? 'accent' : 'default'}>
            {changes
              ? `${changes} change${changes === 1 ? '' : 's'}`
              : 'no changes'}
          </Badge>
        </CardHeader>
        <CardBody className="wk-preview-body">
          {changes ? (
            <>
              <FileDiff
                oldFile={{ name: FILE, contents: entryYaml(entryShape(entry)) }}
                newFile={{
                  name: FILE,
                  contents: entryYaml(entryShape(entry, draft)),
                }}
                disableFileHeader
              />
              <p className="wk-note">
                Saving rewrites this entry and restarts {name}.
              </p>
            </>
          ) : (
            <p className="wk-note">
              Edit a setting to see the change to {FILE} before anything is
              written.
            </p>
          )}
          <div className="wk-buttons">
            <Button
              variant="ghost"
              size="sm"
              disabled={!changes}
              onClick={() => onLoaded(entry)}
            >
              Discard
            </Button>
            <Button
              variant="primary"
              size="sm"
              disabled={!changes || actions.busy}
              onClick={() => void save()}
            >
              Save and restart
            </Button>
          </div>
        </CardBody>
      </Card>
    </div>
  )
}
