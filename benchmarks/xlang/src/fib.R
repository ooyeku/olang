fib <- function(n) if (n < 2) n else fib(n - 1) + fib(n - 2)
t0 <- proc.time()[["elapsed"]]
r <- fib(32)
ms <- as.integer((proc.time()[["elapsed"]] - t0) * 1000)
cat("CHECK", r, "\n")
cat("MS", ms, "\n")
