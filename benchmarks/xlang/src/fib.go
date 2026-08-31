package main
import ("fmt"; "time")
func fib(n int64) int64 { if n < 2 { return n }; return fib(n-1) + fib(n-2) }
func main() {
    t0 := time.Now()
    r := fib(32)
    fmt.Printf("CHECK %d\nMS %d\n", r, time.Since(t0).Milliseconds())
}
