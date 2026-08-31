fn steps(mut n: i64) -> i64 {
    let mut c = 0;
    while n != 1 {
        n = if n % 2 == 0 { n / 2 } else { 3 * n + 1 };
        c += 1;
    }
    c
}
fn main() {
    let t0 = std::time::Instant::now();
    let mut total: i64 = 0;
    for i in 1..=300000 { total += steps(i); }
    println!("CHECK {}", total);
    println!("MS {}", t0.elapsed().as_millis());
}
