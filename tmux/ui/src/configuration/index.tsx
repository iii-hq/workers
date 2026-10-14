/**
 * The `tmux` configuration form, rendered in global Settings and from the
 * page header's settings action. The host owns load, dirty state, save and
 * reset; this form only proposes the next draft.
 */

import {
  Button,
  type ConfigFormProps,
  IconButton,
  Input,
  type JsonValue,
  List,
  ListItem,
  Panel,
  SettingsDeck,
  SettingsField,
  SettingsList,
  SettingsRow,
  SettingsSection,
  StatusPanel,
} from '@iii-dev/console-ui'
import { formatDuration } from '@iii-dev/console-ui/format'
import { Bot, Plus, Trash2 } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import {
  type AgentModel,
  agentIdError,
  DEFAULT_TMUX_BINARY,
  NUMBER_FIELDS,
  type NumberField,
  numberError,
  numberValue,
  parseProcesses,
  parseWholeNumber,
  type PatternList,
  patternError,
  readTmuxConfiguration,
  type TextField,
  textValue,
  withAgent,
  withDefaultAgents,
  withNewAgent,
  withNumber,
  withoutAgent,
  withText,
} from './model'

const NUMBER_DESCRIPTIONS: Record<NumberField, string> = {
  poll_interval_ms: 'How often sessions, windows and agent states are re-read while a page is open.',
  ports_interval_ms: 'How often processes and listening ports are scanned while a page is open.',
  busy_window_ms: 'An agent without working patterns counts as working while its pane changed within this window.',
  command_timeout_ms: 'How long one tmux command may take before it is abandoned.',
  max_terminals: 'Console terminals (tmux clients) the worker runs at once.',
  terminal_idle_timeout_ms: 'A console terminal not heard from for this long is detached.',
}

function echo(key: NumberField, value: number | null): string {
  const spec = NUMBER_FIELDS[key]
  const shown = value ?? spec.default
  const text = spec.unit === 'ms' ? formatDuration(shown) : String(shown)
  return value === null ? `Default · ${text}` : text
}

/** A whole-number field that commits only parseable values. */
function NumberSetting({
  field,
  value,
  error,
  onChange,
}: {
  field: NumberField
  value: number | null
  error?: string
  onChange(next: number | null): void
}) {
  const [text, setText] = useState(value === null ? '' : String(value))
  const committed = useRef(value)
  useEffect(() => {
    if (value !== committed.current) {
      committed.current = value
      setText(value === null ? '' : String(value))
    }
  }, [value])
  const parsed = parseWholeNumber(text)
  const localError = parsed === undefined ? 'Enter a whole number.' : numberError(field, parsed ?? null)
  const spec = NUMBER_FIELDS[field]
  return (
    <SettingsField
      id={`tmux-cfg-${field}`}
      field={field}
      data-path={field}
      label={spec.label}
      description={NUMBER_DESCRIPTIONS[field]}
      meta={echo(field, value)}
      error={localError ?? error}
      controlSize="compact"
      renderControl={(controlProps) => (
        <Input
          {...controlProps}
          className="tmux-mono"
          inputMode="numeric"
          value={text}
          placeholder={String(spec.default)}
          onChange={(next) => {
            setText(next)
            const result = parseWholeNumber(next)
            if (result === undefined) return
            committed.current = result
            onChange(result)
          }}
        />
      )}
    />
  )
}

function TextSetting({
  field,
  label,
  description,
  placeholder,
  value,
  error,
  onChange,
}: {
  field: TextField
  label: string
  description: string
  placeholder: string
  value: string
  error?: string
  onChange(next: string): void
}) {
  return (
    <SettingsField
      id={`tmux-cfg-${field}`}
      field={field}
      data-path={field}
      label={label}
      description={description}
      error={error}
      renderControl={(controlProps) => (
        <Input
          {...controlProps}
          className="tmux-mono"
          value={value}
          placeholder={placeholder}
          autoComplete="off"
          spellCheck={false}
          onChange={onChange}
        />
      )}
    />
  )
}

function PatternEditor({
  agentIndex,
  list,
  label,
  description,
  patterns,
  onChange,
}: {
  agentIndex: number
  list: PatternList
  label: string
  description: string
  patterns: string[]
  onChange(next: string[]): void
}) {
  return (
    <SettingsSection
      title={label}
      description={description}
      action={
        <Button size="sm" variant="ghost" onClick={() => onChange([...patterns, ''])}>
          <Plus aria-hidden />
          Add pattern
        </Button>
      }
    >
      <SettingsList>
        {patterns.length === 0 ? (
          <SettingsRow label="No patterns" description="Nothing in the pane decides this state." />
        ) : (
          patterns.map((pattern, index) => (
            <SettingsField
              // biome-ignore lint/suspicious/noArrayIndexKey: patterns are positional and may repeat while edited.
              key={index}
              id={`tmux-cfg-agent-${agentIndex}-${list}-${index}`}
              field={`agents.${agentIndex}.${list}.${index}`}
              data-path={`agents.${agentIndex}.${list}.${index}`}
              label={`Pattern ${index + 1}`}
              error={patternError(pattern) ?? undefined}
              layout="stacked"
              controlSize="full"
              renderControl={(controlProps) => (
                <div className="tmux-pattern-row">
                  <Input
                    {...controlProps}
                    className="tmux-mono"
                    value={pattern}
                    placeholder="esc to interrupt"
                    spellCheck={false}
                    onChange={(next) => onChange(patterns.map((p, i) => (i === index ? next : p)))}
                  />
                  <IconButton
                    label={`Remove pattern ${index + 1}`}
                    variant="ghost"
                    onClick={() => onChange(patterns.filter((_, i) => i !== index))}
                  >
                    <Trash2 aria-hidden />
                  </IconButton>
                </div>
              )}
            />
          ))
        )}
      </SettingsList>
    </SettingsSection>
  )
}

function AgentDetail({
  agents,
  index,
  onPatch,
  onRemove,
}: {
  agents: AgentModel[]
  index: number
  onPatch(patch: Partial<Omit<AgentModel, 'raw'>>): void
  onRemove(): void
}) {
  const agent = agents[index]
  const [processText, setProcessText] = useState(agent.processes.join(', '))
  return (
    <div className="tmux-config-agent">
      <SettingsSection title="Identity">
        <SettingsList>
          <SettingsField
            id={`tmux-cfg-agent-${index}-id`}
            field={`agents.${index}.id`}
            data-path={`agents.${index}.id`}
            label="Id"
            description="Shown on the window badge."
            error={agentIdError(agents, index) ?? undefined}
            renderControl={(controlProps) => (
              <Input
                {...controlProps}
                className="tmux-mono"
                value={agent.id}
                spellCheck={false}
                onChange={(next) => onPatch({ id: next })}
              />
            )}
          />
          <SettingsField
            id={`tmux-cfg-agent-${index}-label`}
            field={`agents.${index}.label`}
            data-path={`agents.${index}.label`}
            label="Name"
            renderControl={(controlProps) => (
              <Input {...controlProps} value={agent.label} onChange={(next) => onPatch({ label: next })} />
            )}
          />
          <SettingsField
            id={`tmux-cfg-agent-${index}-process`}
            field={`agents.${index}.process`}
            data-path={`agents.${index}.process`}
            label="Process names"
            description="A pane runs this agent when a process under it has one of these names (comma separated); a Node CLI matches by its script name."
            error={agent.processes.length === 0 ? 'Add at least one process name.' : undefined}
            renderControl={(controlProps) => (
              <Input
                {...controlProps}
                className="tmux-mono"
                value={processText}
                placeholder="claude"
                spellCheck={false}
                onChange={(next) => {
                  setProcessText(next)
                  onPatch({ processes: parseProcesses(next) })
                }}
              />
            )}
          />
        </SettingsList>
      </SettingsSection>
      <PatternEditor
        agentIndex={index}
        list="waiting_patterns"
        label="Waiting for you"
        description="Regular expressions matched against the last lines of the pane; a match means the agent asks for input (it wins over working)."
        patterns={agent.waiting}
        onChange={(next) => onPatch({ waiting: next })}
      />
      <PatternEditor
        agentIndex={index}
        list="working_patterns"
        label="Working"
        description="A match means the agent is busy. With no patterns, any change of the pane within the working window counts as busy."
        patterns={agent.working}
        onChange={(next) => onPatch({ working: next })}
      />
      <SettingsSection>
        <SettingsList>
          <SettingsRow
            label="Remove this agent"
            description="Panes running it show their command instead of an agent badge."
            control={
              <Button size="sm" variant="ghost" onClick={onRemove}>
                <Trash2 aria-hidden />
                Remove
              </Button>
            }
          />
        </SettingsList>
      </SettingsSection>
    </div>
  )
}

export function TmuxConfigForm(props: ConfigFormProps) {
  const model = readTmuxConfiguration(props.value)
  const [openAgent, setOpenAgent] = useState<number | null>(null)
  const rootRef = useRef<HTMLDivElement | null>(null)
  const handledFocus = useRef('')
  const focusKey = props.focusField?.length ? JSON.stringify(props.focusField.map(String)) : ''
  const error = (pointer: string) => props.errors?.get(pointer)
  const change = (next: JsonValue) => props.onChange(next)

  // A deep link opens the agent it names, then focuses the exact field.
  useEffect(() => {
    if (!focusKey) {
      handledFocus.current = ''
      return
    }
    if (handledFocus.current === focusKey || !rootRef.current) return
    const path = JSON.parse(focusKey) as string[]
    if (path[0] === 'agents' && path[1] !== undefined && openAgent !== Number(path[1])) {
      setOpenAgent(Number(path[1]))
      return
    }
    const target =
      rootRef.current.querySelector<HTMLElement>(`[data-path="${CSS.escape(path.join('.'))}"]`) ??
      rootRef.current.querySelector<HTMLElement>(`[data-field="${CSS.escape(path[0])}"]`)
    target?.scrollIntoView({ block: 'center' })
    const focusable = target?.matches('input, button, [tabindex]')
      ? target
      : target?.querySelector<HTMLElement>('input, button, [tabindex]')
    if (!focusable) return
    handledFocus.current = focusKey
    focusable.focus()
  }, [focusKey, openAgent])

  if (model.hasOpaqueRoot) {
    return (
      <StatusPanel
        variant="warn"
        headline="This configuration is not an object"
        detail="It is kept as stored. Replace it with the defaults to edit it here."
        action={
          <Button size="sm" variant="ghost" onClick={() => change({})}>
            Use defaults
          </Button>
        }
      />
    )
  }

  const activeAgent = openAgent !== null && model.agents[openAgent] ? openAgent : null

  const overview = (
    <div className="tmux-config-overview">
      <SettingsSection
        title="tmux server"
        description="Which tmux the worker drives. Changes apply to the next tmux call; open console terminals keep their server."
      >
        <SettingsList>
          <TextSetting
            field="tmux_binary"
            label="tmux binary"
            description="A name looked up on PATH or an absolute path. The worker's own executable is never used, even though it is also called tmux."
            placeholder={DEFAULT_TMUX_BINARY}
            value={textValue(model, 'tmux_binary')}
            error={error('/tmux_binary')}
            onChange={(next) => change(withText(props.value, 'tmux_binary', next))}
          />
          <TextSetting
            field="socket_name"
            label="Socket name"
            description="tmux -L: a named server under tmux's socket directory. Empty uses the default server."
            placeholder="default"
            value={textValue(model, 'socket_name')}
            error={error('/socket_name')}
            onChange={(next) => change(withText(props.value, 'socket_name', next))}
          />
          <TextSetting
            field="socket_path"
            label="Socket path"
            description="tmux -S: an explicit socket file; wins over the socket name."
            placeholder="/tmp/tmux-1000/default"
            value={textValue(model, 'socket_path')}
            error={error('/socket_path')}
            onChange={(next) => change(withText(props.value, 'socket_path', next))}
          />
        </SettingsList>
      </SettingsSection>

      <SettingsSection title="Live updates" description="Polling runs only while at least one console page is open.">
        <SettingsList>
          {(['poll_interval_ms', 'ports_interval_ms', 'busy_window_ms', 'command_timeout_ms'] as const).map((field) => (
            <NumberSetting
              key={field}
              field={field}
              value={numberValue(model, field)}
              error={error(`/${field}`)}
              onChange={(next) => change(withNumber(props.value, field, next))}
            />
          ))}
        </SettingsList>
      </SettingsSection>

      <SettingsSection title="Console terminals" description="Each open tmux page runs one tmux client in the worker.">
        <SettingsList>
          {(['max_terminals', 'terminal_idle_timeout_ms'] as const).map((field) => (
            <NumberSetting
              key={field}
              field={field}
              value={numberValue(model, field)}
              error={error(`/${field}`)}
              onChange={(next) => change(withNumber(props.value, field, next))}
            />
          ))}
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        title="Agents"
        description="How a pane is recognised as running an agent CLI, and how its state is read from the screen."
        action={
          model.agentsOpaque ? undefined : (
            <Button
              size="sm"
              variant="ghost"
              onClick={() => {
                change(withNewAgent(props.value, 'agent'))
                setOpenAgent(model.agents.length)
              }}
            >
              <Plus aria-hidden />
              Add agent
            </Button>
          )
        }
      >
        {model.agentsOpaque ? (
          <StatusPanel
            variant="warn"
            headline="The agent list is stored in a form this page cannot edit"
            detail="It is kept as stored."
            action={
              <Button size="sm" variant="ghost" onClick={() => change(withDefaultAgents(props.value))}>
                Use built-in agents
              </Button>
            }
          />
        ) : model.agentsDefault ? (
          <SettingsList>
            <SettingsRow
              label="Built-in agents"
              description="Claude Code, Codex, OpenCode and Gemini, as the worker defines them. Adding an agent replaces this list with your own."
            />
          </SettingsList>
        ) : model.agents.length === 0 ? (
          <SettingsList>
            <SettingsRow
              label="No agents"
              description="No pane shows an agent badge."
              control={
                <Button size="sm" variant="ghost" data-settings-deck-fallback onClick={() => change(withDefaultAgents(props.value))}>
                  Use built-in agents
                </Button>
              }
            />
          </SettingsList>
        ) : (
          <Panel className="tmux-config-agents">
            <List aria-label="Agents">
              {model.agents.map((agent, index) => (
                <ListItem
                  // biome-ignore lint/suspicious/noArrayIndexKey: ids are edited in place, the position is the identity.
                  key={index}
                  data-settings-deck-fallback={index === 0 ? '' : undefined}
                  leading={<Bot aria-hidden />}
                  label={agent.label || agent.id || 'Unnamed agent'}
                  description={
                    agent.processes.length > 0 ? `matches ${agent.processes.join(', ')}` : 'matches no process yet'
                  }
                  trailing={
                    <span className="tmux-mono tmux-faint">
                      {agent.waiting.length + agent.working.length} pattern
                      {agent.waiting.length + agent.working.length === 1 ? '' : 's'}
                    </span>
                  }
                  onClick={() => setOpenAgent(index)}
                />
              ))}
            </List>
          </Panel>
        )}
      </SettingsSection>
    </div>
  )

  return (
    <div className="tmux-config" ref={rootRef}>
    <SettingsDeck
      open={activeAgent !== null}
      title={activeAgent !== null ? model.agents[activeAgent].label || model.agents[activeAgent].id || 'Agent' : 'Agent'}
      backLabel="tmux settings"
      overview={overview}
      detail={
        activeAgent !== null ? (
          <AgentDetail
            key={activeAgent}
            agents={model.agents}
            index={activeAgent}
            onPatch={(patch) => change(withAgent(props.value, activeAgent, patch))}
            onRemove={() => {
              change(withoutAgent(props.value, activeAgent))
              setOpenAgent(null)
            }}
          />
        ) : null
      }
      onBack={() => setOpenAgent(null)}
      autoFocusDetail={!focusKey}
    />
    </div>
  )
}
