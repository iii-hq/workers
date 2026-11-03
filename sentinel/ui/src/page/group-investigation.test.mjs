import test from 'node:test'
import assert from 'node:assert/strict'
import { MEMBER_EVIDENCE_MAX_BYTES, CONTEXT_MAX_BYTES, groupInvestigationProblem, hypothesisBytes, intentSignature } from './group-investigation.js'
test('group selection accepts only 2 to 5 distinct IDs', () => {
  for (const ids of [[], ['a'], ['a','a'], ['a',''], ['a','b','c','d','e','f']]) assert.ok(groupInvestigationProblem(ids))
  assert.equal(groupInvestigationProblem(['a','b']), '')
  assert.equal(groupInvestigationProblem(['a','b','c','d','e']), '')
})
test('hypothesis uses backend UTF-8 byte boundary', () => {
  assert.equal(hypothesisBytes('é'), 2)
  assert.equal(groupInvestigationProblem(['a','b'], 'é'.repeat(1000)), '')
  assert.ok(groupInvestigationProblem(['a','b'], 'é'.repeat(1001)))
})
test('same selection reordering keeps request intent, edits change it', () => {
  assert.equal(intentSignature(['b','a'],'p','h'), intentSignature(['a','b'],'p','h'))
  assert.notEqual(intentSignature(['a','b'],'p','h'), intentSignature(['a','b'],'p','edited'))
})
test('visible budgets match joint backend evidence excerpt and context limits', () => {
  assert.equal(MEMBER_EVIDENCE_MAX_BYTES, 3500)
  assert.equal(CONTEXT_MAX_BYTES, 32768)
})
