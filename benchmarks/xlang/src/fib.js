function fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
const t0 = performance.now();
const r = fib(32);
console.log(`CHECK ${r}`);
console.log(`MS ${Math.round(performance.now() - t0)}`);
