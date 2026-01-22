#!/bin/bash
# Bundle OllamaBar into a macOS app bundle
# Usage: ./scripts/bundle-app.sh [--sign IDENTITY] [--notarize]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
APP_NAME="OllamaBar"
BUNDLE_ID="com.offquant.ollamabar"

# Output directories
BUILD_DIR="$PROJECT_ROOT/target/release"
DIST_DIR="$PROJECT_ROOT/dist"
APP_BUNDLE="$DIST_DIR/$APP_NAME.app"

# Parse arguments
SIGN_IDENTITY=""
DO_NOTARIZE=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --sign)
            SIGN_IDENTITY="$2"
            shift 2
            ;;
        --notarize)
            DO_NOTARIZE=true
            shift
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

echo "Building OllamaBar release..."
cargo build --release -p ollama-bar

echo "Creating app bundle..."
rm -rf "$APP_BUNDLE"
mkdir -p "$APP_BUNDLE/Contents/MacOS"
mkdir -p "$APP_BUNDLE/Contents/Resources"

# Copy binary
cp "$BUILD_DIR/ollama-bar" "$APP_BUNDLE/Contents/MacOS/"

# Copy Info.plist
cp "$PROJECT_ROOT/crates/ollama-bar/assets/Info.plist" "$APP_BUNDLE/Contents/"

# Copy icon if it exists
if [[ -f "$PROJECT_ROOT/crates/ollama-bar/assets/AppIcon.icns" ]]; then
    cp "$PROJECT_ROOT/crates/ollama-bar/assets/AppIcon.icns" "$APP_BUNDLE/Contents/Resources/"
fi

echo "App bundle created at: $APP_BUNDLE"

# Code signing
if [[ -n "$SIGN_IDENTITY" ]]; then
    echo "Signing app bundle..."

    # Sign with hardened runtime (required for notarization)
    codesign --force --options runtime --sign "$SIGN_IDENTITY" \
        --entitlements "$PROJECT_ROOT/crates/ollama-bar/assets/entitlements.plist" \
        "$APP_BUNDLE"

    echo "Verifying signature..."
    codesign --verify --verbose=4 "$APP_BUNDLE"

    # Check for notarization requirements
    spctl --assess --type execute --verbose=4 "$APP_BUNDLE" || true
fi

# Notarization
if [[ "$DO_NOTARIZE" == true ]]; then
    if [[ -z "$SIGN_IDENTITY" ]]; then
        echo "Error: --sign IDENTITY is required for notarization"
        exit 1
    fi

    echo "Creating zip for notarization..."
    ZIP_FILE="$DIST_DIR/$APP_NAME.zip"
    ditto -c -k --keepParent "$APP_BUNDLE" "$ZIP_FILE"

    echo "Submitting for notarization..."
    echo "Note: Ensure APPLE_ID, APPLE_TEAM_ID, and app-specific password are configured"
    echo "Run: xcrun notarytool store-credentials --apple-id YOUR_APPLE_ID --team-id YOUR_TEAM_ID"

    # Submit for notarization
    xcrun notarytool submit "$ZIP_FILE" --keychain-profile "notarytool-profile" --wait

    echo "Stapling ticket..."
    xcrun stapler staple "$APP_BUNDLE"

    echo "Verifying notarization..."
    spctl --assess --type execute --verbose=4 "$APP_BUNDLE"

    # Recreate zip with stapled app
    rm "$ZIP_FILE"
    ditto -c -k --keepParent "$APP_BUNDLE" "$ZIP_FILE"

    echo "Notarization complete!"
fi

# Create DMG
echo "Creating DMG..."
DMG_FILE="$DIST_DIR/$APP_NAME.dmg"
rm -f "$DMG_FILE"

hdiutil create -volname "$APP_NAME" \
    -srcfolder "$APP_BUNDLE" \
    -ov -format UDZO \
    "$DMG_FILE"

if [[ -n "$SIGN_IDENTITY" ]]; then
    echo "Signing DMG..."
    codesign --force --sign "$SIGN_IDENTITY" "$DMG_FILE"
fi

echo ""
echo "Build complete!"
echo "  App bundle: $APP_BUNDLE"
echo "  DMG: $DMG_FILE"
ls -lh "$DMG_FILE"
