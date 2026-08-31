n <- 2000000
t0 <- proc.time()[["elapsed"]]
parts <- character(n)
for (i in 0:(n - 1)) parts[i + 1] <- as.character(i %% 1000)
s <- paste(parts, collapse = "")
cat("CHECK", nchar(s), "\n")
cat("MS", as.integer((proc.time()[["elapsed"]] - t0) * 1000), "\n")
