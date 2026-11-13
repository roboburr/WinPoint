// D-G31 in vendor/ui/js/mock/settings.js (R3-UI, notes/claude/round3/r3-ui.md): a language other than nl/en is
// invalid_value with the host's text and key (a wrong type stays bad_value), and settings.import keeps the current
// language when the file's is invalid or (D-G49, WU-FIX) missing.
//   node --experimental-default-type=module tests/i18n/mock_settings.test.mjs      (from the project root)
import assert from 'node:assert/strict';
const mod = (await import(new URL('../../vendor/ui/js/mock/settings.js', import.meta.url))).default;
const M = mod.methods;
const m = { emit() {} };
async function fails(p) { try { await p; } catch (e) { return { code: e.code, data: e.data, key: e.key, message: e.message }; } throw new Error('did not fail'); }
let n = 0;
for (const v of ['EN', 'de', '', ' en', 'nl-NL', 'en_GB']) {
  const e = await fails(M['settings.set']({ path: 'language', value: v }, m));
  assert.deepEqual(e, { code: 'invalid_value', data: { path: 'language' }, key: 'settings.badValue', message: 'Deze waarde past niet bij deze instelling.' }); n++;
}
assert.equal((await fails(M['settings.set']({ path: 'language.custom', value: 'x' }, m))).code, 'invalid_value'); n++;
assert.deepEqual((await fails(M['settings.set']({ path: 'language', value: 1 }, m))).code, 'bad_value'); n++;
assert.deepEqual((await fails(M['settings.set']({ path: 'language', value: { custom: 'x' } }, m))).code, 'bad_value'); n++;
assert.deepEqual((await fails(M['settings.patch']({ patch: { language: { custom: 'x' } } }, m))), { code: 'bad_value', data: { paths: ['language'] }, key: undefined, message: 'Deze waarde past niet bij deze instelling.' }); n++;
assert.equal((await fails(M['settings.patch']({ patch: { language: 'fr', theme: { mode: 'light' } } }, m))).code, 'invalid_value'); n++;
assert.deepEqual((await fails(M['settings.patch']({ patch: { language: 'fr', theme: { mode: 3 } } }, m))).data, { paths: ['theme.mode'] }); n++;
assert.equal((await M['settings.set']({ path: 'language', value: 'en' }, m)).language, 'en'); n++;
// import: an invalid language is dropped and the current one stays; the rest of the file is taken
let s = await M['settings.import']({ json: JSON.stringify({ language: 'fr', theme: { mode: 'light' } }) }, m);
assert.equal(s.language, 'en'); assert.equal(s.theme.mode, 'light'); n++;
s = await M['settings.import']({ json: JSON.stringify({ language: 'nl', theme: { mode: 'dark' } }) }, m);
assert.equal(s.language, 'nl'); n++;
s = await M['settings.import']({ json: JSON.stringify({ language: 'EN', theme: { mode: 'light' } }) }, m);
assert.equal(s.language, 'nl'); n++;
// D-G49 (WU-FIX): a file without a language keeps the current one too (nl here, then en)
s = await M['settings.import']({ json: JSON.stringify({ theme: { mode: 'dark' } }) }, m);
assert.equal(s.language, 'nl'); assert.equal(s.theme.mode, 'dark'); n++;
assert.equal((await M['settings.set']({ path: 'language', value: 'en' }, m)).language, 'en'); n++;
s = await M['settings.import']({ json: { version: 1, taskbar: { alignment: 'left' } } }, m);
assert.equal(s.language, 'en'); assert.equal(s.taskbar.alignment, 'left'); n++;
console.log(`ok ${n}`);
