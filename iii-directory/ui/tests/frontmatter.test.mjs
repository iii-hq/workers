import assert from 'node:assert/strict'
import test from 'node:test'
import {
  frontmatterBody,
  frontmatterFieldIsSimpleBoolean,
  readFrontmatterField,
  restoreFrontmatterFields,
  setFrontmatterField,
  withoutFrontmatterFields,
} from '../src/page/frontmatter.ts'

test('structured fields round-trip without dropping advanced metadata', () => {
  const content = `---
name: browser
description: >-
  Drive a browser and
  inspect real pages.
type: how-to
function_id: browser::open
---
# Browser

Body.
`
  const name = readFrontmatterField(content, ['title', 'name'], 'name')
  const description = readFrontmatterField(content, ['description'])
  assert.equal(name.value, 'browser')
  assert.equal(description.value, 'Drive a browser and inspect real pages.')

  const source = withoutFrontmatterFields(content, [name.key, description.key])
  assert.doesNotMatch(source, /^name:|^description:/m)
  assert.match(source, /^type: how-to$/m)
  assert.match(source, /^function_id: browser::open$/m)

  const restored = restoreFrontmatterFields(source, [name, description])
  assert.match(restored, /description: >-\n  Drive a browser and\n  inspect real pages\./)
  assert.equal(readFrontmatterField(restored, ['name']).value, 'browser')
  assert.equal(
    readFrontmatterField(restored, ['description']).value,
    'Drive a browser and inspect real pages.',
  )
  assert.equal(frontmatterBody(restored), '# Browser\n\nBody.\n')
})

test('editing a field creates frontmatter around a plain markdown body', () => {
  const next = setFrontmatterField('# Body\n', 'name', 'ptbr', true)
  assert.equal(readFrontmatterField(next, ['name']).value, 'ptbr')
  assert.equal(frontmatterBody(next), '# Body\n')
})

test('quoted multiline descriptions decode for the textarea', () => {
  const next = setFrontmatterField(
    '---\nname: test\ndescription: old\n---\nBody\n',
    'description',
    'first line\nsecond line',
  )
  assert.equal(
    readFrontmatterField(next, ['description']).value,
    'first line\nsecond line',
  )
})

test('model invocation option writes a bare boolean and stays out of Content', () => {
  const content = `---
name: ask-matt
description: Route to the right skill.
license: MIT
---
Body
`
  const enabled = setFrontmatterField(
    content,
    'disable-model-invocation',
    'true',
    true,
  )
  assert.equal(
    readFrontmatterField(enabled, ['disable-model-invocation']).value,
    'true',
  )
  assert.match(enabled, /^disable-model-invocation: true$/m)
  assert.match(enabled, /^license: MIT$/m)

  const fields = [
    readFrontmatterField(enabled, ['name']),
    readFrontmatterField(enabled, ['description']),
    {
      ...readFrontmatterField(enabled, ['disable-model-invocation']),
      bare: true,
    },
  ]
  const editorSource = withoutFrontmatterFields(
    enabled,
    fields.map((field) => field.key),
  )
  assert.doesNotMatch(editorSource, /^disable-model-invocation: true$/m)
  assert.match(editorSource, /^license: MIT$/m)

  const restored = restoreFrontmatterFields(
    editorSource.replace('Body', 'Updated body'),
    fields,
  )
  assert.match(restored, /^disable-model-invocation: true$/m)
  assert.match(restored, /^license: MIT$/m)
  assert.match(restored, /Updated body/)

  assert.equal(
    frontmatterFieldIsSimpleBoolean(
      '---\ndisable-model-invocation: true\n---\n',
      'disable-model-invocation',
    ),
    true,
  )
  assert.equal(
    frontmatterFieldIsSimpleBoolean(
      '---\ndisable-model-invocation: !!bool |-\n  true\n---\n',
      'disable-model-invocation',
    ),
    false,
  )
  assert.equal(
    frontmatterFieldIsSimpleBoolean(
      '---\ndisable-model-invocation: true\n  trailing\n---\n',
      'disable-model-invocation',
    ),
    false,
  )
  assert.equal(
    frontmatterFieldIsSimpleBoolean(
      "---\n'disable-model-invocation': true\n---\n",
      'disable-model-invocation',
    ),
    false,
  )
  assert.equal(
    frontmatterFieldIsSimpleBoolean(
      '---\n? disable-model-invocation\n: true\n---\n',
      'disable-model-invocation',
    ),
    false,
  )
  assert.equal(
    frontmatterFieldIsSimpleBoolean(
      '---\n{disable-model-invocation: true}\n---\n',
      'disable-model-invocation',
    ),
    false,
  )
  assert.equal(
    frontmatterFieldIsSimpleBoolean(
      '---\n? !!str disable-model-invocation\n: true\n---\n',
      'disable-model-invocation',
    ),
    false,
  )
  const advanced = `---
name: ask-matt
disable-model-invocation: !!bool |-
  true
---
Body
`
  assert.match(
    withoutFrontmatterFields(advanced, ['name']),
    /^disable-model-invocation: !!bool \|-$/m,
  )
})
