const n = 200000, k = 10, iters = 15;
const xs = [], ys = [];
let seed = 42;
for (let i = 0; i < n; i++) {
  seed = seed * 48271 % 2147483647;
  xs.push(10 * seed / 2147483647);
  seed = seed * 48271 % 2147483647;
  ys.push(10 * seed / 2147483647);
}
const t0 = performance.now();
const stride = Math.floor(n / k);
const cx = [], cy = [];
for (let c = 0; c < k; c++) { cx.push(xs[c * stride]); cy.push(ys[c * stride]); }
const assign = new Array(n).fill(0);
for (let it = 0; it < iters; it++) {
  for (let i = 0; i < n; i++) {
    let best = 0, bd = 1000000;
    for (let c = 0; c < k; c++) {
      const dx = xs[i] - cx[c], dy = ys[i] - cy[c];
      const d = dx * dx + dy * dy;
      if (d < bd) { bd = d; best = c; }
    }
    assign[i] = best;
  }
  const sx = new Array(k).fill(0), sy = new Array(k).fill(0), ct = new Array(k).fill(0);
  for (let i = 0; i < n; i++) {
    const c = assign[i];
    sx[c] += xs[i]; sy[c] += ys[i]; ct[c]++;
  }
  for (let c = 0; c < k; c++) if (ct[c] > 0) { cx[c] = sx[c] / ct[c]; cy[c] = sy[c] / ct[c]; }
}
const sizes = new Array(k).fill(0);
for (let i = 0; i < n; i++) sizes[assign[i]]++;
console.log(`CHECK ${Math.max(...sizes)}`);
console.log(`MS ${Math.round(performance.now() - t0)}`);
