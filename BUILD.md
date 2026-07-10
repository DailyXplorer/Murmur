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

This builds the `Murmur` executable, creates `~/Applications/MurmurDist/Murmur.app`, signs it locally, and launches it.

Set `MURMUR_DIST_DIR` to stage the app somewhere else. Earlier versions staged the app under `/tmp`, but macOS purges `/tmp` at reboot, so the app bundle (and any launch-at-login entry pointing at it) silently disappeared after a restart.

## Smoke Checks

```bash
MURMUR_PORTABLE_SMOKE=1 ./script/build_and_run.sh --verify
MURMUR_PORTABLE_SMOKE=1 ./script/build_and_run.sh --readiness
```

`MURMUR_PORTABLE_SMOKE=1` makes the staged app use isolated app data under `~/Applications/MurmurDist/`.

## Local Archives

```bash
MURMUR_PORTABLE_SMOKE=1 ./script/build_and_run.sh --release-readiness
```

This creates ZIP and DMG artifacts under `~/Applications/MurmurDist/archive/` and validates the app signature plus archive contents.

## Stable Local Signing

By default, when no suitable identity is found, the build script signs the app ad-hoc (`-`). An ad-hoc signature has no stable identity, so every rebuild produces a binary that macOS TCC no longer recognizes: the Accessibility toggle in System Settings still looks ON, but `AXIsProcessTrusted()` returns `false` and the global dictation shortcut silently stops working until the grant is redone.

To keep the Accessibility grant across rebuilds, create a self-signed code-signing certificate named `Murmur Dev`:

1. Open Keychain Access and choose Keychain Access → Certificate Assistant → Create a Certificate.
2. Set Name to `Murmur Dev`.
3. Set Identity Type to "Self-Signed Root".
4. Set Certificate Type to "Code Signing".
5. Create the certificate.

The build script detects identities in this order:

1. `MURMUR_CODESIGN_IDENTITY` (always wins if set)
2. A `Murmur Dev` identity
3. The first `Apple Development` identity
4. Ad-hoc signing (`-`), with a loud warning

After any signature change (for example the first build after creating the certificate, or switching identities), macOS treats the app as a new program: remove Murmur from System Settings → Privacy & Security → Accessibility and add it back, then relaunch the app.

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

- `MURMUR_DIST_DIR` - staged app and archive output directory, defaults to `~/Applications/MurmurDist` (the old default under `/tmp` was purged at every reboot, which silently broke launch-at-login)
- `MURMUR_ARCHIVE_DIR` - archive output directory, defaults to `$MURMUR_DIST_DIR/archive`
- `MURMUR_BUNDLE_ID` - bundle identifier, defaults to `com.pais.murmur`
- `MURMUR_APP_VERSION` - app version, defaults to `0.1.0`
- `MURMUR_APP_BUILD` - build number, defaults to `1`
- `MURMUR_CODESIGN_IDENTITY` - signing identity, overrides the detection cascade described in "Stable Local Signing"; without it the script falls back to `Murmur Dev`, then `Apple Development`, then ad-hoc signing (`-`)
- `MURMUR_ENTITLEMENTS_PLIST` - entitlements path, defaults to `Resources/Entitlements.plist`
