args <- commandArgs(trailingOnly = TRUE)
stopifnot(length(args) == 5L, args[[1]] %in% c("install", "verify"))
action <- args[[1]]
private <- normalizePath(args[[2]], winslash = "/", mustWork = TRUE)
work <- normalizePath(args[[3]], winslash = "/", mustWork = TRUE)
expected_r <- args[[4]]
expected_commit <- args[[5]]
.libPaths(c(private, .Library), include.site = FALSE)
setwd(work)
options(stringsAsFactors = FALSE, repos = c(CRAN = "https://cloud.r-project.org"), timeout = 600)
Sys.setenv(OMP_NUM_THREADS = "1", OPENBLAS_NUM_THREADS = "1", MKL_NUM_THREADS = "1")
if (as.character(getRversion()) != expected_r) {
  cat(paste0("ASTROBURST_ISSUE\tExpected R ", expected_r, " before installing packages; found ", as.character(getRversion()), "\n"))
  flush(stdout())
  quit(status = 1L)
}
install_and_verify <- function() {
  if (action == "install") {
    writeLines("void astroburst_build_probe(void) {}", "build_probe.c")
    r_binary <- file.path(R.home("bin"), if (.Platform$OS.type == "windows") "R.exe" else "R")
    status <- system2(r_binary, c("CMD", "SHLIB", "build_probe.c"), stdout = "build-tool-stdout.log", stderr = "build-tool-stderr.log")
    if (!identical(status, 0L)) stop("R build tools are unavailable. Install the Rtools version required by this R installation, then retry in a new installation folder. See build-tool-stderr.log")
    packages <- c("dplyr", "FITSio", "ggplot2", "Matrix", "matrixStats", "RANN", "Rcpp", "sf", "tibble", "tidyr", "viridis", "remotes")
    missing <- packages[!vapply(packages, requireNamespace, logical(1), quietly = TRUE)]
    package_type <- if (.Platform$OS.type == "windows") "win.binary" else "source"
    if (length(missing)) utils::install.packages(missing, lib = private, type = package_type, dependencies = c("Depends", "Imports", "LinkingTo"))
    missing <- packages[!vapply(packages, requireNamespace, logical(1), quietly = TRUE)]
    if (length(missing)) stop(paste("Required dependencies are missing:", paste(missing, collapse = ", ")))
    remotes::install_github(paste0("RafaelSdeSouza/sagui@", expected_commit), lib = private, dependencies = FALSE, upgrade = "never", build_vignettes = FALSE)
  }
  stopifnot(requireNamespace("sagui", quietly = TRUE))
  description <- utils::packageDescription("sagui")
  stopifnot(identical(description$RemoteSha, expected_commit))
  location <- normalizePath(find.package("sagui"), winslash = "/", mustWork = TRUE)
  stopifnot(startsWith(paste0(location, "/"), paste0(private, "/")))
  for (optional in c("torch", "fastcluster")) {
    if (nzchar(system.file(package = optional))) stop(paste("CPU profile excludes", optional))
  }
  nx <- 11L
  ny <- 7L
  g1 <- outer(seq_len(nx), seq_len(ny), function(x, y) exp(-((x - 4)^2 + (y - 3)^2) / 2))
  g2 <- outer(seq_len(nx), seq_len(ny), function(x, y) exp(-((x - 8)^2 + (y - 5)^2) / 2))
  cube <- array(c(10 * g1 + 3 * g2 + 0.01, 3 * g1 + 12 * g2 + 0.02, 6 * g1 + 5 * g2 + 0.03), dim = c(nx, ny, 3L))
  original <- cube
  checks <- list()
  for (kind in c("exact", "starlet", "sparse")) {
    params <- list(input = cube, Ncomp = 2L, use_starlet_mask = identical(kind, "starlet"),
      mask_pretransform = "none", cluster_pretransform = "none", starlet_J = 2L,
      starlet_scales = 1:2, denoise_k = 0.5, mode = "soft", positive_only = TRUE,
      clean_mask = FALSE, mask_mode = "na", hclust_method = "ward.D2")
    segment <- sagui::segment_regions
    if (kind == "sparse") {
      segment <- sagui::segment_regions_large
      params$knn_k <- nx * ny - 1L
      params$auto_k <- FALSE
    }
    set.seed(1L)
    first <- do.call(segment, params)
    set.seed(1L)
    second <- do.call(segment, params)
    stopifnot(identical(dim(first$cluster_map), c(nx, ny)), identical(dim(first$mask), c(nx, ny)))
    stopifnot(identical(first$cluster_map, second$cluster_map), identical(cube, original))
    labels <- first$cluster_map[is.finite(first$cluster_map)]
    stopifnot(identical(sort(unique(as.integer(labels))), 1:2), length(labels) >= 2L)
    checks[[kind]] <- data.frame(case = kind, width = nx, height = ny, regions = 2L, labelled = length(labels), deterministic = TRUE)
  }
  utils::write.table(do.call(rbind, checks), "self-tests.tsv", sep = "\t", row.names = FALSE, quote = FALSE)
  packages <- as.data.frame(utils::installed.packages(lib.loc = .libPaths()), stringsAsFactors = FALSE)
  utils::write.table(packages[, c("Package", "Version", "LibPath", "Built")], "packages.tsv", sep = "\t", row.names = FALSE, quote = TRUE)
  writeLines(capture.output(sessionInfo()), "session.txt")
  cat(paste("ASTROBURST_R", as.character(getRversion()), as.character(description$Version), as.character(description$RemoteSha), sep = "\t"), "\n", sep = "")
  cat("ASTROBURST_INSTALL_READY\n")
}
tryCatch(install_and_verify(), error = function(e) {
  cat(paste0("ASTROBURST_ERROR\t", gsub("[[:space:]]+", " ", paste(conditionMessage(e), collapse = " ")), "\n"))
  flush(stdout())
  quit(status = 1L)
})
