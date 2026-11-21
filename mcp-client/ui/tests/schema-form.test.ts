import { describe, expect, it } from 'vitest'
import {
  type FormField,
  fieldsFromSchema,
  fieldText,
  missingRequired,
  optionalLabel,
  parsePayload,
  requiredText,
  setField,
} from '../src/schema-form'
import listIssues from './fixtures/linear-list-issues.schema.json'
import saveIssue from './fixtures/linear-save-issue.schema.json'

function byName(schema: unknown): Record<string, FormField> {
  const { required, optional } = fieldsFromSchema(schema)
  return Object.fromEntries([...required, ...optional].map((field) => [field.name, field]))
}

const f = byName(listIssues)
const s = byName(saveIssue)

describe('fieldsFromSchema on Linear', () => {
  it('list_issues: 23 optional inputs, kinds from the schema', () => {
    const { required, optional } = fieldsFromSchema(listIssues)
    expect([required.length, optional.length]).toEqual([0, 23])
    expect(f.team).toMatchObject({ kind: 'string', required: false, label: 'team' })
    expect(f.assignee.kind).toBe('string')
    expect(f.limit).toMatchObject({ kind: 'number', hint: 'max 250 · default 50' })
    expect(f.orderBy).toMatchObject({ kind: 'enum', enumValues: ['createdAt', 'updatedAt'], hint: 'default updatedAt' })
    expect(f.includeArchived).toMatchObject({ kind: 'boolean', hint: 'default false' })
    expect(f.fields.kind).toBe('string-list')
    expect(f.cursor.description).toContain('Omit for the first page')
  })

  it('save_issue: 32 inputs, nested ones as JSON', () => {
    const { required, optional } = fieldsFromSchema(saveIssue)
    expect([required.length, optional.length]).toEqual([0, 32])
    expect(s.title.kind).toBe('string')
    expect(s.estimate.kind).toBe('number')
    expect(s.priority.kind).toBe('number')
    expect(s.addLabels.kind).toBe('string-list')
    expect(s.links.kind).toBe('json')
    expect(s.patch.kind).toBe('json')
    expect(s.slaType.kind).toBe('json')
    expect(s.slaBreachesAt.kind).toBe('json')
  })
})

describe('fieldsFromSchema, the rest', () => {
  it('puts required first and reads a prompt as all strings', () => {
    const prompt = {
      type: 'object',
      properties: {
        focus: { type: 'string' },
        language: { type: 'string', title: 'Language', description: 'The language to review' },
      },
      required: ['language'],
    }
    expect(fieldsFromSchema(prompt)).toEqual({
      required: [
        { name: 'language', label: 'Language', kind: 'string', required: true, description: 'The language to review' },
      ],
      optional: [{ name: 'focus', label: 'focus', kind: 'string', required: false }],
    })
  })

  it('hints a number with its bounds', () => {
    const x = byName({
      type: 'object',
      properties: {
        n: { type: 'integer', minimum: 1, maximum: 10 },
        xs: { type: 'array', items: { type: 'number' } },
        o: { type: 'object' },
      },
    })
    expect(x.n).toMatchObject({ kind: 'number', hint: 'min 1 · max 10' })
    expect(x.xs.kind).toBe('json')
    expect(x.o.kind).toBe('json')
  })

  it('agrees with missingRequired on required names: deduped, absent properties kept', () => {
    const schema = { type: 'object', properties: { team: { type: 'string' } }, required: ['team', 'ghost', 'team'] }
    const { required, optional } = fieldsFromSchema(schema)
    expect(required.map((field) => field.name)).toEqual(['team', 'ghost'])
    expect(required[1].kind).toBe('json')
    expect(optional).toEqual([])
    expect(missingRequired(schema, {})).toEqual(['team', 'ghost'])
  })

  it('gives no field for a schema that is not an object with properties', () => {
    for (const x of [undefined, null, {}, { type: 'object' }, 'x']) {
      expect(fieldsFromSchema(x)).toEqual({ required: [], optional: [] })
    }
  })
})

describe('setField and fieldText', () => {
  it('writes the typed value and removes an emptied key', () => {
    expect(setField({}, f.team, 'ENG')).toEqual({ team: 'ENG' })
    expect(setField({ team: 'ENG', limit: 5 }, f.team, '')).toEqual({ limit: 5 })
    expect(setField({}, f.limit, '5')).toEqual({ limit: 5 })
    expect(setField({}, f.limit, 'five')).toEqual({ limit: 'five' })
    expect(setField({ limit: 5 }, f.limit, '')).toEqual({})
    expect(setField({}, f.includeArchived, true)).toEqual({ includeArchived: true })
    expect(setField({ includeArchived: false }, f.includeArchived, null)).toEqual({})
    expect(setField({}, f.orderBy, 'createdAt')).toEqual({ orderBy: 'createdAt' })
    expect(setField({}, s.addLabels, 'Bug\n\n Urgent \n')).toEqual({ addLabels: ['Bug', 'Urgent'] })
    expect(setField({ addLabels: ['Bug'] }, s.addLabels, ' \n')).toEqual({})
    expect(setField({}, s.links, '[{"url":"https://x.test","title":"X"}]')).toEqual({
      links: [{ url: 'https://x.test', title: 'X' }],
    })
    expect(() => setField({}, s.links, '[{')).toThrow(SyntaxError)
  })

  it('keeps an edited key where it is', () => {
    let payload = setField({}, f.team, 'ENG')
    payload = setField(payload, f.limit, '5')
    payload = setField(payload, f.team, 'OPS')
    expect(Object.keys(payload)).toEqual(['team', 'limit'])
  })

  it('never changes the payload it was given', () => {
    const before = { team: 'ENG' }
    setField(before, f.limit, '5')
    expect(before).toEqual({ team: 'ENG' })
  })

  it('reads a field back as text', () => {
    const payload = { team: 'ENG', limit: 5, includeArchived: false, fields: ['id', 'title'], assignee: null }
    expect(fieldText(payload, f.team)).toBe('ENG')
    expect(fieldText(payload, f.limit)).toBe('5')
    expect(fieldText(payload, f.includeArchived)).toBe('false')
    expect(fieldText(payload, f.fields)).toBe('id\ntitle')
    expect(fieldText(payload, f.assignee)).toBe('')
    expect(fieldText(payload, f.cursor)).toBe('')
    expect(fieldText({ links: [{ url: 'u', title: 't' }] }, s.links)).toBe(
      '[\n  {\n    "url": "u",\n    "title": "t"\n  }\n]',
    )
  })

  it('missingRequired names absent and empty required keys', () => {
    const schema = { type: 'object', properties: { a: {}, b: {}, c: {} }, required: ['a', 'b'] }
    expect(missingRequired(schema, {})).toEqual(['a', 'b'])
    expect(missingRequired(schema, { a: '', b: 0 })).toEqual(['a'])
    expect(missingRequired(listIssues, {})).toEqual([])
  })

  it('says which required inputs are missing', () => {
    expect(requiredText(['team'])).toBe('team is required')
    expect(requiredText(['title', 'team'])).toBe('title, team are required')
  })

  it('reads the JSON view as a payload object or says why not', () => {
    expect(parsePayload('{"team":"ENG"}')).toEqual({ payload: { team: 'ENG' } })
    for (const text of ['[]', '1', 'null', '"x"']) {
      expect(parsePayload(text)).toEqual({ error: 'The input must be a JSON object' })
    }
    expect(parsePayload('{"team":')).toMatchObject({ error: expect.stringMatching(/^Not valid JSON: ./) })
  })

  it('labels the optional disclosure', () => {
    expect(optionalLabel(2, 7)).toBe('7 more optional inputs')
    expect(optionalLabel(2, 1)).toBe('1 more optional input')
    expect(optionalLabel(0, 23)).toBe('23 optional inputs')
  })
})
