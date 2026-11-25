args <- commandArgs(trailingOnly = TRUE)
stopifnot(length(args) >= 3L)
action <- args[[1]]
private <- normalizePath(args[[2]], winslash = "/", mustWork = TRUE)
work <- normalizePath(args[[3]], winslash = "/", mustWork = TRUE)
.libPaths(c(private, .Library), include.site = FALSE)
setwd(work)
options(stringsAsFactors = FALSE)
Sys.setenv(OMP_NUM_THREADS = "1", OPENBLAS_NUM_THREADS = "1", MKL_NUM_THREADS = "1")
expected_r <- if (length(args) >= 5L) args[[4]] else ""
expected_commit <- if (length(args) >= 5L) args[[5]] else ""
one_line <- function(text) gsub("[[:space:]]+", " ", paste(text, collapse = " "))
emit <- function(marker, text) {
  cat(paste0(marker, "\t", one_line(text), "\n"))
  flush(stdout())
}
fail <- function(text) {
  emit("ASTROBURST_ERROR", text)
  quit(status = 1L)
}
refuse <- function(text) {
  emit("ASTROBURST_REFUSED", text)
  quit(status = 3L)
}
issues <- character()
rv <- as.character(getRversion())
if (nzchar(expected_r) && rv != expected_r) issues <- c(issues, paste0("Expected R ", expected_r, "; found ", rv))
found <- requireNamespace("sagui", quietly = TRUE)
version <- ""
commit <- ""
if (found) {
  desc <- utils::packageDescription("sagui")
  version <- as.character(desc$Version)
  commit <- if (is.null(desc$RemoteSha)) "" else as.character(desc$RemoteSha)
  if (nzchar(expected_commit) && commit != expected_commit) issues <- c(issues, "SAGUI does not match the pinned upstream commit")
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
if (length(issues)) fail(paste(issues, collapse = "; "))
stopifnot(action == "run", length(args) == 18L)
nx <- as.integer(args[[6]])
ny <- as.integer(args[[7]])
nb <- as.integer(args[[8]])
nregions <- as.integer(args[[9]])
starlet <- identical(args[[10]], "true")
scales <- as.integer(strsplit(args[[11]], ",", fixed = TRUE)[[1]])
threshold <- as.numeric(args[[12]])
method <- args[[13]]
knn <- as.integer(args[[14]])
seed <- as.integer(args[[15]])
commit_available <- suppressWarnings(as.numeric(args[[16]]))
physical_available <- suppressWarnings(as.numeric(args[[17]]))
sed_scaling <- args[[18]]
stopifnot(method %in% c("ward", "sparse"), sed_scaling %in% c("median_centre", "median_mad"))
values <- readBin("science.bin", what = "double", n = nx * ny * nb, size = 8L, endian = "little")
stopifnot(length(values) == nx * ny * nb)
cube <- array(values, dim = c(nx, ny, nb))
eligible <- apply(is.finite(cube), c(1, 2), all)
hclust_limit <- 65536L
precheck_mask <- tryCatch(if (starlet) {
  sagui:::build_starlet_mask(list(imDat = cube, hdr = NULL, axDat = NULL), collapse_fn = sagui::collapse_white_light,
    pretransform = "none", starlet_J = max(scales), starlet_scales = scales, include_coarse = FALSE,
    denoise_k = threshold, mode = "soft", positive_only = TRUE, clean_mask = FALSE, min_mask_area = 1L,
    close_size = 1L, open_size = 1L, keep_largest = FALSE)$mask
} else {
  is.finite(sagui::collapse_white_light(cube))
}, error = function(e) fail(conditionMessage(e)))
clustered <- sum(precheck_mask & eligible, na.rm = TRUE)
format_bytes <- function(bytes) if (bytes >= 1024^3) sprintf("%.1f GB", bytes / 1024^3) else sprintf("%.1f MB", bytes / 1024^2)
if (method == "ward") {
  pair_bytes <- 12 * as.numeric(clustered) * (as.numeric(clustered) - 1)
  need <- pair_bytes + as.numeric(nx) * ny * nb * 64 + 128 * 1024^2
  cat(sprintf("ASTROBURST_CLUSTER\t%d\t%.0f\t%d\n", clustered, pair_bytes, hclust_limit))
  flush(stdout())
  if (clustered > hclust_limit) refuse(sprintf("Exact Ward in R is limited to %d clustered pixels; this field has %d support pixels, an upper bound on the spectra SAGUI keeps after its validity filter. Crop the field, raise the starlet threshold or select sparse Ward", hclust_limit, clustered))
  if (is.finite(commit_available) && need > commit_available) refuse(sprintf("Exact Ward needs up to about %s for %d support pixels (an upper bound on the spectra SAGUI keeps after its validity filter) but only %s can be committed; crop the field, raise the starlet threshold or select sparse Ward", format_bytes(need), clustered, format_bytes(commit_available)))
  if (is.finite(physical_available) && need > 0.8 * physical_available) emit("ASTROBURST_WARNING", sprintf("Exact Ward needs up to about %s for %d support pixels and %s is free; the system may page to disk", format_bytes(need), clustered, format_bytes(physical_available)))
} else {
  cat(sprintf("ASTROBURST_CLUSTER\t%d\tNA\tNA\n", clustered))
  flush(stdout())
}
params <- list(input = cube, Ncomp = nregions, use_starlet_mask = starlet,
  mask_pretransform = "none", cluster_pretransform = "none", starlet_J = max(scales),
  starlet_scales = scales, denoise_k = threshold, mode = "soft", positive_only = TRUE,
  clean_mask = FALSE, mask_mode = "na", hclust_method = "ward.D2")
if (sed_scaling == "median_mad") params["scale_fn"] <- list(NULL)
segment <- function() {
  set.seed(seed)
  if (method == "ward") {
    fit <- do.call(sagui::segment_regions, params)
  } else {
    params$knn_k <- knn
    params$auto_k <- FALSE
    fit <- do.call(sagui::segment_regions_large, params)
  }
  info <- fit$backend_info
  if (!is.null(info)) {
    utils::write.table(data.frame(knn_k = info$knn_k, requested_Ncomp = info$requested_Ncomp, actual_Ncomp = info$actual_Ncomp,
      disconnected = isTRUE(info$disconnected), valid_pixels = info$valid_pixels), "backend.tsv", sep = "\t", row.names = FALSE, quote = FALSE)
    if (isTRUE(info$disconnected)) stop(sprintf("Sparse Ward neighbour graph is disconnected: %d clusters for %d requested; increase the nearest-neighbour count", info$actual_Ncomp, nregions))
  }
  fit
}
result <- withCallingHandlers(
  tryCatch(segment(), error = function(e) fail(conditionMessage(e))),
  warning = function(w) emit("ASTROBURST_WARNING", conditionMessage(w)))
stopifnot(identical(dim(result$cluster_map), c(nx, ny)))
labels <- as.integer(result$cluster_map)
labels[is.na(labels)] <- 0L
upstream <- as.integer(result$mask)
upstream[is.na(upstream)] <- 0L
support <- as.integer(result$mask & eligible)
support[is.na(support)] <- 0L
writeBin(labels, "labels.bin", size = 4L, endian = "little")
writeBin(support, "support.bin", size = 4L, endian = "little")
writeBin(upstream, "support-upstream.bin", size = 4L, endian = "little")
writeLines(capture.output(sessionInfo()), "session.txt")
