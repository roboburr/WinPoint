// The mock router (vendor/ui/js/mock/index.js) as the UI sees it (R3-UI fix round, notes/claude/round3/r3-ui.md):
// a thrown mock error keeps code, message, key, params and data (contracts C2, D-G41); the router's own text follows
// the UI language; ?mockRestartAll=live makes shell.restartAll answer as a host outside test mode (dryRun:false).
// No network, no bus: the mocks run in this process only.
//   node --experimental-default-type=module tests/i18n/mock_index.test.mjs      (from the project root)
import assert from 'node:assert/strict';

globalThis.location = new URL('http://127.0.0.1/ui/index.html?mock=1&lang=en');
globalThis.document ??= { hidden: true };            // mock/windows.js' timer looks at it
const { createMock } = await import(new URL('../../vendor/ui/js/mock/index.js', import.meta.url));
const warn = console.warn;
console.warn = () => {};                              // the router names an unknown method in the console
let n = 0;
async function fails(mock, method, args) {
  try { await mock.call(method, args); } catch (e) {
    return { code: e.code, key: e.key, params: e.params, data: e.data, message: e.message };
  }
  throw new Error(`${method} did not fail`);
}

const mock = await createMock({ dispatch() {} });
// D-G31's refusal through the router: the host's key and C2's data survive
assert.deepEqual(await fails(mock, 'settings.set', { path: 'language', value: 'fr' }),
  { code: 'invalid_value', key: 'settings.badValue', params: {}, data: { path: 'language' },
    message: 'Deze waarde past niet bij deze instelling.' }); n++;
assert.deepEqual(await fails(mock, 'settings.patch', { patch: { language: 'fr' } }),
  { code: 'invalid_value', key: 'settings.badValue', params: {}, data: { paths: ['language'] },
    message: 'Deze waarde past niet bij deze instelling.' }); n++;
// an error without key or data stays without them
const e = await fails(mock, 'settings.set', { path: 'language', value: 1 });
assert.equal(e.code, 'bad_value'); assert.equal(e.key, undefined); n++;
// the router's own text in the UI language (lang=en)
assert.deepEqual(await fails(mock, 'no.such', {}),
  { code: 'unknown_method', key: undefined, params: undefined, data: undefined, message: 'Unknown function: no.such' }); n++;
// shell.restartAll: a dry run by default, a real answer with ?mockRestartAll=live (also for the pending answer)
const first = await mock.call('shell.restartAll', { reason: 'libraries' });
assert.equal(first.ok, true); assert.equal(first.dryRun, true); assert.equal(first.pending, undefined); n++;
globalThis.location = new URL('http://127.0.0.1/ui/index.html?mock=1&lang=en&mockRestartAll=live');
const live = await createMock({ dispatch() {} });
const again = await live.call('shell.restartAll', { reason: 'libraries' });
assert.deepEqual([again.ok, again.pending, again.dryRun, again.nonce], [true, true, false, first.nonce]); n++;
assert.equal((await fails(live, 'shell.restartAll', { reason: 'other' })).code, 'bad_args'); n++;

console.warn = warn;
console.log(`ok ${n}`);
process.exit(0);                                      // the mocks' own timers (host restart, windows) end here
