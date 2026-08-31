#include <cstdio>
#include <cmath>
#include <chrono>
#include <vector>
int main() {
    const int n = 300;
    std::vector<std::vector<double>> a(n, std::vector<double>(n)), b = a, c = a;
    for (int i = 0; i < n; i++)
        for (int j = 0; j < n; j++) {
            a[i][j] = ((i * j) % 100) * 0.01;
            b[i][j] = ((i + j) % 100) * 0.01;
        }
    auto t0 = std::chrono::steady_clock::now();
    for (int i = 0; i < n; i++)
        for (int j = 0; j < n; j++) {
            double s = 0;
            for (int k = 0; k < n; k++) s += a[i][k] * b[k][j];
            c[i][j] = s;
        }
    double t = 0;
    for (int i = 0; i < n; i++) for (int j = 0; j < n; j++) t += c[i][j];
    auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::steady_clock::now() - t0).count();
    printf("CHECK %lld\nMS %lld\n", (long long)llround(t), (long long)ms);
}
