#include <cstdio>
#include <chrono>
long steps(long n) {
    long c = 0;
    while (n != 1) { n = n % 2 == 0 ? n / 2 : 3 * n + 1; c++; }
    return c;
}
int main() {
    auto t0 = std::chrono::steady_clock::now();
    long total = 0;
    for (long i = 1; i <= 300000; i++) total += steps(i);
    auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::steady_clock::now() - t0).count();
    printf("CHECK %ld\nMS %lld\n", total, (long long)ms);
}
