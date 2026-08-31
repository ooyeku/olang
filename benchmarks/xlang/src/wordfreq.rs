use std::collections::HashMap;
fn main() {
    let n = 3000000i64;
    let t0 = std::time::Instant::now();
    let mut m: HashMap<String, i64> = HashMap::new();
    let mut seed = 42i64;
    for _ in 0..n {
        seed = seed * 48271 % 2147483647;
        *m.entry(format!("w{}", seed % 50000)).or_insert(0) += 1;
    }
    let maxf = m.values().max().copied().unwrap_or(0);
    println!("CHECK {}", m.len() as i64 * 1000000 + maxf);
    println!("MS {}", t0.elapsed().as_millis());
}
