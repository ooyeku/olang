#include <cstdio>
#include <chrono>
long fib(long n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
int main() {
    auto t0 = std::chrono::steady_clock::now();
    long r = fib(32);
    auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::steady_clock::now() - t0).count();
    printf("CHECK %ld\nMS %lld\n", r, (long long)ms);
}
