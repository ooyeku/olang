function steps(n) {
  let c = 0;
  while (n !== 1) { n = n % 2 === 0 ? n / 2 : 3 * n + 1; c++; }
  return c;
}
const t0 = performance.now();
let total = 0;
for (let i = 1; i <= 300000; i++) total += steps(i);
console.log(`CHECK ${total}`);
console.log(`MS ${Math.round(performance.now() - t0)}`);
