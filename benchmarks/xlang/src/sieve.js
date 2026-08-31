const n = 10000000;
const t0 = performance.now();
const composite = new Uint8Array(n + 1);
let i = 2;
while (i * i <= n) {
  if (!composite[i]) {
    for (let j = i * i; j <= n; j += i) composite[j] = 1;
  }
  i++;
}
let count = 0;
for (let p = 2; p <= n; p++) if (!composite[p]) count++;
console.log(`CHECK ${count}`);
console.log(`MS ${Math.round(performance.now() - t0)}`);
