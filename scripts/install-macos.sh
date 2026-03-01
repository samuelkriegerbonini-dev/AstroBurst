#!/usr/bin/env bash
set -euo pipefail

REPO="samuelkriegerbonini-dev/AstroBurst"
APP_NAME="AstroBurst"
INSTALL_DIR="/Applications"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

info()  { echo -e "${CYAN}[INFO]${NC} $1"; }
ok()    { echo -e "${GREEN}[OK]${NC} $1"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $1"; }
error() { echo -e "${RED}[ERROR]${NC} $1"; exit 1; }

detect_arch() {
    local arch
    arch=$(uname -m)
    case "$arch" in
        x86_64) echo "x86_64" ;;
        arm64)  echo "aarch64" ;;
        *) error "Unsupported architecture: $arch" ;;
    esac
}

check_dependencies() {
    if ! command -v curl &>/dev/null; then
        error "curl is required. Install Xcode Command Line Tools:\n  xcode-select --install"
    fi
}

get_latest_release() {
    local url="https://api.github.com/repos/${REPO}/releases/latest"
    curl -sL "$url" | grep '"tag_name"' | head -1 | sed -E 's/.*"v([^"]+)".*/\1/'
}

download_and_install() {
    local version="$1"
    local arch="$2"
    local tmp_dir

    tmp_dir=$(mktemp -d)
    trap "rm -rf $tmp_dir" EXIT

    local base_url="https://github.com/${REPO}/releases/download/v${version}"

    local arch_suffix
    case "$arch" in
        x86_64)  arch_suffix="x64" ;;
        aarch64) arch_suffix="aarch64" ;;
    esac

    local filename="AstroBurst_${version}_${arch_suffix}.dmg"

    info "Downloading ${filename}..."
    curl -fsSL "${base_url}/${filename}" -o "${tmp_dir}/${filename}" || error "Download failed. Check if release v${version} exists."

    info "Mounting DMG..."
    local mount_point
    mount_point=$(hdiutil attach "${tmp_dir}/${filename}" -nobrowse -readonly 2>/dev/null | grep "/Volumes" | awk '{print $NF}')

    if [ -z "$mount_point" ]; then
        error "Failed to mount DMG"
    fi

    if [ -d "${INSTALL_DIR}/${APP_NAME}.app" ]; then
        warn "Removing previous installation..."
        rm -rf "${INSTALL_DIR}/${APP_NAME}.app"
    fi

    info "Installing to ${INSTALL_DIR}..."
    cp -R "${mount_point}/${APP_NAME}.app" "${INSTALL_DIR}/"

    info "Unmounting DMG..."
    hdiutil detach "$mount_point" -quiet

    if [[ "$arch" == "aarch64" ]]; then
        info "Removing quarantine attribute..."
        xattr -cr "${INSTALL_DIR}/${APP_NAME}.app" 2>/dev/null || true
    fi

    ok "AstroBurst v${version} installed to ${INSTALL_DIR}/${APP_NAME}.app"
}

install_homebrew_deps() {
    if ! command -v brew &>/dev/null; then
        return
    fi

    info "Checking Homebrew dependencies..."
    local deps_needed=false

    if ! command -v rustc &>/dev/null; then
        deps_needed=true
    fi

    if $deps_needed; then
        info "Installing Rust via Homebrew..."
        brew install rust
    fi
}

build_from_source() {
    info "Building from source..."

    for dep in rustc cargo node; do
        if ! command -v "$dep" &>/dev/null; then
            error "Missing: $dep\nInstall Rust: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh\nInstall Node: brew install node"
        fi
    done

    if ! command -v pnpm &>/dev/null; then
        info "Installing pnpm..."
        npm install -g pnpm
    fi

    info "Installing frontend dependencies..."
    pnpm install

    info "Building release (this may take several minutes)..."
    pnpm tauri build

    local dmg_file
    dmg_file=$(find src-tauri/target/release/bundle/dmg -name "*.dmg" 2>/dev/null | head -1)

    if [ -n "$dmg_file" ]; then
        info "Opening DMG for installation..."
        open "$dmg_file"
        ok "DMG created at: $dmg_file"
        echo -e "  Drag ${APP_NAME}.app to Applications to complete installation."
    else
        local app_dir
        app_dir=$(find src-tauri/target/release/bundle/macos -name "*.app" 2>/dev/null | head -1)
        if [ -n "$app_dir" ]; then
            cp -R "$app_dir" "${INSTALL_DIR}/"
            ok "Installed to ${INSTALL_DIR}"
        else
            error "Build succeeded but no bundle found"
        fi
    fi
}

uninstall() {
    info "Uninstalling AstroBurst..."

    if [ -d "${INSTALL_DIR}/${APP_NAME}.app" ]; then
        rm -rf "${INSTALL_DIR}/${APP_NAME}.app"
        ok "Removed ${INSTALL_DIR}/${APP_NAME}.app"
    else
        warn "AstroBurst not found in ${INSTALL_DIR}"
    fi

    local app_support="${HOME}/Library/Application Support/com.astroburst.desktop"
    if [ -d "$app_support" ]; then
        rm -rf "$app_support"
        ok "Removed application data"
    fi
}

usage() {
    cat << EOF
${CYAN}AstroBurst Installer for macOS${NC}

Usage: $0 [OPTION]

Options:
  install       Download and install latest release (default)
  build         Build from source and install
  uninstall     Remove AstroBurst
  --help        Show this help

Examples:
  curl -fsSL https://raw.githubusercontent.com/${REPO}/main/scripts/install-macos.sh | bash
  ./install-macos.sh build
  ./install-macos.sh uninstall

Requirements:
  - macOS 11+ (Big Sur or later)
  - For building: Rust 1.75+, Node.js 18+, Xcode Command Line Tools
EOF
}

main() {
    echo ""
    echo -e "${CYAN}╔══════════════════════════════════════╗${NC}"
    echo -e "${CYAN}║     AstroBurst Installer (macOS)     ║${NC}"
    echo -e "${CYAN}╚══════════════════════════════════════╝${NC}"
    echo ""

    if [[ "$(uname)" != "Darwin" ]]; then
        error "This script is for macOS only. Use install-linux.sh for Linux."
    fi

    local action="${1:-install}"

    case "$action" in
        install)
            check_dependencies

            local arch version
            arch=$(detect_arch)
            version=$(get_latest_release)

            if [ -z "$version" ]; then
                warn "No release found. Building from source instead..."
                build_from_source
            else
                info "Latest version: v${version}"
                info "Architecture: ${arch}"
                download_and_install "$version" "$arch"
            fi
            ;;
        build)
            check_dependencies
            build_from_source
            ;;
        uninstall)
            uninstall
            ;;
        --help|-h)
            usage
            ;;
        *)
            error "Unknown option: $action\nRun '$0 --help' for usage."
            ;;
    esac
}

main "$@"
