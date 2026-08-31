import time
def fib(n):
    return n if n < 2 else fib(n - 1) + fib(n - 2)
t0 = time.perf_counter()
r = fib(32)
ms = int((time.perf_counter() - t0) * 1000)
print(f"CHECK {r}")
print(f"MS {ms}")
