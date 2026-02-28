#!/usr/bin/env bash
set -euo pipefail

REPO="samuelkriegerbonini-dev/AstroBurst"
APP_NAME="AstroBurst"
INSTALL_DIR="/opt/astroburst"
BIN_LINK="/usr/local/bin/astroburst"

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
        x86_64|amd64) echo "x86_64" ;;
        aarch64|arm64) echo "aarch64" ;;
        *) error "Unsupported architecture: $arch" ;;
    esac
}

detect_package_format() {
    if command -v dpkg &>/dev/null; then
        echo "deb"
    elif command -v rpm &>/dev/null; then
        echo "rpm"
    else
        echo "appimage"
    fi
}

check_dependencies() {
    local missing=()

    for dep in curl; do
        if ! command -v "$dep" &>/dev/null; then
            missing+=("$dep")
        fi
    done

    if [ ${#missing[@]} -gt 0 ]; then
        error "Missing dependencies: ${missing[*]}\nInstall them and try again."
    fi
}

install_system_deps() {
    info "Checking system dependencies..."

    if command -v apt-get &>/dev/null; then
        local pkgs=(libwebkit2gtk-4.1-0 libgtk-3-0 libayatana-appindicator3-1)
        local to_install=()
        for pkg in "${pkgs[@]}"; do
            if ! dpkg -l "$pkg" &>/dev/null 2>&1; then
                to_install+=("$pkg")
            fi
        done
        if [ ${#to_install[@]} -gt 0 ]; then
            info "Installing: ${to_install[*]}"
            sudo apt-get update -qq
            sudo apt-get install -y -qq "${to_install[@]}"
        fi
    elif command -v dnf &>/dev/null; then
        local pkgs=(webkit2gtk4.1 gtk3 libappindicator-gtk3)
        for pkg in "${pkgs[@]}"; do
            if ! rpm -q "$pkg" &>/dev/null 2>&1; then
                sudo dnf install -y "$pkg"
            fi
        done
    fi

    ok "System dependencies satisfied"
}

get_latest_release() {
    local url="https://api.github.com/repos/${REPO}/releases/latest"
    curl -sL "$url" | grep '"tag_name"' | head -1 | sed -E 's/.*"v([^"]+)".*/\1/'
}

download_and_install() {
    local version="$1"
    local arch="$2"
    local format="$3"
    local tmp_dir

    tmp_dir=$(mktemp -d)
    trap "rm -rf $tmp_dir" EXIT

    local base_url="https://github.com/${REPO}/releases/download/v${version}"
    local filename

    case "$format" in
        deb)
            filename="astroburst_${version}_${arch}.deb"
            info "Downloading ${filename}..."
            curl -fsSL "${base_url}/${filename}" -o "${tmp_dir}/${filename}" || error "Download failed. Check if release exists."
            info "Installing .deb package..."
            sudo dpkg -i "${tmp_dir}/${filename}"
            sudo apt-get install -f -y -qq
            ;;
        rpm)
            filename="astroburst-${version}-1.${arch}.rpm"
            info "Downloading ${filename}..."
            curl -fsSL "${base_url}/${filename}" -o "${tmp_dir}/${filename}" || error "Download failed. Check if release exists."
            info "Installing .rpm package..."
            sudo rpm -U "${tmp_dir}/${filename}"
            ;;
        appimage)
            filename="astroburst_${version}_${arch}.AppImage"
            info "Downloading ${filename}..."
            curl -fsSL "${base_url}/${filename}" -o "${tmp_dir}/${filename}" || error "Download failed. Check if release exists."

            sudo mkdir -p "$INSTALL_DIR"
            sudo cp "${tmp_dir}/${filename}" "${INSTALL_DIR}/${APP_NAME}.AppImage"
            sudo chmod +x "${INSTALL_DIR}/${APP_NAME}.AppImage"
            sudo ln -sf "${INSTALL_DIR}/${APP_NAME}.AppImage" "$BIN_LINK"

            cat > "/tmp/astroburst.desktop" << EOF
[Desktop Entry]
Name=AstroBurst
Comment=High-Performance Astronomical Image Processor
Exec=${INSTALL_DIR}/${APP_NAME}.AppImage
Icon=astroburst
Type=Application
Categories=Science;Astronomy;Graphics;
MimeType=application/fits;image/fits;
Terminal=false
EOF
            sudo mv "/tmp/astroburst.desktop" /usr/share/applications/astroburst.desktop
            ;;
    esac

    ok "AstroBurst v${version} installed successfully!"
}

build_from_source() {
    info "Building from source..."

    for dep in rustc cargo node npm; do
        if ! command -v "$dep" &>/dev/null; then
            error "Missing build dependency: $dep"
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

    local format
    format=$(detect_package_format)
    local bundle_dir="src-tauri/target/release/bundle"

    case "$format" in
        deb)
            local deb_file
            deb_file=$(find "$bundle_dir/deb" -name "*.deb" | head -1)
            if [ -n "$deb_file" ]; then
                sudo dpkg -i "$deb_file"
                ok "Installed from .deb"
            fi
            ;;
        rpm)
            local rpm_file
            rpm_file=$(find "$bundle_dir/rpm" -name "*.rpm" | head -1)
            if [ -n "$rpm_file" ]; then
                sudo rpm -U "$rpm_file"
                ok "Installed from .rpm"
            fi
            ;;
        appimage)
            local appimage_file
            appimage_file=$(find "$bundle_dir/appimage" -name "*.AppImage" | head -1)
            if [ -n "$appimage_file" ]; then
                sudo mkdir -p "$INSTALL_DIR"
                sudo cp "$appimage_file" "${INSTALL_DIR}/${APP_NAME}.AppImage"
                sudo chmod +x "${INSTALL_DIR}/${APP_NAME}.AppImage"
                sudo ln -sf "${INSTALL_DIR}/${APP_NAME}.AppImage" "$BIN_LINK"
                ok "Installed AppImage"
            fi
            ;;
    esac
}

uninstall() {
    info "Uninstalling AstroBurst..."

    if command -v dpkg &>/dev/null && dpkg -l astroburst &>/dev/null 2>&1; then
        sudo dpkg -r astroburst
    elif command -v rpm &>/dev/null && rpm -q astroburst &>/dev/null 2>&1; then
        sudo rpm -e astroburst
    else
        sudo rm -rf "$INSTALL_DIR"
        sudo rm -f "$BIN_LINK"
        sudo rm -f /usr/share/applications/astroburst.desktop
    fi

    ok "AstroBurst uninstalled"
}

usage() {
    cat << EOF
${CYAN}AstroBurst Installer for Linux${NC}

Usage: $0 [OPTION]

Options:
  install       Download and install latest release (default)
  build         Build from source and install
  uninstall     Remove AstroBurst
  --help        Show this help

Examples:
  curl -fsSL https://raw.githubusercontent.com/${REPO}/main/scripts/install-linux.sh | bash
  ./install-linux.sh build
  ./install-linux.sh uninstall
EOF
}

main() {
    echo ""
    echo -e "${CYAN}╔══════════════════════════════════════╗${NC}"
    echo -e "${CYAN}║     AstroBurst Installer (Linux)     ║${NC}"
    echo -e "${CYAN}╚══════════════════════════════════════╝${NC}"
    echo ""

    local action="${1:-install}"

    case "$action" in
        install)
            check_dependencies
            install_system_deps

            local arch format version
            arch=$(detect_arch)
            format=$(detect_package_format)
            version=$(get_latest_release)

            if [ -z "$version" ]; then
                warn "No release found. Building from source instead..."
                build_from_source
            else
                info "Latest version: v${version}"
                info "Architecture: ${arch}"
                info "Package format: ${format}"
                download_and_install "$version" "$arch" "$format"
            fi
            ;;
        build)
            check_dependencies
            install_system_deps
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
