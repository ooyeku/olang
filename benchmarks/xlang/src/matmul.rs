fn main() {
    let n = 300;
    let a: Vec<Vec<f64>> = (0..n).map(|i| (0..n).map(|j| ((i * j) % 100) as f64 * 0.01).collect()).collect();
    let b: Vec<Vec<f64>> = (0..n).map(|i| (0..n).map(|j| ((i + j) % 100) as f64 * 0.01).collect()).collect();
    let t0 = std::time::Instant::now();
    let mut c = vec![vec![0.0f64; n]; n];
    for i in 0..n {
        for j in 0..n {
            let mut s = 0.0;
            for k in 0..n {
                s += a[i][k] * b[k][j];
            }
            c[i][j] = s;
        }
    }
    let mut t = 0.0;
    for i in 0..n { for j in 0..n { t += c[i][j]; } }
    println!("CHECK {}", t.round() as i64);
    println!("MS {}", t0.elapsed().as_millis());
}
