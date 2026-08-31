package main
import ("fmt"; "strconv"; "strings"; "time")
func main() {
    const n = 2000000
    t0 := time.Now()
    var b strings.Builder
    for i := 0; i < n; i++ { b.WriteString(strconv.Itoa(i % 1000)) }
    fmt.Printf("CHECK %d\nMS %d\n", b.Len(), time.Since(t0).Milliseconds())
}
