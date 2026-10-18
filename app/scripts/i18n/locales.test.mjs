import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  normalizeMessages,
  parseMessage,
  parseXliff,
  placeholders,
  pseudoMessage,
  pseudoText,
  pseudoTranslations,
  runtimeJson,
  serializeMessage,
  targetTranslations,
  xliffContentToText,
} from './locales.mjs';

const XLF = `<?xml version="1.0" encoding="UTF-8" ?>
<xliff version="1.2" xmlns="urn:oasis:names:tc:xliff:document:1.2">
  <file source-language="en-US" datatype="plaintext" original="ng2.template">
    <body>
      <trans-unit id="a.hello" datatype="html">
        <source>Hello <x id="INTERPOLATION" equiv-text="{{ name }}"/> &amp; &lt;friends&gt;</source>
        <context-group purpose="location"><context context-type="sourcefile">src/app/x.ts</context><context context-type="linenumber">12</context></context-group>
        <note priority="1" from="description">Greeting</note>
        <note priority="1" from="meaning">verb</note>
      </trans-unit>
      <trans-unit id="a.spans" datatype="html">
        <source>{VAR_PLURAL, plural, =1 {<x id="INTERPOLATION"/> span} other {<x id="INTERPOLATION"/> spans}}</source>
      </trans-unit>
    </body>
  </file>
</xliff>`;

test('parses XLIFF units into runtime text with placeholders, meaning and description', () => {
  assert.deepEqual(parseXliff(XLF), [
    { id: 'a.hello', source: 'Hello {$INTERPOLATION} & <friends>', meaning: 'verb', description: 'Greeting' },
    { id: 'a.spans', source: '{VAR_PLURAL, plural, =1 {{INTERPOLATION} span} other {{INTERPOLATION} spans}}' },
  ]);
  assert.equal(xliffContentToText('<x id="START_TAG_B" ctype="x-b" equiv-text="&lt;b&gt;"/>x<x id="CLOSE_TAG_B"/>'), '{$START_TAG_B}x{$CLOSE_TAG_B}');
});

test('a unit without a source is an error', () => {
  assert.throws(() => parseXliff('<trans-unit id="x"><target>t</target></trans-unit>'), /no <source>/);
});

test('normalised messages ignore source locations and order', () => {
  const moved = XLF.replace('<context context-type="linenumber">12</context>', '<context context-type="linenumber">99</context>');
  assert.deepEqual(normalizeMessages(parseXliff(moved)), normalizeMessages(parseXliff(XLF).reverse()));
  assert.notDeepEqual(normalizeMessages(parseXliff(XLF.replace('Hello', 'Hi'))), normalizeMessages(parseXliff(XLF)));
});

test('message syntax round-trips, including nested ICU', () => {
  for (const s of ['plain', 'a {$PH} b', '{VAR_PLURAL, plural, =1 {one {X}} other {{VAR_SELECT_1, select, a {A} other {B}} many}}', 'JSON: {"k": 1} }', '']) {
    assert.equal(serializeMessage(parseMessage(s)), s);
  }
  assert.deepEqual(placeholders(parseMessage('{VAR_PLURAL, plural, =1 {{A}} other {{$B} {A}}} {$C}')), ['A', 'B', 'C']);
  // A `{$X}` inside an ICU case (as converted from XLIFF) is written back without the `$`.
  assert.equal(serializeMessage(parseMessage('{VAR_PLURAL, plural, other {{$X} items}}')), '{VAR_PLURAL, plural, other {{X} items}}');
  assert.deepEqual(parseMessage('a {b} c'), [{ text: 'a {b} c' }]);
  assert.throws(() => parseMessage('{VAR_PLURAL, plural, =1 {x}'), /unclosed/);
  assert.throws(() => parseMessage('{$PH'), /unclosed placeholder/);
});

test('pseudo text is accented, about 30% longer and keeps non-letters', () => {
  const p = pseudoText('Open the file settings');
  assert.doesNotMatch(p, /[A-Za-z]/);
  assert.ok(p.length >= 'Open the file settings'.length * 1.2, p);
  assert.equal(pseudoText('12:30 — …'), '12:30 — …');
});

test('pseudo messages are bracketed, keep placeholders and ICU syntax', () => {
  assert.equal(pseudoMessage('Hi {$NAME}'), '⟦Ĥîî {$NAME}⟧');
  const icu = pseudoMessage('{VAR_PLURAL, plural, =1 {{N} span} other {{N} spans}}');
  assert.match(icu, /^\{VAR_PLURAL, plural, =1 \{⟦\{N\} [^A-Za-z]+⟧\} other \{⟦\{N\} [^A-Za-z]+⟧\}\}$/);
  assert.equal(pseudoMessage('{VAR_SELECT, select, file {x} other {y}}').includes('file {'), true);
  assert.deepEqual(Object.keys(pseudoTranslations(parseXliff(XLF))), ['a.hello', 'a.spans']);
});

test('translated targets become runtime translations; placeholder mismatches fail', () => {
  const msgs = [
    { id: 'a', source: 'Hi {$N}', target: 'Hallo {$N}' },
    { id: 'b', source: 'Bye' },
    { id: 'c', source: 'x', target: '  ' },
  ];
  assert.deepEqual(targetTranslations(msgs), { a: 'Hallo {$N}' });
  assert.throws(() => targetTranslations([{ id: 'a', source: 'Hi {$N}', target: 'Hallo' }]), /placeholders/);
});

test('runtime JSON has the loadTranslations shape with sorted ids', () => {
  assert.equal(runtimeJson('en-XA', { b: '2', a: '1' }), '{\n  "locale": "en-XA",\n  "translations": {\n    "a": "1",\n    "b": "2"\n  }\n}\n');
});
