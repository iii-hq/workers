import assert from 'node:assert/strict'
import { mayDismiss, pickOption, questionAnswers, textSelection, type AskQuestion, type Question } from './src/question-rules.ts'

const question: AskQuestion = {
  header: 'Builds', question: 'Where should builds run?', multi_select: false,
  options: [{ label: 'Same machine' }, { label: 'Sandbox', recommended: true }],
}
const noRecommendation = { ...question, options: [{ label: 'Same machine' }, { label: 'Sandbox' }] }
assert.deepEqual(textSelection(noRecommendation), { picked: [], other: '' }, 'no first-option default')
assert.equal(questionAnswers([noRecommendation], [textSelection(noRecommendation)]), null)
assert.deepEqual(textSelection(question), { picked: ['Sandbox'], other: '' }, 'the recommended option may be anywhere')
const typed = textSelection(question, ' Another machine ')
assert.deepEqual(typed, { picked: [], other: ' Another machine ' }, 'typing clears the choice')
assert.deepEqual(questionAnswers([question], [typed]), [{ header: 'Builds', other: 'Another machine' }])
assert.equal(questionAnswers([question], [textSelection(question, ' ')]), null, 'blank text is not an answer')
assert.deepEqual(textSelection(question, '').picked, ['Sandbox'], 'clearing text restores only the recommendation')
assert.deepEqual(textSelection(noRecommendation, '').picked, [], 'clearing text without a recommendation leaves no choice')
const picked = pickOption(question, typed, 'Same machine')
assert.deepEqual(picked, { picked: ['Same machine'], other: '' }, 'a choice clears free text')
assert.deepEqual(questionAnswers([question], [picked]), [{ header: 'Builds', choice: 'Same machine' }])
const multi = { ...question, multi_select: true }
const both = pickOption(multi, textSelection(multi), 'Same machine')
assert.deepEqual(questionAnswers([multi], [both]), [{ header: 'Builds', choices: ['Same machine', 'Sandbox'] }], 'choices use option order')
assert.deepEqual(pickOption(multi, both, 'Sandbox').picked, ['Same machine'], 'multi-select toggles a choice')
assert.equal(questionAnswers([question, noRecommendation], [textSelection(question), textSelection(noRecommendation)]), null, 'every question needs an answer')
assert.equal(questionAnswers([], []), null)
for (const klass of ['scope', 'routine', 'run_health', 'security', 'publish', 'external_cost', 'data']) {
  assert.equal(mayDismiss({ about: { class: klass } } as Question), ['scope', 'routine', 'run_health'].includes(klass), klass)
}
assert.equal(mayDismiss({ about: null } as Question), true, 'missing about defaults to scope')
console.log('questions: selection, structured answers and dismissal checks passed')
