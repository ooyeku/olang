steps <- function(n) {
  c <- 0L
  while (n != 1) {
    if (n %% 2 == 0) n <- n %/% 2 else n <- 3 * n + 1
    c <- c + 1L
  }
  c
}
t0 <- proc.time()[["elapsed"]]
total <- 0
for (i in 1:300000) total <- total + steps(i)
cat("CHECK", format(total, scientific = FALSE), "\n")
cat("MS", as.integer((proc.time()[["elapsed"]] - t0) * 1000), "\n")
