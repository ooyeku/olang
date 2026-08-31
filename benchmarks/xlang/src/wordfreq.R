n <- 3000000
t0 <- proc.time()[["elapsed"]]
m <- new.env(hash = TRUE)
seed <- 42
for (i in 1:n) {
  seed <- (seed * 48271) %% 2147483647
  word <- paste0("w", seed %% 50000)
  cur <- m[[word]]
  if (is.null(cur)) m[[word]] <- 1L else m[[word]] <- cur + 1L
}
maxf <- 0L
distinct <- 0L
for (k in ls(m)) {
  distinct <- distinct + 1L
  v <- m[[k]]
  if (v > maxf) maxf <- v
}
cat("CHECK", format(distinct * 1000000 + maxf, scientific = FALSE), "\n")
cat("MS", as.integer((proc.time()[["elapsed"]] - t0) * 1000), "\n")
