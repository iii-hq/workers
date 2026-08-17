/**
 * A JSON Schema as a field table: name, type, required, default, and the
 * schema's own description, nested objects indented under their parent.
 *
 * The raw schema stays one tab away for the cases this cannot express
 * (`oneOf` unions, `$ref` chains, custom keywords). This view exists because
 * an operator reading "what does this function take" should not have to parse
 * draft-07 by eye — the same reason the registry site renders functions as
 * docs rather than as JSON.
 */

import { JsonHighlight } from '@iii-dev/console-ui'
import { Note } from './widgets'

const MAX_DEPTH = 3

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** `type` may be a string, a nullable union, or implied by the keywords. */
function typeName(schema: Record<string, unknown>): string {
  const t = schema.type
  if (typeof t === 'string') return t
  if (Array.isArray(t)) {
    const named = t.filter((x) => typeof x === 'string' && x !== 'null')
    const nullable = t.includes('null')
    if (named.length > 0) {
      return `${named.join(' | ')}${nullable ? '?' : ''}`
    }
  }
  if (Array.isArray(schema.enum)) return 'enum'
  if (isRecord(schema.properties)) return 'object'
  if (schema.items !== undefined) return 'array'
  if (Array.isArray(schema.oneOf) || Array.isArray(schema.anyOf)) return 'union'
  if (typeof schema.$ref === 'string')
    return schema.$ref.split('/').pop() ?? 'ref'
  return 'any'
}

interface Row {
  path: string
  name: string
  type: string
  required: boolean
  description?: string
  defaultValue?: string
  enumValues?: string
  depth: number
}

function collect(
  schema: unknown,
  depth: number,
  prefix: string,
  out: Row[],
): void {
  if (!isRecord(schema) || depth > MAX_DEPTH) return
  const props = isRecord(schema.properties) ? schema.properties : null
  if (!props) return
  const required = new Set(
    Array.isArray(schema.required)
      ? schema.required.filter((k): k is string => typeof k === 'string')
      : [],
  )
  // Required fields first: they are what a caller must supply.
  const keys = Object.keys(props)
  const ordered = [
    ...keys.filter((k) => required.has(k)),
    ...keys.filter((k) => !required.has(k)),
  ]
  for (const key of ordered) {
    const field = props[key]
    if (!isRecord(field)) continue
    out.push({
      path: `${prefix}${key}`,
      name: key,
      type: typeName(field),
      required: required.has(key),
      description:
        typeof field.description === 'string' ? field.description : undefined,
      defaultValue:
        field.default !== undefined ? JSON.stringify(field.default) : undefined,
      enumValues: Array.isArray(field.enum)
        ? field.enum.map((v) => JSON.stringify(v)).join(' · ')
        : undefined,
      depth,
    })
    collect(field, depth + 1, `${prefix}${key}.`, out)
    const items = field.items
    if (isRecord(items)) collect(items, depth + 1, `${prefix}${key}[].`, out)
  }
}

/** Field names a caller can type, for the invoke editor's completions. */
export function schemaFieldNames(schema: unknown): string[] {
  const rows: Row[] = []
  collect(schema, 0, '', rows)
  return [...new Set(rows.map((r) => r.name))]
}

export function SchemaTable({
  schema,
  empty,
}: {
  schema: unknown
  empty: string
}) {
  if (schema === undefined || schema === null) return <Note>{empty}</Note>

  const rows: Row[] = []
  collect(schema, 0, '', rows)

  if (rows.length === 0) {
    // A schema with no properties is still information: a scalar response, a
    // free-form object. Show it rather than claiming there is nothing.
    return (
      <JsonHighlight
        code={JSON.stringify(schema, null, 2)}
        className="console-catalog-json"
        wrap
      />
    )
  }

  const title =
    isRecord(schema) && typeof schema.title === 'string' ? schema.title : null
  const description =
    isRecord(schema) && typeof schema.description === 'string'
      ? schema.description
      : null

  return (
    <div className="console-catalog-schema">
      {title || description ? (
        <div className="console-catalog-schema-head">
          {title ? <span className="title">{title}</span> : null}
          {description ? <span className="desc">{description}</span> : null}
        </div>
      ) : null}
      <table>
        <thead>
          <tr>
            <th>field</th>
            <th>type</th>
            <th>notes</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row.path}>
              <td>
                <span
                  className="field"
                  style={{ paddingLeft: `${row.depth * 12}px` }}
                >
                  {row.name}
                </span>
                {row.required ? <span className="req">required</span> : null}
              </td>
              <td>
                <span className="type">{row.type}</span>
              </td>
              <td>
                {row.description ? (
                  <span className="desc">{row.description}</span>
                ) : null}
                {row.enumValues ? (
                  <span className="enum">one of {row.enumValues}</span>
                ) : null}
                {row.defaultValue ? (
                  <span className="default">default {row.defaultValue}</span>
                ) : null}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
