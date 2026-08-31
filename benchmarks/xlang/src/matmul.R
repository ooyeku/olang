n <- 300
a <- matrix(0, n, n); b <- matrix(0, n, n); cm <- matrix(0, n, n)
for (i in 1:n) for (j in 1:n) {
  a[i, j] <- (((i - 1) * (j - 1)) %% 100) * 0.01
  b[i, j] <- (((i - 1) + (j - 1)) %% 100) * 0.01
}
t0 <- proc.time()[["elapsed"]]
for (i in 1:n) {
  for (j in 1:n) {
    s <- 0
    for (k in 1:n) s <- s + a[i, k] * b[k, j]
    cm[i, j] <- s
  }
}
t <- 0
for (i in 1:n) for (j in 1:n) t <- t + cm[i, j]
cat("CHECK", format(round(t), scientific = FALSE), "\n")
cat("MS", as.integer((proc.time()[["elapsed"]] - t0) * 1000), "\n")
