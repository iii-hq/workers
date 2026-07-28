import { useEffect, useState } from 'react'
import { Button, CodeEditor, EmptyState, Input } from '@iii-dev/console-ui'
import { Plus, X } from './icons'
import type { MemoryRule } from './memory-data'

/**
 * The bank's markdown rules — injected whole into the system prompt of
 * every session using this bank. Each rule is a plain `.md` file on disk;
 * editing here and editing the file are equivalent. Saving empty content
 * removes a rule. Editing uses the console's shared Monaco `CodeEditor`
 * (never a bundled editor).
 */

interface RulesPanelProps {
  rules: MemoryRule[]
  onSet: (name: string, content: string) => Promise<boolean>
  busy: boolean
}

function RuleEditor({
  name,
  initial,
  onSet,
  busy,
}: {
  name: string
  initial: string
  onSet: (name: string, content: string) => Promise<boolean>
  busy: boolean
}) {
  const [content, setContent] = useState(initial)
  // Re-seed the editor when a live refresh changes the rule on disk and
  // the user has no local edits in flight. After a successful save,
  // `touched` stays set until the refreshed `initial` catches up with the
  // saved content — clearing it earlier would flash the stale text back.
  const [touched, setTouched] = useState(false)
  const [confirmDelete, setConfirmDelete] = useState(false)
  useEffect(() => {
    if (!touched) setContent(initial)
    else if (content === initial) setTouched(false)
  }, [initial, touched, content])

  const dirty = content !== initial
  return (
    <div className="mem-rule">
      <div className="mem-rule-head">
        <span className="mem-rule-name">{name}.md</span>
        <div className="mem-rule-actions">
          {dirty ? (
            <>
              <span className="mem-unsaved">unsaved</span>
              <Button
                variant="ghost"
                size="sm"
                disabled={busy}
                onClick={() => {
                  // Emptied content means delete — that path gets the
                  // explicit confirm, never a silent removal via save.
                  if (content.trim() === '') {
                    setConfirmDelete(true)
                    return
                  }
                  void onSet(name, content)
                }}
              >
                save
              </Button>
            </>
          ) : null}
          {confirmDelete ? (
            <>
              <Button
                variant="ghost"
                size="sm"
                disabled={busy}
                className="mem-danger"
                onClick={() => {
                  void onSet(name, '')
                  setConfirmDelete(false)
                }}
              >
                delete {name}.md?
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setConfirmDelete(false)}
              >
                keep
              </Button>
            </>
          ) : (
            <Button
              variant="ghost"
              size="icon"
              disabled={busy}
              aria-label={`delete rule ${name}`}
              onClick={() => setConfirmDelete(true)}
            >
              <X size={14} aria-hidden />
            </Button>
          )}
        </div>
      </div>
      <div className="mem-rule-editor">
        <CodeEditor
          value={content}
          onChange={(next) => {
            setContent(next)
            setTouched(true)
          }}
          language="markdown"
          aria-label={`rule ${name}`}
          className="mem-editor"
          placeholder="empty the content and save to remove this rule (asks to confirm)"
        />
      </div>
    </div>
  )
}

/** A rule name from whatever was typed or pasted: lowercased, separators
 * dashed, invalid chars dropped, trimmed to the 64-char limit. */
function slugifyRuleName(raw: string): string {
  return raw
    .toLowerCase()
    .replace(/[\s:/\\]+/g, '-')
    .replace(/[^a-z0-9_-]/g, '')
    .replace(/-{2,}/g, '-')
    .replace(/^[-_]+/, '')
    .slice(0, 64)
    .replace(/[-_]+$/, '')
}

export function RulesPanel({ rules, onSet, busy }: RulesPanelProps) {
  const [newName, setNewName] = useState('')
  const validName = /^[a-z0-9][a-z0-9_-]{0,63}$/.test(newName)
  const suggestion = validName ? '' : slugifyRuleName(newName)

  return (
    <div className="mem-stack">
      <p className="mem-hint">
        every chat on this bank starts with these — word for word, every turn.
        put what must always hold here: voice, conventions, constants. correct
        the agent in chat ("stop using em-dashes") and the correction lands in
        learned.md by itself; each rule is a plain markdown file on disk, and
        editing it here or in your editor is the same thing.
      </p>
      {rules.length === 0 ? (
        <EmptyState
          title="no rules yet"
          description="add one named 'style' with a line like 'answer tersely, code first' — then ask anything in chat on this bank and watch every reply follow it. corrections you make in chat will grow a learned.md here on their own."
        />
      ) : (
        rules.map((rule) => (
          <RuleEditor
            key={rule.name}
            name={rule.name}
            initial={rule.content}
            onSet={onSet}
            busy={busy}
          />
        ))
      )}
      <form
        className="mem-stack tight"
        onSubmit={(e) => {
          e.preventDefault()
          if (busy) return
          const name = validName ? newName : suggestion
          if (!name) return
          void onSet(name, `# ${name}\n`).then((ok) => {
            if (ok) setNewName('')
          })
        }}
      >
        <div className="mem-row">
          <Input
            value={newName}
            onChange={setNewName}
            placeholder="name the rule first (e.g. style) — content goes in the editor after"
            aria-label="new rule name"
            className="mem-flex1"
          />
          <Button
            type="submit"
            variant="ghost"
            size="sm"
            disabled={busy || (!validName && !suggestion)}
            className="mem-gap1"
          >
            <Plus size={14} aria-hidden />
            add rule
          </Button>
        </div>
        {newName && !validName ? (
          <p className="mem-subhint">
            {suggestion
              ? `names are lowercase-with-dashes (it becomes ${suggestion}.md) — adding will create "${suggestion}"; paste the content into its editor after`
              : 'names are lowercase letters, numbers, and dashes — like style or coding-rules'}
          </p>
        ) : null}
      </form>
    </div>
  )
}
