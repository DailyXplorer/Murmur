# Build Instructions

Handy is built as a native macOS SwiftPM app.

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

This builds the `Handy` executable, creates `/tmp/handy-native-dist/Handy.app`, signs it locally, and launches it.

## Smoke Checks

```bash
HANDY_PORTABLE_SMOKE=1 ./script/build_and_run.sh --verify
HANDY_PORTABLE_SMOKE=1 ./script/build_and_run.sh --readiness
```

`HANDY_PORTABLE_SMOKE=1` makes the staged app use isolated app data under `/tmp/handy-native-dist/`.

## Local Archives

```bash
HANDY_PORTABLE_SMOKE=1 ./script/build_and_run.sh --release-readiness
```

This creates ZIP and DMG artifacts under `/tmp/handy-native-dist/archive/` and validates the app signature plus archive contents.

## Optional Notarization

For local use, ad-hoc signing is enough. For distributing a prebuilt app outside your machine, use a Developer ID Application certificate and Apple notarization credentials:

```bash
HANDY_CODESIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)" \
HANDY_NOTARY_KEYCHAIN_PROFILE="your-notarytool-profile" \
./script/build_and_run.sh --notarize
```

The script also accepts Apple ID credentials through:

- `HANDY_NOTARY_APPLE_ID`
- `HANDY_NOTARY_TEAM_ID`
- `HANDY_NOTARY_PASSWORD`

## Useful Environment Variables

- `HANDY_DIST_DIR` - staged app and archive output directory, defaults to `/tmp/handy-native-dist`
- `HANDY_ARCHIVE_DIR` - archive output directory, defaults to `$HANDY_DIST_DIR/archive`
- `HANDY_BUNDLE_ID` - bundle identifier, defaults to `com.pais.handy`
- `HANDY_APP_VERSION` - app version, defaults to `0.1.0`
- `HANDY_APP_BUILD` - build number, defaults to `1`
- `HANDY_CODESIGN_IDENTITY` - signing identity, defaults to ad-hoc signing (`-`) when no local identity is configured
- `HANDY_ENTITLEMENTS_PLIST` - entitlements path, defaults to `Resources/Entitlements.plist`
