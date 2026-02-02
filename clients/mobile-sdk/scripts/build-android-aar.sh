#!/usr/bin/env bash
#
# build-android-aar.sh
# Build script for SoliDB Android AAR
#
# This script:
#   - Builds Rust library for Android targets (aarch64-linux-android, armv7-linux-androideabi, x86_64-linux-android)
#   - Generates Kotlin bindings using uniffi-bindgen
#   - Creates AAR file with JNI libraries
#   - Outputs to build/android/
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
BUILD_DIR="${PROJECT_ROOT}/clients/mobile-sdk/build/android"

# Android targets
ANDROID_ARM64_TARGET="aarch64-linux-android"
ANDROID_ARMV7_TARGET="armv7-linux-androideabi"
ANDROID_X86_64_TARGET="x86_64-linux-android"

# Android NDK configuration
ANDROID_NDK_VERSION="25"
ANDROID_API_LEVEL="21"

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
    
    # Check for Android NDK
    if [[ -z "${ANDROID_NDK_HOME:-}" ]]; then
        if [[ -z "${ANDROID_HOME:-}" ]]; then
            error_exit "ANDROID_NDK_HOME or ANDROID_HOME environment variable must be set"
        fi
        
        # Try to find NDK in ANDROID_HOME
        local ndk_path="${ANDROID_HOME}/ndk/${ANDROID_NDK_VERSION}*"
        if [[ -d $ndk_path ]]; then
            export ANDROID_NDK_HOME=$(ls -d $ndk_path | head -1)
            log_info "Found NDK at: $ANDROID_NDK_HOME"
        else
            error_exit "Android NDK not found. Please install NDK $ANDROID_NDK_VERSION or set ANDROID_NDK_HOME"
        fi
    else
        log_info "Using NDK at: $ANDROID_NDK_HOME"
    fi
    
    log_success "All prerequisites found"
}

# Check and install Rust targets if needed
check_rust_targets() {
    log_info "Checking Rust targets..."
    
    local targets=("$ANDROID_ARM64_TARGET" "$ANDROID_ARMV7_TARGET" "$ANDROID_X86_64_TARGET")
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

# Get the appropriate clang binary for a target
get_clang() {
    local target=$1
    local host_tag
    
    # Determine host tag based on OS
    case "$(uname -s)" in
        Darwin)
            host_tag="darwin-x86_64"
            ;;
        Linux)
            host_tag="linux-x86_64"
            ;;
        *)
            error_exit "Unsupported host OS: $(uname -s)"
            ;;
    esac
    
    local toolchain_dir="${ANDROID_NDK_HOME}/toolchains/llvm/prebuilt/${host_tag}/bin"
    
    case "$target" in
        "aarch64-linux-android")
            echo "${toolchain_dir}/aarch64-linux-android${ANDROID_API_LEVEL}-clang"
            ;;
        "armv7-linux-androideabi")
            echo "${toolchain_dir}/armv7a-linux-androideabi${ANDROID_API_LEVEL}-clang"
            ;;
        "x86_64-linux-android")
            echo "${toolchain_dir}/x86_64-linux-android${ANDROID_API_LEVEL}-clang"
            ;;
        *)
            error_exit "Unknown target: $target"
            ;;
    esac
}

# Get the appropriate ar binary for a target
get_ar() {
    local host_tag
    
    case "$(uname -s)" in
        Darwin)
            host_tag="darwin-x86_64"
            ;;
        Linux)
            host_tag="linux-x86_64"
            ;;
        *)
            error_exit "Unsupported host OS: $(uname -s)"
            ;;
    esac
    
    echo "${ANDROID_NDK_HOME}/toolchains/llvm/prebuilt/${host_tag}/bin/llvm-ar"
}

# Clean build directory
clean_build_dir() {
    log_info "Cleaning build directory..."
    rm -rf "$BUILD_DIR"
    mkdir -p "$BUILD_DIR"
    mkdir -p "${BUILD_DIR}/jniLibs"
    log_success "Build directory cleaned"
}

# Build for a specific Android target
build_for_android_target() {
    local target=$1
    local abi=$2
    
    log_info "Building for $target (ABI: $abi)..."
    
    cd "$RUST_CLIENT_DIR"
    
    # Get the appropriate compilers
    local clang
    clang=$(get_clang "$target")
    local ar
    ar=$(get_ar)
    
    if [[ ! -f "$clang" ]]; then
        error_exit "Clang not found at $clang"
    fi
    
    if [[ ! -f "$ar" ]]; then
        error_exit "AR not found at $ar"
    fi
    
    log_info "Using clang: $clang"
    log_info "Using ar: $ar"
    
    # Build with the correct environment
    CC="$clang" \
    AR="$ar" \
    cargo build --release --target "$target" \
        || error_exit "Failed to build for $target"
    
    # Copy the library to jniLibs
    local lib_path="${RUST_CLIENT_DIR}/target/${target}/release/libsolidb_client.so"
    local dest_dir="${BUILD_DIR}/jniLibs/${abi}"
    
    mkdir -p "$dest_dir"
    
    if [[ -f "$lib_path" ]]; then
        cp "$lib_path" "$dest_dir/"
        log_success "Built shared library for $target"
    else
        error_exit "No library found for $target at $lib_path"
    fi
}

# Generate Kotlin bindings
generate_kotlin_bindings() {
    log_info "Generating Kotlin bindings..."
    
    cd "$RUST_CLIENT_DIR"
    
    uniffi-bindgen generate src/solidb_client.udl \
        --language kotlin \
        --out-dir "${BUILD_DIR}/kotlin" \
        || error_exit "Failed to generate Kotlin bindings"
    
    log_success "Kotlin bindings generated"
}

# Create AAR structure
create_aar_structure() {
    log_info "Creating AAR structure..."
    
    local aar_dir="${BUILD_DIR}/solidb_client"
    
    # Create AAR directory structure
    mkdir -p "${aar_dir}/jni"
    mkdir -p "${aar_dir}/classes"
    
    # Copy JNI libraries
    cp -r "${BUILD_DIR}/jniLibs"/* "${aar_dir}/jni/" 2>/dev/null || true
    
    # Copy Kotlin files
    if [[ -d "${BUILD_DIR}/kotlin" ]]; then
        mkdir -p "${aar_dir}/kotlin/com/solidb/client"
        cp "${BUILD_DIR}/kotlin"/*.kt "${aar_dir}/kotlin/com/solidb/client/" 2>/dev/null || true
    fi
    
    log_success "AAR structure created"
}

# Create AAR manifest
create_manifest() {
    log_info "Creating AndroidManifest.xml..."
    
    local manifest_dir="${BUILD_DIR}/solidb_client"
    mkdir -p "$manifest_dir"
    
    cat > "${manifest_dir}/AndroidManifest.xml" << 'EOF'
<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    package="com.solidb.client"
    android:versionCode="1"
    android:versionName="0.7.0">
    
    <uses-sdk android:minSdkVersion="21" android:targetSdkVersion="34" />
    
    <uses-permission android:name="android.permission.INTERNET" />
    <uses-permission android:name="android.permission.ACCESS_NETWORK_STATE" />
    
</manifest>
EOF
    
    log_success "AndroidManifest.xml created"
}

# Create AAR file
create_aar_file() {
    log_info "Creating AAR file..."
    
    local aar_dir="${BUILD_DIR}/solidb_client"
    local aar_file="${BUILD_DIR}/solidb_client.aar"
    
    cd "$aar_dir"
    
    # Create the AAR file (ZIP format)
    zip -r "${aar_file}" . -x "*.DS_Store" \
        || error_exit "Failed to create AAR file"
    
    if [[ ! -f "$aar_file" ]]; then
        error_exit "AAR file not created"
    fi
    
    cd - > /dev/null
    
    log_success "AAR file created at $aar_file"
}

# Create a simple build.gradle file for reference
create_build_gradle() {
    log_info "Creating reference build.gradle..."
    
    cat > "${BUILD_DIR}/build.gradle.kts" << 'EOF'
// Example build.gradle.kts for integrating solidb_client.aar
// Add this to your app's build.gradle.kts:

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.example.myapp"
    compileSdk = 34
    
    defaultConfig {
        applicationId = "com.example.myapp"
        minSdk = 21
        targetSdk = 34
        versionCode = 1
        versionName = "1.0"
    }
}

dependencies {
    // Add the AAR file
    implementation(files("libs/solidb_client.aar"))
    
    // Kotlin coroutines for async operations
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.7.3")
}
EOF
    
    log_success "Reference build.gradle created"
}

# Create README for integration
create_integration_readme() {
    log_info "Creating integration README..."
    
    cat > "${BUILD_DIR}/README.md" << 'EOF'
# SoliDB Android SDK Integration

## Quick Start

1. Copy `solidb_client.aar` to your app's `libs/` directory

2. Add to your app's `build.gradle.kts`:
   ```kotlin
   dependencies {
       implementation(files("libs/solidb_client.aar"))
   }
   ```

3. Use in your Kotlin code:
   ```kotlin
   import com.solidb.client.*
   
   val config = SyncConfig(
       deviceId = Utils.generateDeviceId(),
       serverUrl = "https://your-server.com:6745",
       apiKey = "your-api-key",
       collections = listOf("todos"),
       syncIntervalSecs = 30,
       maxRetries = 5,
       autoSync = true
   )
   
   val syncManager = SyncManager(config)
   syncManager.start()
   ```

## JNI Libraries

The AAR contains native libraries for these ABIs:
- `arm64-v8a` (aarch64-linux-android)
- `armeabi-v7a` (armv7-linux-androideabi)
- `x86_64` (x86_64-linux-android)

## See Also

- Full documentation: https://solidb.io/docs/mobile
- Example app: clients/android-example/
EOF
    
    log_success "Integration README created"
}

# Clean up temporary files
cleanup() {
    log_info "Cleaning up temporary files..."
    
    # Keep only the AAR and important files
    local temp_dir="${BUILD_DIR}/temp_$$"
    mkdir -p "$temp_dir"
    
    # Save important files
    mv "${BUILD_DIR}/solidb_client.aar" "$temp_dir/" 2>/dev/null || true
    mv "${BUILD_DIR}/build.gradle.kts" "$temp_dir/" 2>/dev/null || true
    mv "${BUILD_DIR}/README.md" "$temp_dir/" 2>/dev/null || true
    mv "${BUILD_DIR}/kotlin" "$temp_dir/" 2>/dev/null || true
    
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
    echo "  Android Build Complete"
    echo "========================================"
    echo ""
    echo "Output: ${BUILD_DIR}/solidb_client.aar"
    echo ""
    echo "Kotlin bindings: ${BUILD_DIR}/kotlin/"
    ls -la "${BUILD_DIR}/kotlin/" 2>/dev/null || echo "  (Kotlin bindings generated)"
    echo ""
    echo "To use in your project:"
    echo "  1. Copy solidb_client.aar to your app's libs/ directory"
    echo "  2. Add to build.gradle.kts: implementation(files(\"libs/solidb_client.aar\"))"
    echo "  3. See ${BUILD_DIR}/README.md for more details"
    echo ""
}

# Main build process
main() {
    echo "========================================"
    echo "  SoliDB Android AAR Builder"
    echo "========================================"
    echo ""
    
    check_prerequisites
    check_rust_targets
    clean_build_dir
    
    # Build for all Android targets
    build_for_android_target "$ANDROID_ARM64_TARGET" "arm64-v8a"
    build_for_android_target "$ANDROID_ARMV7_TARGET" "armeabi-v7a"
    build_for_android_target "$ANDROID_X86_64_TARGET" "x86_64"
    
    # Generate Kotlin bindings
    generate_kotlin_bindings
    
    # Create AAR structure
    create_aar_structure
    
    # Create manifest
    create_manifest
    
    # Create AAR file
    create_aar_file
    
    # Create integration files
    create_build_gradle
    create_integration_readme
    
    # Cleanup
    cleanup
    
    # Print summary
    print_summary
}

# Run main
main "$@"
