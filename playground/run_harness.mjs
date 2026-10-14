// `node run_harness.mjs <wasm> <program.ol>`: run a program in the browser
// runtime the way the playground does (olang_run, no page: every dom host
// import answers nothing) and print its output, then its error if any.
// tests/browser_agreement_test.rs runs a program here and natively and
// compares the two.
import { readFile } from "node:fs/promises";

const bytes = await readFile(process.argv[2]);
const source = await readFile(process.argv[3]);
let ex;
const env = new Proxy({
  host_now_ms: () => performance.now(),
  host_epoch_ms: () => Date.now(),
  host_random_bytes: (ptr, len) =>
    crypto.getRandomValues(new Uint8Array(ex.memory.buffer, ptr >>> 0, len >>> 0)),
}, { get: (t, k) => (k in t ? t[k] : () => 0) });
const { instance } = await WebAssembly.instantiate(bytes, { env });
ex = instance.exports;
const ptr = ex.olang_alloc(source.length) >>> 0;
new Uint8Array(ex.memory.buffer, ptr, source.length).set(source);
const res = ex.olang_run(ptr, source.length) >>> 0;
ex.olang_dealloc(ptr, source.length);
const len = new DataView(ex.memory.buffer).getUint32(res, true);
const out = JSON.parse(new TextDecoder().decode(new Uint8Array(ex.memory.buffer, res + 4, len)));
process.stdout.write(out.output);
if (out.error) process.stdout.write("ERROR: " + out.error + "\n");
