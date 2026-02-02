#!/usr/bin/env bash
#
# build-all.sh
# Master build script for SoliDB Mobile SDK
#
# This script:
#   - Checks prerequisites (rust targets, uniffi-bindgen)
#   - Runs both iOS and Android builds
#   - Packages everything for distribution
#   - Creates a release tarball
#

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m' # No Color

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
MOBILE_SDK_DIR="${PROJECT_ROOT}/clients/mobile-sdk"
DIST_DIR="${MOBILE_SDK_DIR}/dist"

# Version (from Cargo.toml)
VERSION=$(grep '^version' "${PROJECT_ROOT}/clients/rust-client/Cargo.toml" | head -1 | cut -d'"' -f2)

# Build flags
BUILD_IOS=${BUILD_IOS:-true}
BUILD_ANDROID=${BUILD_ANDROID:-true}
SKIP_CHECKS=${SKIP_CHECKS:-false}
CREATE_TARBALL=${CREATE_TARBALL:-true}

# Logging functions
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_section() {
    echo ""
    echo -e "${CYAN}${BOLD}========================================${NC}"
    echo -e "${CYAN}${BOLD}  $1${NC}"
    echo -e "${CYAN}${BOLD}========================================${NC}"
    echo ""
}

# Error handler
error_exit() {
    log_error "$1"
    exit 1
}

# Check if a command exists
check_command() {
    if ! command -v "$1" &> /dev/null; then
        error_exit "$1 is required but not installed"
    fi
}

# Check prerequisites
check_prerequisites() {
    log_section "Checking Prerequisites"
    
    # Check for Rust
    check_command "cargo"
    check_command "rustc"
    
    # Check for uniffi-bindgen
    check_command "uniffi-bindgen"
    
    # Check for zip (for creating AAR and tarballs)
    check_command "zip"
    
    # Display versions
    log_info "Rust version: $(rustc --version)"
    log_info "Cargo version: $(cargo --version)"
    log_info "UniFFI version: $(uniffi-bindgen --version 2>&1 | head -1)"
    
    log_success "All prerequisites found"
}

# Check Rust targets
check_rust_targets() {
    log_section "Checking Rust Targets"
    
    # iOS targets
    if [[ "$BUILD_IOS" == "true" ]]; then
        log_info "Checking iOS targets..."
        local ios_targets=("aarch64-apple-ios" "aarch64-apple-ios-sim" "x86_64-apple-ios")
        for target in "${ios_targets[@]}"; do
            if ! rustup target list --installed | grep -q "$target"; then
                log_info "Installing target: $target"
                rustup target add "$target" || log_warning "Failed to install $target (will try anyway)"
            fi
        done
    fi
    
    # Android targets
    if [[ "$BUILD_ANDROID" == "true" ]]; then
        log_info "Checking Android targets..."
        local android_targets=("aarch64-linux-android" "armv7-linux-androideabi" "x86_64-linux-android")
        for target in "${android_targets[@]}"; do
            if ! rustup target list --installed | grep -q "$target"; then
                log_info "Installing target: $target"
                rustup target add "$target" || log_warning "Failed to install $target (will try anyway)"
            fi
        done
    fi
    
    log_success "Rust targets checked"
}

# Check environment variables
check_environment() {
    log_section "Checking Environment"
    
    if [[ "$BUILD_ANDROID" == "true" ]]; then
        if [[ -z "${ANDROID_NDK_HOME:-}" ]] && [[ -z "${ANDROID_HOME:-}" ]]; then
            log_warning "ANDROID_NDK_HOME or ANDROID_HOME not set - Android build may fail"
            log_info "Install Android NDK and set ANDROID_NDK_HOME environment variable"
        else
            if [[ -n "${ANDROID_NDK_HOME:-}" ]]; then
                log_info "ANDROID_NDK_HOME: $ANDROID_NDK_HOME"
            fi
            if [[ -n "${ANDROID_HOME:-}" ]]; then
                log_info "ANDROID_HOME: $ANDROID_HOME"
            fi
        fi
    fi
    
    if [[ "$BUILD_IOS" == "true" ]]; then
        if ! command -v xcodebuild &> /dev/null; then
            log_warning "xcodebuild not found - iOS build may fail"
            log_info "Install Xcode Command Line Tools"
        else
            log_info "Xcode found: $(xcodebuild -version 2>&1 | head -1)"
        fi
    fi
    
    log_success "Environment checked"
}

# Clean distribution directory
clean_dist() {
    log_section "Cleaning Distribution Directory"
    rm -rf "$DIST_DIR"
    mkdir -p "$DIST_DIR"
    log_success "Distribution directory cleaned"
}

# Run iOS build
build_ios() {
    log_section "Building iOS Framework"
    
    local ios_script="${SCRIPT_DIR}/build-ios-framework.sh"
    
    if [[ ! -f "$ios_script" ]]; then
        error_exit "iOS build script not found: $ios_script"
    fi
    
    chmod +x "$ios_script"
    
    log_info "Running iOS build script..."
    if ! "$ios_script"; then
        error_exit "iOS build failed"
    fi
    
    log_success "iOS build completed"
}

# Run Android build
build_android() {
    log_section "Building Android AAR"
    
    local android_script="${SCRIPT_DIR}/build-android-aar.sh"
    
    if [[ ! -f "$android_script" ]]; then
        error_exit "Android build script not found: $android_script"
    fi
    
    chmod +x "$android_script"
    
    log_info "Running Android build script..."
    if ! "$android_script"; then
        error_exit "Android build failed"
    fi
    
    log_success "Android build completed"
}

# Copy build artifacts to dist
copy_artifacts() {
    log_section "Copying Build Artifacts"
    
    # iOS artifacts
    if [[ "$BUILD_IOS" == "true" ]]; then
        local ios_build="${MOBILE_SDK_DIR}/build/ios"
        local ios_dist="${DIST_DIR}/ios"
        
        if [[ -d "$ios_build" ]]; then
            mkdir -p "$ios_dist"
            
            # Copy XCFramework
            if [[ -d "${ios_build}/SoliDBClient.xcframework" ]]; then
                cp -R "${ios_build}/SoliDBClient.xcframework" "$ios_dist/"
                log_success "Copied XCFramework"
            fi
            
            # Copy Swift bindings
            if [[ -f "${ios_build}/solidb_client.swift" ]]; then
                cp "${ios_build}/solidb_client.swift" "$ios_dist/"
                log_success "Copied Swift bindings"
            fi
            
            # Copy headers
            cp "${ios_build}"/*.h "$ios_dist/" 2>/dev/null || true
            cp "${ios_build}"/*.modulemap "$ios_dist/" 2>/dev/null || true
        else
            log_warning "iOS build directory not found"
        fi
    fi
    
    # Android artifacts
    if [[ "$BUILD_ANDROID" == "true" ]]; then
        local android_build="${MOBILE_SDK_DIR}/build/android"
        local android_dist="${DIST_DIR}/android"
        
        if [[ -d "$android_build" ]]; then
            mkdir -p "$android_dist"
            
            # Copy AAR
            if [[ -f "${android_build}/solidb_client.aar" ]]; then
                cp "${android_build}/solidb_client.aar" "$android_dist/"
                log_success "Copied AAR file"
            fi
            
            # Copy Kotlin bindings
            if [[ -d "${android_build}/kotlin" ]]; then
                mkdir -p "${android_dist}/kotlin"
                cp "${android_build}/kotlin"/*.kt "${android_dist}/kotlin/" 2>/dev/null || true
                log_success "Copied Kotlin bindings"
            fi
            
            # Copy README
            cp "${android_build}/README.md" "$android_dist/" 2>/dev/null || true
            cp "${android_build}/build.gradle.kts" "$android_dist/" 2>/dev/null || true
        else
            log_warning "Android build directory not found"
        fi
    fi
    
    log_success "Artifacts copied to distribution directory"
}

# Create distribution tarball
create_tarball() {
    log_section "Creating Distribution Package"
    
    local tarball_name="solidb-mobile-sdk-${VERSION}.tar.gz"
    local tarball_path="${DIST_DIR}/${tarball_name}"
    
    # Create a temporary directory for packaging
    local temp_dir=$(mktemp -d)
    local package_dir="${temp_dir}/solidb-mobile-sdk-${VERSION}"
    mkdir -p "$package_dir"
    
    # Copy artifacts
    if [[ "$BUILD_IOS" == "true" ]] && [[ -d "${DIST_DIR}/ios" ]]; then
        cp -R "${DIST_DIR}/ios" "$package_dir/"
    fi
    
    if [[ "$BUILD_ANDROID" == "true" ]] && [[ -d "${DIST_DIR}/android" ]]; then
        cp -R "${DIST_DIR}/android" "$package_dir/"
    fi
    
    # Create README
    cat > "${package_dir}/README.md" << EOF
# SoliDB Mobile SDK ${VERSION}

Native iOS and Android SDKs for SoliDB with offline-first synchronization.

## Contents

EOF
    
    if [[ "$BUILD_IOS" == "true" ]]; then
        cat >> "${package_dir}/README.md" << EOF
### iOS SDK
- \`ios/SoliDBClient.xcframework\` - XCFramework for iOS integration
- \`ios/solidb_client.swift\` - Swift bindings

### iOS Integration

1. Drag \`SoliDBClient.xcframework\` into your Xcode project
2. Add to 'Frameworks, Libraries, and Embedded Content'
3. Set embed option to 'Embed & Sign'

See \`clients/ios-example/\` for a complete example.

EOF
    fi
    
    if [[ "$BUILD_ANDROID" == "true" ]]; then
        cat >> "${package_dir}/README.md" << EOF
### Android SDK
- \`android/solidb_client.aar\` - AAR file for Android integration
- \`android/kotlin/\` - Kotlin bindings

### Android Integration

1. Copy \`solidb_client.aar\` to your app's \`libs/\` directory
2. Add to \`build.gradle.kts\`:
   \`\`\`kotlin
   dependencies {
       implementation(files("libs/solidb_client.aar"))
   }
   \`\`\`

See \`clients/android-example/\` for a complete example.

EOF
    fi
    
    cat >> "${package_dir}/README.md" << EOF
## Documentation

Full documentation: https://solidb.io/docs/mobile

## License

MIT License - See LICENSE file for details
EOF
    
    # Create the tarball
    cd "$temp_dir"
    tar -czf "$tarball_path" "solidb-mobile-sdk-${VERSION}"
    cd - > /dev/null
    
    # Clean up temp directory
    rm -rf "$temp_dir"
    
    if [[ -f "$tarball_path" ]]; then
        log_success "Distribution package created: ${tarball_name}"
        log_info "Size: $(du -h "$tarball_path" | cut -f1)"
    else
        error_exit "Failed to create tarball"
    fi
}

# Create checksums
create_checksums() {
    log_section "Creating Checksums"
    
    cd "$DIST_DIR"
    
    # Create SHA256 checksums
    if command -v sha256sum &> /dev/null; then
        sha256sum *.tar.gz > checksums.sha256 2>/dev/null || true
        log_success "SHA256 checksums created"
    elif command -v shasum &> /dev/null; then
        shasum -a 256 *.tar.gz > checksums.sha256 2>/dev/null || true
        log_success "SHA256 checksums created"
    else
        log_warning "No checksum utility found"
    fi
    
    cd - > /dev/null
}

# Print final summary
print_summary() {
    log_section "Build Summary"
    
    echo -e "${BOLD}Version:${NC} ${VERSION}"
    echo ""
    
    if [[ "$BUILD_IOS" == "true" ]]; then
        echo -e "${BOLD}iOS:${NC}"
        if [[ -d "${DIST_DIR}/ios" ]]; then
            ls -lh "${DIST_DIR}/ios/" 2>/dev/null || echo "  (Files created)"
        else
            echo "  (Not built)"
        fi
        echo ""
    fi
    
    if [[ "$BUILD_ANDROID" == "true" ]]; then
        echo -e "${BOLD}Android:${NC}"
        if [[ -d "${DIST_DIR}/android" ]]; then
            ls -lh "${DIST_DIR}/android/" 2>/dev/null || echo "  (Files created)"
        else
            echo "  (Not built)"
        fi
        echo ""
    fi
    
    if [[ "$CREATE_TARBALL" == "true" ]] && [[ -f "${DIST_DIR}/solidb-mobile-sdk-${VERSION}.tar.gz" ]]; then
        echo -e "${BOLD}Distribution Package:${NC}"
        echo "  ${DIST_DIR}/solidb-mobile-sdk-${VERSION}.tar.gz"
        echo "  Size: $(du -h "${DIST_DIR}/solidb-mobile-sdk-${VERSION}.tar.gz" | cut -f1)"
        echo ""
    fi
    
    echo -e "${GREEN}${BOLD}All builds completed successfully!${NC}"
    echo ""
    echo "Distribution directory: ${DIST_DIR}"
}

# Print usage
print_usage() {
    cat << EOF
Usage: $0 [OPTIONS]

Build SoliDB Mobile SDK for iOS and Android

OPTIONS:
    --ios-only          Build only iOS framework
    --android-only      Build only Android AAR
    --skip-checks       Skip prerequisite checks (faster, but risky)
    --no-tarball        Don't create distribution tarball
    --help              Show this help message

ENVIRONMENT VARIABLES:
    BUILD_IOS           Set to 'false' to skip iOS build (default: true)
    BUILD_ANDROID       Set to 'false' to skip Android build (default: true)
    SKIP_CHECKS         Set to 'true' to skip prerequisite checks (default: false)
    CREATE_TARBALL      Set to 'false' to skip tarball creation (default: true)
    ANDROID_NDK_HOME    Path to Android NDK (required for Android build)
    ANDROID_HOME        Path to Android SDK (alternative to ANDROID_NDK_HOME)

EXAMPLES:
    # Build everything
    $0

    # Build only iOS
    $0 --ios-only

    # Build only Android
    $0 --android-only

    # Build with custom environment
    BUILD_IOS=false BUILD_ANDROID=true $0

EOF
}

# Parse command line arguments
parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            --ios-only)
                BUILD_ANDROID=false
                shift
                ;;
            --android-only)
                BUILD_IOS=false
                shift
                ;;
            --skip-checks)
                SKIP_CHECKS=true
                shift
                ;;
            --no-tarball)
                CREATE_TARBALL=false
                shift
                ;;
            --help)
                print_usage
                exit 0
                ;;
            *)
                log_error "Unknown option: $1"
                print_usage
                exit 1
                ;;
        esac
    done
}

# Main build process
main() {
    # Parse arguments
    parse_args "$@"
    
    # Print header
    log_section "SoliDB Mobile SDK Builder v${VERSION}"
    
    # Check what we're building
    if [[ "$BUILD_IOS" == "true" ]]; then
        log_info "iOS build: ENABLED"
    else
        log_info "iOS build: SKIPPED"
    fi
    
    if [[ "$BUILD_ANDROID" == "true" ]]; then
        log_info "Android build: ENABLED"
    else
        log_info "Android build: SKIPPED"
    fi
    
    echo ""
    
    # Run checks unless skipped
    if [[ "$SKIP_CHECKS" == "false" ]]; then
        check_prerequisites
        check_rust_targets
        check_environment
    else
        log_warning "Skipping prerequisite checks (--skip-checks)"
    fi
    
    # Clean distribution directory
    clean_dist
    
    # Run builds
    if [[ "$BUILD_IOS" == "true" ]]; then
        build_ios
    fi
    
    if [[ "$BUILD_ANDROID" == "true" ]]; then
        build_android
    fi
    
    # Copy artifacts
    copy_artifacts
    
    # Create tarball
    if [[ "$CREATE_TARBALL" == "true" ]]; then
        create_tarball
        create_checksums
    fi
    
    # Print summary
    print_summary
}

# Run main
main "$@"
