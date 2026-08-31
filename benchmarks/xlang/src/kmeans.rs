fn main() {
    let n = 200000usize;
    let k = 10usize;
    let iters = 15;
    let mut xs = vec![0.0f64; n];
    let mut ys = vec![0.0f64; n];
    let mut seed = 42i64;
    for i in 0..n {
        seed = seed * 48271 % 2147483647;
        xs[i] = 10.0 * seed as f64 / 2147483647.0;
        seed = seed * 48271 % 2147483647;
        ys[i] = 10.0 * seed as f64 / 2147483647.0;
    }
    let t0 = std::time::Instant::now();
    let stride = n / k;
    let mut cx: Vec<f64> = (0..k).map(|c| xs[c * stride]).collect();
    let mut cy: Vec<f64> = (0..k).map(|c| ys[c * stride]).collect();
    let mut assign = vec![0usize; n];
    for _ in 0..iters {
        for i in 0..n {
            let mut best = 0;
            let mut bd = 1000000.0;
            for c in 0..k {
                let dx = xs[i] - cx[c];
                let dy = ys[i] - cy[c];
                let d = dx * dx + dy * dy;
                if d < bd { bd = d; best = c; }
            }
            assign[i] = best;
        }
        let mut sx = vec![0.0; k];
        let mut sy = vec![0.0; k];
        let mut ct = vec![0i64; k];
        for i in 0..n {
            let c = assign[i];
            sx[c] += xs[i]; sy[c] += ys[i]; ct[c] += 1;
        }
        for c in 0..k {
            if ct[c] > 0 { cx[c] = sx[c] / ct[c] as f64; cy[c] = sy[c] / ct[c] as f64; }
        }
    }
    let mut sizes = vec![0i64; k];
    for i in 0..n { sizes[assign[i]] += 1; }
    println!("CHECK {}", sizes.iter().max().unwrap());
    println!("MS {}", t0.elapsed().as_millis());
}
