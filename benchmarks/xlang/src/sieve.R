n <- 10000000
t0 <- proc.time()[["elapsed"]]
composite <- integer(n + 1)
i <- 2
while (i * i <= n) {
  if (composite[i] == 0) {
    j <- i * i
    while (j <= n) {
      composite[j] <- 1L
      j <- j + i
    }
  }
  i <- i + 1
}
count <- 0
for (p in 2:n) if (composite[p] == 0) count <- count + 1
cat("CHECK", count, "\n")
cat("MS", as.integer((proc.time()[["elapsed"]] - t0) * 1000), "\n")
