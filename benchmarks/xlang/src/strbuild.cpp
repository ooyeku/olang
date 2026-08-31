#include <cstdio>
#include <chrono>
#include <string>
int main() {
    const int n = 2000000;
    auto t0 = std::chrono::steady_clock::now();
    std::string s;
    for (int i = 0; i < n; i++) s += std::to_string(i % 1000);
    auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::steady_clock::now() - t0).count();
    printf("CHECK %zu\nMS %lld\n", s.size(), (long long)ms);
}
