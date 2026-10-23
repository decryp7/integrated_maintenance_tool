import { test } from 'node:test';
import assert from 'node:assert/strict';
import { problems } from './lint-effects.mjs';

test('an effect that only reads or calls out is fine', () => {
  assert.deepEqual(problems('effect(() => { console.log(a()); });'), []);
});
test('a state-writing effect needs the tag', () => {
  assert.equal(problems('effect(() => { x.set(1); });').length, 1);
  assert.equal(problems('effect(() => { patchState(s, {a: 1}); });').length, 1);
  assert.equal(problems('effect(() => { x.update((v) => v + 1); });').length, 1);
});
test('the tag in the comment right above justifies it', () => {
  assert.deepEqual(problems('// effect-state: mirrors the router\n// more words\neffect(() => { x.set(1); });'), []);
  assert.equal(problems('// effect-state: far\n\neffect(() => { x.set(1); });').length, 1);
});
test('writes in strings and comments do not count, nested braces do', () => {
  assert.deepEqual(problems("effect(() => { log('x.set(1)'); /* y.set(2) */ });"), []);
  assert.equal(problems('effect(() => { if (a) { untracked(() => { x.set(1); }); } });').length, 1);
});
test('.effect( members are not effects and the line is reported', () => {
  assert.deepEqual(problems('foo.effect(() => x.set(1));'), []);
  assert.equal(problems('a\nb\neffect(() => x.set(1));')[0].line, 3);
});
