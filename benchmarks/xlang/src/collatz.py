import time
def steps(n):
    c = 0
    while n != 1:
        n = n // 2 if n % 2 == 0 else 3 * n + 1
        c += 1
    return c
t0 = time.perf_counter()
total = 0
for i in range(1, 300001):
    total += steps(i)
print(f"CHECK {total}")
print(f"MS {int((time.perf_counter() - t0) * 1000)}")
