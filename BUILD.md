# Build Instructions

This guide covers how to set up the development environment and build Murmur
from source on macOS.

## Prerequisites

- [Rust](https://rustup.rs/) (latest stable)
- [Bun](https://bun.sh/) package manager
- [Tauri Prerequisites](https://tauri.app/start/prerequisites/)
- Xcode Command Line Tools

Install the Command Line Tools with:

```bash
xcode-select --install
```

## Setup Instructions

### 1. Clone the Repository

```bash
git clone git@github.com:DailyXplorer/Murmur.git
cd Murmur
```

### 2. Install Dependencies

```bash
bun install
```

### 3. Start Dev Server

```bash
bun tauri dev
```

If CMake rejects an old dependency policy:

```bash
CMAKE_POLICY_VERSION_MINIMUM=3.5 bun run tauri dev
```

### 4. Build for Production

```bash
bun run tauri build
```

This compiles a release binary and generates a macOS app bundle and DMG under
`src-tauri/target/release/bundle/`.

## Code signing and notarization

Local development builds use the ad-hoc `signingIdentity: "-"` in
`src-tauri/tauri.conf.json`. That is enough to run the app on the Mac that
built it.

Official GitHub Actions releases import an Apple Developer ID certificate,
codesign the app with the hardened runtime and `Entitlements.plist`, and
notarize the bundle with Apple when the corresponding secrets are present.
Updater artifacts are signed with the Tauri updater key so the in-app updater
can verify downloads.

Install official builds from [GitHub Releases](https://github.com/DailyXplorer/Murmur/releases)
so Gatekeeper accepts them. Ad-hoc local rebuilds are not notarized.

## Troubleshooting

### macOS Accessibility remains enabled after a local rebuild

Local builds use the ad-hoc `signingIdentity: "-"`. A rebuild can have a new macOS code
identity while the old **System Settings > Privacy & Security > Accessibility** entry
remains visibly enabled, leaving Murmur on `Waiting...`.

After installing the final bundle at `/Applications/Murmur.app`, quit Murmur, clear only its
stale Accessibility record, then reopen it:

```bash
osascript -e 'tell application id "com.dailyxplorer.murmur" to quit' || true
tccutil reset Accessibility com.dailyxplorer.murmur
open /Applications/Murmur.app
```

Grant Accessibility again when prompted. This does not reset Microphone or other TCC
services, and official releases normally do not need it.

For optional diagnosis, compare the designated requirements of the previous and rebuilt
bundles:

```bash
codesign -dr - /path/to/previous/Murmur.app 2>&1
codesign -dr - /Applications/Murmur.app 2>&1
```

An ad-hoc requirement contains a `cdhash`; a changed requirement confirms the rebuild is
not covered by the old grant. The reset procedure does not require this check.

See [issue #1618](https://github.com/DailyXplorer/Murmur/issues/1618) for the related onboarding
and stale-permission report.
