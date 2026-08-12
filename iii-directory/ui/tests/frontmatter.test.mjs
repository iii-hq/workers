import assert from 'node:assert/strict'
import test from 'node:test'
import {
  frontmatterBody,
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
