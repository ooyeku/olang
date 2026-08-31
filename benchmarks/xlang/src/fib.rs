fn fib(n: i64) -> i64 { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }
fn main() {
    let t0 = std::time::Instant::now();
    let r = fib(32);
    println!("CHECK {}", r);
    println!("MS {}", t0.elapsed().as_millis());
}
