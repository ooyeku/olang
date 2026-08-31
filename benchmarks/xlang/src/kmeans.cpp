#include <cstdio>
#include <chrono>
#include <vector>
#include <algorithm>
int main() {
    const int n = 200000, k = 10, iters = 15;
    std::vector<double> xs(n), ys(n);
    long seed = 42;
    for (int i = 0; i < n; i++) {
        seed = seed * 48271 % 2147483647;
        xs[i] = 10.0 * seed / 2147483647.0;
        seed = seed * 48271 % 2147483647;
        ys[i] = 10.0 * seed / 2147483647.0;
    }
    auto t0 = std::chrono::steady_clock::now();
    int stride = n / k;
    std::vector<double> cx(k), cy(k);
    for (int c = 0; c < k; c++) { cx[c] = xs[c * stride]; cy[c] = ys[c * stride]; }
    std::vector<int> assign(n, 0);
    for (int it = 0; it < iters; it++) {
        for (int i = 0; i < n; i++) {
            int best = 0; double bd = 1000000.0;
            for (int c = 0; c < k; c++) {
                double dx = xs[i] - cx[c], dy = ys[i] - cy[c];
                double d = dx * dx + dy * dy;
                if (d < bd) { bd = d; best = c; }
            }
            assign[i] = best;
        }
        std::vector<double> sx(k, 0), sy(k, 0);
        std::vector<int> ct(k, 0);
        for (int i = 0; i < n; i++) {
            int c = assign[i];
            sx[c] += xs[i]; sy[c] += ys[i]; ct[c]++;
        }
        for (int c = 0; c < k; c++) if (ct[c] > 0) { cx[c] = sx[c] / ct[c]; cy[c] = sy[c] / ct[c]; }
    }
    std::vector<int> sizes(k, 0);
    for (int i = 0; i < n; i++) sizes[assign[i]]++;
    auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::steady_clock::now() - t0).count();
    printf("CHECK %d\nMS %lld\n", *std::max_element(sizes.begin(), sizes.end()), (long long)ms);
}
