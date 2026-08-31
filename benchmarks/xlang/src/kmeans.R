n <- 200000; k <- 10; iters <- 15
xs <- numeric(n); ys <- numeric(n)
seed <- 42
for (i in 1:n) {
  seed <- (seed * 48271) %% 2147483647
  xs[i] <- 10.0 * seed / 2147483647.0
  seed <- (seed * 48271) %% 2147483647
  ys[i] <- 10.0 * seed / 2147483647.0
}
t0 <- proc.time()[["elapsed"]]
stride <- n %/% k
cx <- numeric(k); cy <- numeric(k)
for (c in 1:k) { cx[c] <- xs[(c - 1) * stride + 1]; cy[c] <- ys[(c - 1) * stride + 1] }
asgn <- integer(n)
for (it in 1:iters) {
  for (i in 1:n) {
    best <- 1; bd <- 1000000.0
    for (c in 1:k) {
      dx <- xs[i] - cx[c]; dy <- ys[i] - cy[c]
      d <- dx * dx + dy * dy
      if (d < bd) { bd <- d; best <- c }
    }
    asgn[i] <- best
  }
  sx <- numeric(k); sy <- numeric(k); ct <- integer(k)
  for (i in 1:n) {
    c <- asgn[i]
    sx[c] <- sx[c] + xs[i]; sy[c] <- sy[c] + ys[i]; ct[c] <- ct[c] + 1L
  }
  for (c in 1:k) if (ct[c] > 0) { cx[c] <- sx[c] / ct[c]; cy[c] <- sy[c] / ct[c] }
}
sizes <- integer(k)
for (i in 1:n) sizes[asgn[i]] <- sizes[asgn[i]] + 1L
cat("CHECK", max(sizes), "\n")
cat("MS", as.integer((proc.time()[["elapsed"]] - t0) * 1000), "\n")
