import time
n = 3000000
t0 = time.perf_counter()
m = {}
seed = 42
for _ in range(n):
    seed = seed * 48271 % 2147483647
    word = "w" + str(seed % 50000)
    m[word] = m.get(word, 0) + 1
maxf = max(m.values())
print(f"CHECK {len(m) * 1000000 + maxf}")
print(f"MS {int((time.perf_counter() - t0) * 1000)}")
