#!/usr/bin/env bash
#
# build-ios-framework.sh
# Build script for SoliDB iOS XCFramework
#
# This script:
#   - Builds Rust library for iOS targets (aarch64-apple-ios, aarch64-apple-ios-sim, x86_64-apple-ios)
#   - Generates Swift bindings using uniffi-bindgen
#   - Creates XCFramework bundle for distribution
#   - Outputs to build/ios/
#

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
RUST_CLIENT_DIR="${PROJECT_ROOT}/clients/rust-client"
BUILD_DIR="${PROJECT_ROOT}/clients/mobile-sdk/build/ios"

# iOS targets
IOS_DEVICE_TARGET="aarch64-apple-ios"
IOS_SIM_ARM_TARGET="aarch64-apple-ios-sim"
IOS_SIM_X86_TARGET="x86_64-apple-ios"

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
    log_info "Checking prerequisites..."
    
    # Check for Rust
    check_command "cargo"
    check_command "rustc"
    
    # Check for uniffi-bindgen
    check_command "uniffi-bindgen"
    
    # Check for lipo (for creating universal binaries)
    check_command "lipo"
    
    # Check for xcodebuild
    check_command "xcodebuild"
    
    log_success "All prerequisites found"
}

# Check and install Rust targets if needed
check_rust_targets() {
    log_info "Checking Rust targets..."
    
    local targets=("$IOS_DEVICE_TARGET" "$IOS_SIM_ARM_TARGET" "$IOS_SIM_X86_TARGET")
    local installed_targets
    installed_targets=$(rustup target list --installed)
    
    for target in "${targets[@]}"; do
        if echo "$installed_targets" | grep -q "$target"; then
            log_info "Target $target already installed"
        else
            log_info "Installing target $target..."
            rustup target add "$target" || error_exit "Failed to install target $target"
        fi
    done
    
    log_success "All Rust targets ready"
}

# Clean build directory
clean_build_dir() {
    log_info "Cleaning build directory..."
    rm -rf "$BUILD_DIR"
    mkdir -p "$BUILD_DIR"
    log_success "Build directory cleaned"
}

# Build for a specific target
build_for_target() {
    local target=$1
    local output_name=$2
    
    log_info "Building for $target..."
    
    cd "$RUST_CLIENT_DIR"
    
    cargo build --release --target "$target" \
        || error_exit "Failed to build for $target"
    
    # Copy the library to build directory
    local lib_path="${RUST_CLIENT_DIR}/target/${target}/release/libsolidb_client.dylib"
    local static_path="${RUST_CLIENT_DIR}/target/${target}/release/libsolidb_client.a"
    
    if [[ -f "$static_path" ]]; then
        cp "$static_path" "${BUILD_DIR}/${output_name}.a"
        log_success "Built static library for $target"
    elif [[ -f "$lib_path" ]]; then
        cp "$lib_path" "${BUILD_DIR}/${output_name}.dylib"
        log_success "Built dynamic library for $target"
    else
        error_exit "No library found for $target"
    fi
}

# Create universal binary for simulator
create_universal_simulator() {
    log_info "Creating universal simulator binary..."
    
    local arm_lib="${BUILD_DIR}/libsolidb_client_ios_sim_arm64.a"
    local x86_lib="${BUILD_DIR}/libsolidb_client_ios_sim_x86_64.a"
    local universal_lib="${BUILD_DIR}/libsolidb_client_ios_sim.a"
    
    if [[ ! -f "$arm_lib" ]] || [[ ! -f "$x86_lib" ]]; then
        error_exit "Simulator libraries not found"
    fi
    
    lipo -create "$arm_lib" "$x86_lib" -output "$universal_lib" \
        || error_exit "Failed to create universal binary"
    
    # Verify the universal binary
    lipo -info "$universal_lib"
    
    log_success "Universal simulator binary created"
}

# Generate Swift bindings
generate_swift_bindings() {
    log_info "Generating Swift bindings..."
    
    cd "$RUST_CLIENT_DIR"
    
    uniffi-bindgen generate src/solidb_client.udl \
        --language swift \
        --out-dir "$BUILD_DIR" \
        || error_exit "Failed to generate Swift bindings"
    
    log_success "Swift bindings generated"
}

# Create XCFramework
create_xcframework() {
    log_info "Creating XCFramework..."
    
    local device_lib="${BUILD_DIR}/libsolidb_client_ios_device.a"
    local sim_lib="${BUILD_DIR}/libsolidb_client_ios_sim.a"
    local swift_modulemap="${BUILD_DIR}/solidb_client.modulemap"
    local swift_headers="${BUILD_DIR}/solidb_clientFFI.h"
    local output_framework="${BUILD_DIR}/SoliDBClient.xcframework"
    
    # Check if required files exist
    if [[ ! -f "$device_lib" ]]; then
        error_exit "Device library not found at $device_lib"
    fi
    
    if [[ ! -f "$sim_lib" ]]; then
        error_exit "Simulator library not found at $sim_lib"
    fi
    
    if [[ ! -f "$swift_headers" ]]; then
        error_exit "Swift headers not found at $swift_headers"
    fi
    
    # Create framework structure
    rm -rf "$output_framework"
    
    xcodebuild -create-xcframework \
        -library "$device_lib" \
        -headers "$BUILD_DIR" \
        -library "$sim_lib" \
        -headers "$BUILD_DIR" \
        -output "$output_framework" \
        || error_exit "Failed to create XCFramework"
    
    # Verify the framework
    if [[ ! -d "$output_framework" ]]; then
        error_exit "XCFramework not created"
    fi
    
    log_success "XCFramework created at $output_framework"
}

# Copy Swift source files if generated
copy_swift_files() {
    log_info "Copying Swift source files..."
    
    local swift_file="${BUILD_DIR}/solidb_client.swift"
    if [[ -f "$swift_file" ]]; then
        log_info "Found Swift bindings file"
    else
        log_warning "Swift bindings file not found - may need to generate manually"
    fi
}

# Clean up temporary files
cleanup() {
    log_info "Cleaning up temporary files..."
    
    # Keep only the XCFramework and Swift files
    local temp_dir="${BUILD_DIR}/temp_$$"
    mkdir -p "$temp_dir"
    
    # Save important files
    mv "${BUILD_DIR}/SoliDBClient.xcframework" "$temp_dir/" 2>/dev/null || true
    mv "${BUILD_DIR}"/*.swift "$temp_dir/" 2>/dev/null || true
    mv "${BUILD_DIR}"/*.h "$temp_dir/" 2>/dev/null || true
    mv "${BUILD_DIR}"/*.modulemap "$temp_dir/" 2>/dev/null || true
    
    # Clean and restore
    rm -rf "${BUILD_DIR:?}/"*
    mv "$temp_dir"/* "$BUILD_DIR/" 2>/dev/null || true
    rmdir "$temp_dir" 2>/dev/null || true
    
    log_success "Cleanup complete"
}

# Print summary
print_summary() {
    echo ""
    echo "========================================"
    echo "  iOS Build Complete"
    echo "========================================"
    echo ""
    echo "Output: ${BUILD_DIR}/SoliDBClient.xcframework"
    echo ""
    echo "Framework contents:"
    ls -la "${BUILD_DIR}/SoliDBClient.xcframework/" 2>/dev/null || echo "  (Framework structure created)"
    echo ""
    echo "Swift bindings:"
    ls -la "${BUILD_DIR}/"*.swift 2>/dev/null || echo "  (Swift bindings generated)"
    echo ""
    echo "To use in your project:"
    echo "  1. Drag SoliDBClient.xcframework into Xcode"
    echo "  2. Add to 'Frameworks, Libraries, and Embedded Content'"
    echo "  3. Set embed option to 'Embed & Sign'"
    echo ""
}

# Main build process
main() {
    echo "========================================"
    echo "  SoliDB iOS Framework Builder"
    echo "========================================"
    echo ""
    
    check_prerequisites
    check_rust_targets
    clean_build_dir
    
    # Build for all targets
    build_for_target "$IOS_DEVICE_TARGET" "libsolidb_client_ios_device"
    build_for_target "$IOS_SIM_ARM_TARGET" "libsolidb_client_ios_sim_arm64"
    build_for_target "$IOS_SIM_X86_TARGET" "libsolidb_client_ios_sim_x86_64"
    
    # Create universal simulator binary
    create_universal_simulator
    
    # Generate Swift bindings
    generate_swift_bindings
    
    # Copy Swift files
    copy_swift_files
    
    # Create XCFramework
    create_xcframework
    
    # Cleanup
    cleanup
    
    # Print summary
    print_summary
}

# Run main
main "$@"
