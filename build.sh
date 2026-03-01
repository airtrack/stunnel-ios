#!/bin/bash

# stunnel-ios Build Script
# usage: ./build.sh [--debug | --release]

set -e

# --- Configuration ---
PROJECT_NAME="stunnel-ios"
RUST_CORE_DIR="rust-core"
IOS_DIR="stunnel-ios"
MODE="release"
CARGO_FLAG="--release"

# Parse arguments
for arg in "$@"
do
    case $arg in
        --debug)
        MODE="debug"
        CARGO_FLAG=""
        shift
        ;;
        --release)
        MODE="release"
        CARGO_FLAG="--release"
        shift
        ;;
    esac
done

echo "------------------------------------------------"
echo "🚀 Starting build for $PROJECT_NAME (Mode: $MODE)"
echo "------------------------------------------------"

# 1. Build Rust Core
echo "📦 Building Rust core library..."
cd $RUST_CORE_DIR

echo "  -> Target: aarch64-apple-ios-sim (Simulator)"
cargo build --target aarch64-apple-ios-sim $CARGO_FLAG

echo "  -> Target: aarch64-apple-ios (Device)"
# Note: This may fail on some environments without full iOS SDK path configuration
cargo build --target aarch64-apple-ios $CARGO_FLAG || echo "⚠️  Warning: Device build failed. Simulator build should still work."

cd ..

# 2. Generate Xcode Project
echo "🛠️  Generating Xcode project with xcodegen..."
cd $IOS_DIR
xcodegen generate

# 3. Build Xcode Project (Simulator)
echo "🏗️  Building iOS App for Simulator..."
# We use a generic destination to ensure it builds even if a specific simulator isn't booted
xcodebuild build 
    -scheme stunnel-ios 
    -destination 'generic/platform=iOS Simulator' 
    CODE_SIGNING_ALLOWED=YES 
    CODE_SIGNING_REQUIRED=NO 
    CODE_SIGN_IDENTITY="-" 
    | xcbeautify || xcodebuild build 
    -scheme stunnel-ios 
    -destination 'generic/platform=iOS Simulator' 
    CODE_SIGNING_ALLOWED=YES 
    CODE_SIGN_REQUIRED=NO 
    CODE_SIGN_IDENTITY="-"

echo "------------------------------------------------"
echo "✅ Build Complete!"
echo "Open $IOS_DIR/$PROJECT_NAME.xcodeproj to run in Xcode."
echo "------------------------------------------------"
