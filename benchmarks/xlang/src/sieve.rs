fn main() {
    let n: usize = 10000000;
    let t0 = std::time::Instant::now();
    let mut composite = vec![0u8; n + 1];
    let mut i = 2;
    while i * i <= n {
        if composite[i] == 0 {
            let mut j = i * i;
            while j <= n {
                composite[j] = 1;
                j += i;
            }
        }
        i += 1;
    }
    let count = (2..=n).filter(|&p| composite[p] == 0).count();
    println!("CHECK {}", count);
    println!("MS {}", t0.elapsed().as_millis());
}
