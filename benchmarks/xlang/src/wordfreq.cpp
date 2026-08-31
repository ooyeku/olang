#include <cstdio>
#include <chrono>
#include <string>
#include <unordered_map>
int main() {
    const long n = 3000000;
    auto t0 = std::chrono::steady_clock::now();
    std::unordered_map<std::string, long> m;
    long seed = 42;
    for (long i = 0; i < n; i++) {
        seed = seed * 48271 % 2147483647;
        m["w" + std::to_string(seed % 50000)]++;
    }
    long maxf = 0;
    for (auto& kv : m) if (kv.second > maxf) maxf = kv.second;
    auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::steady_clock::now() - t0).count();
    printf("CHECK %ld\nMS %lld\n", (long)m.size() * 1000000 + maxf, (long long)ms);
}
