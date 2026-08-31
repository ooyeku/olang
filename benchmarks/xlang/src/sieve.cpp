#include <cstdio>
#include <chrono>
#include <vector>
int main() {
    const long n = 10000000;
    auto t0 = std::chrono::steady_clock::now();
    std::vector<unsigned char> composite(n + 1, 0);
    for (long i = 2; i * i <= n; i++)
        if (!composite[i])
            for (long j = i * i; j <= n; j += i) composite[j] = 1;
    long count = 0;
    for (long p = 2; p <= n; p++) if (!composite[p]) count++;
    auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::steady_clock::now() - t0).count();
    printf("CHECK %ld\nMS %lld\n", count, (long long)ms);
}
