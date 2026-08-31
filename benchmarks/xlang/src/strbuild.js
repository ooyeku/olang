const n = 2000000;
const t0 = performance.now();
let s = "";
for (let i = 0; i < n; i++) s += String(i % 1000);
console.log(`CHECK ${s.length}`);
console.log(`MS ${Math.round(performance.now() - t0)}`);
