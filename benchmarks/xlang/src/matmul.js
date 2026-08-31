const n = 300;
const a = [], b = [];
for (let i = 0; i < n; i++) {
  a.push(Array.from({length: n}, (_, j) => ((i * j) % 100) * 0.01));
  b.push(Array.from({length: n}, (_, j) => ((i + j) % 100) * 0.01));
}
const t0 = performance.now();
const c = [];
for (let i = 0; i < n; i++) {
  const row = new Array(n).fill(0);
  for (let j = 0; j < n; j++) {
    let s = 0;
    for (let k = 0; k < n; k++) s += a[i][k] * b[k][j];
    row[j] = s;
  }
  c.push(row);
}
let t = 0;
for (let i = 0; i < n; i++) for (let j = 0; j < n; j++) t += c[i][j];
console.log(`CHECK ${Math.round(t)}`);
console.log(`MS ${Math.round(performance.now() - t0)}`);
