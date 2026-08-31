import time
n = 10000000
t0 = time.perf_counter()
composite = bytearray(n + 1)
i = 2
while i * i <= n:
    if not composite[i]:
        j = i * i
        while j <= n:
            composite[j] = 1
            j += i
    i += 1
count = 0
for p in range(2, n + 1):
    if not composite[p]:
        count += 1
print(f"CHECK {count}")
print(f"MS {int((time.perf_counter() - t0) * 1000)}")
