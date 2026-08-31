import time
n = 200000; k = 10; iters = 15
xs = []; ys = []
seed = 42
for _ in range(n):
    seed = seed * 48271 % 2147483647
    xs.append(10.0 * seed / 2147483647.0)
    seed = seed * 48271 % 2147483647
    ys.append(10.0 * seed / 2147483647.0)
t0 = time.perf_counter()
stride = n // k
cx = [xs[c * stride] for c in range(k)]
cy = [ys[c * stride] for c in range(k)]
assign = [0] * n
for _ in range(iters):
    for i in range(n):
        best = 0; bd = 1000000.0
        xi = xs[i]; yi = ys[i]
        for c in range(k):
            dx = xi - cx[c]; dy = yi - cy[c]
            d = dx * dx + dy * dy
            if d < bd:
                bd = d; best = c
        assign[i] = best
    sx = [0.0] * k; sy = [0.0] * k; ct = [0] * k
    for i in range(n):
        c = assign[i]
        sx[c] += xs[i]; sy[c] += ys[i]; ct[c] += 1
    for c in range(k):
        if ct[c] > 0:
            cx[c] = sx[c] / ct[c]; cy[c] = sy[c] / ct[c]
sizes = [0] * k
for i in range(n):
    sizes[assign[i]] += 1
print(f"CHECK {max(sizes)}")
print(f"MS {int((time.perf_counter() - t0) * 1000)}")
