package main
import ("fmt"; "strconv"; "time")
func main() {
    const n = 3000000
    t0 := time.Now()
    m := make(map[string]int64)
    var seed int64 = 42
    for i := 0; i < n; i++ {
        seed = seed * 48271 % 2147483647
        m["w"+strconv.FormatInt(seed%50000, 10)]++
    }
    var maxf int64 = 0
    for _, v := range m {
        if v > maxf { maxf = v }
    }
    fmt.Printf("CHECK %d\nMS %d\n", int64(len(m))*1000000+maxf, time.Since(t0).Milliseconds())
}
