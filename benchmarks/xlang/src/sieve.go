package main
import ("fmt"; "time")
func main() {
    const n = 10000000
    t0 := time.Now()
    composite := make([]byte, n+1)
    for i := 2; i*i <= n; i++ {
        if composite[i] == 0 {
            for j := i * i; j <= n; j += i { composite[j] = 1 }
        }
    }
    count := 0
    for p := 2; p <= n; p++ {
        if composite[p] == 0 { count++ }
    }
    fmt.Printf("CHECK %d\nMS %d\n", count, time.Since(t0).Milliseconds())
}
