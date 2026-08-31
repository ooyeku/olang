const n = 3000000;
const t0 = performance.now();
const m = new Map();
let seed = 42;
for (let i = 0; i < n; i++) {
  seed = seed * 48271 % 2147483647;
  const word = "w" + (seed % 50000);
  m.set(word, (m.get(word) || 0) + 1);
}
let maxf = 0;
for (const v of m.values()) if (v > maxf) maxf = v;
console.log(`CHECK ${m.size * 1000000 + maxf}`);
console.log(`MS ${Math.round(performance.now() - t0)}`);
