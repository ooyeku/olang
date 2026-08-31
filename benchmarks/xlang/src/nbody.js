const pi = 3.141592653589793, solar = 4 * pi * pi, days = 365.24, dt = 0.01;
const nSteps = 2000000;
const x = [0.0, 4.841431442464721, 8.34336671824458, 12.894369562139131, 15.379697114850917], y = [0.0, -1.1603200440274284, 4.124798564124305, -15.111151401698631, -25.919314609987964], z = [0.0, -0.10362204447112311, -0.4035234171143214, -0.22330757889265573, 0.17925877295037118];
const vx = [0.0, 0.001660076642744037, -0.002767425107268624, 0.002964601375647616, 0.0026806777249038932], vy = [0.0, 0.007699011184197404, 0.004998528012349172, 0.0023784717395948095, 0.001628241700382423], vz = [0.0, -6.90460016972063e-05, 2.3041729757376393e-05, -2.9658956854023756e-05, -9.515922545197159e-05];
const m = [1.0, 0.0009547919384243266, 0.0002858859806661308, 4.366244043351563e-05, 5.1513890204661145e-05];
for (let i = 0; i < 5; i++) { m[i] *= solar; vx[i] *= days; vy[i] *= days; vz[i] *= days; }
let px = 0, py = 0, pz = 0;
for (let i = 0; i < 5; i++) { px += vx[i] * m[i]; py += vy[i] * m[i]; pz += vz[i] * m[i]; }
vx[0] = -px / solar; vy[0] = -py / solar; vz[0] = -pz / solar;
const t0 = performance.now();
for (let s = 0; s < nSteps; s++) {
  for (let i = 0; i < 5; i++)
    for (let j = i + 1; j < 5; j++) {
      const dx = x[i] - x[j], dy = y[i] - y[j], dz = z[i] - z[j];
      const d2 = dx * dx + dy * dy + dz * dz;
      const dist = Math.sqrt(d2);
      const mag = dt / (d2 * dist);
      const mi = m[i] * mag, mj = m[j] * mag;
      vx[i] -= dx * mj; vy[i] -= dy * mj; vz[i] -= dz * mj;
      vx[j] += dx * mi; vy[j] += dy * mi; vz[j] += dz * mi;
    }
  for (let i = 0; i < 5; i++) { x[i] += dt * vx[i]; y[i] += dt * vy[i]; z[i] += dt * vz[i]; }
}
let e = 0;
for (let i = 0; i < 5; i++) {
  e += 0.5 * m[i] * (vx[i] * vx[i] + vy[i] * vy[i] + vz[i] * vz[i]);
  for (let j = i + 1; j < 5; j++) {
    const dx = x[i] - x[j], dy = y[i] - y[j], dz = z[i] - z[j];
    e -= m[i] * m[j] / Math.sqrt(dx * dx + dy * dy + dz * dz);
  }
}
console.log(`CHECK ${Math.round(e * 1000000)}`);
console.log(`MS ${Math.round(performance.now() - t0)}`);
