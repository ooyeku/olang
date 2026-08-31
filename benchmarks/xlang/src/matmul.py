import time
n = 300
a = [[((i * j) % 100) * 0.01 for j in range(n)] for i in range(n)]
b = [[((i + j) % 100) * 0.01 for j in range(n)] for i in range(n)]
t0 = time.perf_counter()
c = [[0.0] * n for _ in range(n)]
for i in range(n):
    ai = a[i]; ci = c[i]
    for j in range(n):
        s = 0.0
        for k in range(n):
            s += ai[k] * b[k][j]
        ci[j] = s
t = 0.0
for i in range(n):
    for j in range(n):
        t += c[i][j]
print(f"CHECK {round(t)}")
print(f"MS {int((time.perf_counter() - t0) * 1000)}")
