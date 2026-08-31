import time
n = 2000000
t0 = time.perf_counter()
s = ""
for i in range(n):
    s += str(i % 1000)
print(f"CHECK {len(s)}")
print(f"MS {int((time.perf_counter() - t0) * 1000)}")
