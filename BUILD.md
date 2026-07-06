# Build Instructions

Murmur for Mac is built as a native macOS SwiftPM app. The built app bundle and executable are still named `Murmur`.

## Prerequisites

- macOS 14 or newer
- Xcode Command Line Tools

```bash
xcode-select --install
```

No previous cross-platform toolchain is required.

## Test

```bash
swift test -debug-info-format none
```

## Run

```bash
./script/build_and_run.sh
```

This builds the `Murmur` executable, creates `/tmp/murmur-native-dist/Murmur.app`, signs it locally, and launches it.

## Smoke Checks

```bash
MURMUR_PORTABLE_SMOKE=1 ./script/build_and_run.sh --verify
MURMUR_PORTABLE_SMOKE=1 ./script/build_and_run.sh --readiness
```

`MURMUR_PORTABLE_SMOKE=1` makes the staged app use isolated app data under `/tmp/murmur-native-dist/`.

## Local Archives

```bash
MURMUR_PORTABLE_SMOKE=1 ./script/build_and_run.sh --release-readiness
```

This creates ZIP and DMG artifacts under `/tmp/murmur-native-dist/archive/` and validates the app signature plus archive contents.

## Optional Notarization

For local use, ad-hoc signing is enough. For distributing a prebuilt app outside your machine, use a Developer ID Application certificate and Apple notarization credentials:

```bash
MURMUR_CODESIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)" \
MURMUR_NOTARY_KEYCHAIN_PROFILE="your-notarytool-profile" \
./script/build_and_run.sh --notarize
```

The script also accepts Apple ID credentials through:

- `MURMUR_NOTARY_APPLE_ID`
- `MURMUR_NOTARY_TEAM_ID`
- `MURMUR_NOTARY_PASSWORD`

## Useful Environment Variables

- `MURMUR_DIST_DIR` - staged app and archive output directory, defaults to `/tmp/murmur-native-dist`
- `MURMUR_ARCHIVE_DIR` - archive output directory, defaults to `$MURMUR_DIST_DIR/archive`
- `MURMUR_BUNDLE_ID` - bundle identifier, defaults to `com.pais.murmur`
- `MURMUR_APP_VERSION` - app version, defaults to `0.1.0`
- `MURMUR_APP_BUILD` - build number, defaults to `1`
- `MURMUR_CODESIGN_IDENTITY` - signing identity, defaults to ad-hoc signing (`-`) when no local identity is configured
- `MURMUR_ENTITLEMENTS_PLIST` - entitlements path, defaults to `Resources/Entitlements.plist`
