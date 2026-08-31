fn main() {
    let pi = 3.141592653589793f64;
    let solar = 4.0 * pi * pi;
    let days = 365.24f64;
    let dt = 0.01f64;
    let n_steps = 2000000;
    let mut x: [f64; 5] = [0.0, 4.841431442464721, 8.34336671824458, 12.894369562139131, 15.379697114850917];
    let mut y: [f64; 5] = [0.0, -1.1603200440274284, 4.124798564124305, -15.111151401698631, -25.919314609987964];
    let mut z: [f64; 5] = [0.0, -0.10362204447112311, -0.4035234171143214, -0.22330757889265573, 0.17925877295037118];
    let mut vx: [f64; 5] = [0.0, 0.001660076642744037, -0.002767425107268624, 0.002964601375647616, 0.0026806777249038932];
    let mut vy: [f64; 5] = [0.0, 0.007699011184197404, 0.004998528012349172, 0.0023784717395948095, 0.001628241700382423];
    let mut vz: [f64; 5] = [0.0, -6.90460016972063e-05, 2.3041729757376393e-05, -2.9658956854023756e-05, -9.515922545197159e-05];
    let mut m: [f64; 5] = [1.0, 0.0009547919384243266, 0.0002858859806661308, 4.366244043351563e-05, 5.1513890204661145e-05];
    for i in 0..5 { m[i] *= solar; vx[i] *= days; vy[i] *= days; vz[i] *= days; }
    let (mut px, mut py, mut pz) = (0.0, 0.0, 0.0);
    for i in 0..5 { px += vx[i] * m[i]; py += vy[i] * m[i]; pz += vz[i] * m[i]; }
    vx[0] = -px / solar; vy[0] = -py / solar; vz[0] = -pz / solar;
    let t0 = std::time::Instant::now();
    for _ in 0..n_steps {
        for i in 0..5 {
            for j in (i + 1)..5 {
                let dx = x[i] - x[j]; let dy = y[i] - y[j]; let dz = z[i] - z[j];
                let d2 = dx * dx + dy * dy + dz * dz;
                let dist = d2.sqrt();
                let mag = dt / (d2 * dist);
                let mi = m[i] * mag; let mj = m[j] * mag;
                vx[i] -= dx * mj; vy[i] -= dy * mj; vz[i] -= dz * mj;
                vx[j] += dx * mi; vy[j] += dy * mi; vz[j] += dz * mi;
            }
        }
        for i in 0..5 { x[i] += dt * vx[i]; y[i] += dt * vy[i]; z[i] += dt * vz[i]; }
    }
    let mut e = 0.0;
    for i in 0..5 {
        let sq = vx[i] * vx[i] + vy[i] * vy[i] + vz[i] * vz[i];
        e += 0.5 * m[i] * sq;
        for j in (i + 1)..5 {
            let dx = x[i] - x[j]; let dy = y[i] - y[j]; let dz = z[i] - z[j];
            let d2 = dx * dx + dy * dy + dz * dz;
            e -= m[i] * m[j] / d2.sqrt();
        }
    }
    println!("CHECK {}", (e * 1000000.0).round() as i64);
    println!("MS {}", t0.elapsed().as_millis());
}
