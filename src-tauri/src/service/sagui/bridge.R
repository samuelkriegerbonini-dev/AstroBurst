args <- commandArgs(trailingOnly = TRUE)
stopifnot(length(args) >= 3L)
action <- args[[1]]
private <- normalizePath(args[[2]], winslash = "/", mustWork = TRUE)
work <- normalizePath(args[[3]], winslash = "/", mustWork = TRUE)
.libPaths(c(private, .Library), include.site = FALSE)
setwd(work)
options(stringsAsFactors = FALSE)
Sys.setenv(OMP_NUM_THREADS = "1", OPENBLAS_NUM_THREADS = "1", MKL_NUM_THREADS = "1")
expected <- "56fd3ea776d7c00e056368498da9412bd58b84bb"
issues <- character()
rv <- as.character(getRversion())
if (rv != "4.6.2") issues <- c(issues, paste("Expected R 4.6.2; found", rv))
found <- requireNamespace("sagui", quietly = TRUE)
version <- ""
commit <- ""
if (found) {
  desc <- utils::packageDescription("sagui")
  version <- as.character(desc$Version)
  commit <- if (is.null(desc$RemoteSha)) "" else as.character(desc$RemoteSha)
  if (commit != expected) issues <- c(issues, "SAGUI does not match the pinned upstream commit")
  location <- normalizePath(find.package("sagui"), winslash = "/", mustWork = TRUE)
  if (!startsWith(paste0(location, "/"), paste0(private, "/"))) issues <- c(issues, "SAGUI must be in the private library")
} else {
  issues <- c(issues, "SAGUI is missing or a dependency could not be loaded")
}
for (optional in c("torch", "fastcluster")) {
  if (nzchar(system.file(package = optional))) issues <- c(issues, paste("CPU profile excludes", optional))
}
cat(paste("ASTROBURST_R", rv, version, commit, sep = "\t"), "\n", sep = "")
for (issue in issues) cat(paste0("ASTROBURST_ISSUE\t", issue, "\n"))
packages <- as.data.frame(utils::installed.packages(lib.loc = .libPaths()), stringsAsFactors = FALSE)
utils::write.table(packages[, c("Package", "Version", "LibPath", "Built")], "packages.tsv", sep = "\t", row.names = FALSE, quote = TRUE)
if (action == "inspect") quit(status = 0L)
if (length(issues)) stop(paste(issues, collapse = "; "))
stopifnot(action == "run", length(args) == 13L)
nx <- as.integer(args[[4]])
ny <- as.integer(args[[5]])
nb <- as.integer(args[[6]])
nregions <- as.integer(args[[7]])
starlet <- identical(args[[8]], "true")
scales <- as.integer(strsplit(args[[9]], ",", fixed = TRUE)[[1]])
threshold <- as.numeric(args[[10]])
method <- args[[11]]
knn <- as.integer(args[[12]])
seed <- as.integer(args[[13]])
set.seed(seed)
values <- readBin("science.bin", what = "double", n = nx * ny * nb, size = 8L, endian = "little")
stopifnot(length(values) == nx * ny * nb)
cube <- array(values, dim = c(nx, ny, nb))
params <- list(input = cube, Ncomp = nregions, use_starlet_mask = starlet,
  mask_pretransform = "none", cluster_pretransform = "none", starlet_J = max(scales),
  starlet_scales = scales, denoise_k = threshold, mode = "soft", positive_only = TRUE,
  clean_mask = FALSE, mask_mode = "na", hclust_method = "ward.D2")
if (method == "ward") {
  result <- do.call(sagui::segment_regions, params)
} else if (method == "sparse") {
  params$knn_k <- knn
  params$auto_k <- FALSE
  result <- do.call(sagui::segment_regions_large, params)
} else {
  stop("Unknown segmentation method")
}
stopifnot(identical(dim(result$cluster_map), c(nx, ny)))
labels <- as.integer(result$cluster_map)
labels[is.na(labels)] <- 0L
support <- as.integer(result$mask)
support[is.na(support)] <- 0L
writeBin(labels, "labels.bin", size = 4L, endian = "little")
writeBin(support, "support.bin", size = 4L, endian = "little")
writeLines(capture.output(sessionInfo()), "session.txt")
