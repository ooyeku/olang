fn main() {
    let n = 2000000;
    let t0 = std::time::Instant::now();
    let mut s = String::new();
    for i in 0..n {
        s.push_str(&(i % 1000).to_string());
    }
    println!("CHECK {}", s.len());
    println!("MS {}", t0.elapsed().as_millis());
}
