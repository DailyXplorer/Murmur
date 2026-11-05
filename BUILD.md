# Build Instructions

This guide covers how to set up the development environment and build Murmur
from source on macOS.

## Prerequisites

- [Rust](https://rustup.rs/) (latest stable)
- [Bun](https://bun.sh/) package manager
- [Tauri Prerequisites](https://tauri.app/start/prerequisites/)
- Xcode Command Line Tools
- On Apple silicon, Xcode 26 or later (Swift 6.2 or later): `src-tauri/build.rs`
  builds the meetings helper (`src-tauri/swift/MurmurMeetingsHelper`) with
  SwiftPM. The first build downloads FluidAudio from GitHub. Intel builds use
  a small C stub instead.

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

The release workflow signs the app with the first certificate it finds in the
`release-signing` environment secrets:

1. An Apple Developer ID certificate (`APPLE_CERTIFICATE`). The workflow
   codesigns with the hardened runtime and `Entitlements.plist`, and notarizes
   the bundle when the Apple ID secrets are also present.
2. The stable self-signed Murmur certificate (`MACOS_CERT_P12_BASE64`). See
   [Stable release signing](#stable-release-signing).
3. Neither. The workflow signs ad hoc, like a local build.

Pull request and main-branch test builds always sign ad hoc and need no
secrets. Updater artifacts are signed with the Tauri updater key so the in-app
updater can verify downloads.

Only a Developer ID build is notarized. Gatekeeper asks for confirmation the
first time a user opens a self-signed or ad hoc download.

### Stable release signing

macOS stores Accessibility and microphone grants against the app's designated
requirement. An ad hoc signature puts the build's `cdhash` in that requirement,
so every update looks like a new app and loses its grants. A certificate
signature puts the certificate's hash in the requirement instead. As long as
every release uses the same certificate, the grants survive updates.

Set up the certificate once:

1. On a Mac, outside any git checkout, create the certificate:

   ```bash
   scripts/create-signing-cert.sh ~/murmur-signing
   ```

   The script writes a PKCS#12 file, its base64 form, a random password, and
   the identity name, valid for 10 years. It refuses to write inside a git
   work tree.

2. In the GitHub repository, open **Settings > Environments >
   release-signing** and add three secrets:

   | Secret                   | Value                                       |
   | ------------------------ | ------------------------------------------- |
   | `MACOS_CERT_P12_BASE64`  | the contents of `murmur-signing.p12.base64` |
   | `MACOS_CERT_PASSWORD`    | the contents of `murmur-signing.password`   |
   | `MACOS_SIGNING_IDENTITY` | the contents of `murmur-signing.identity`   |

   With the GitHub CLI:

   ```bash
   gh secret set MACOS_CERT_P12_BASE64 --env release-signing < ~/murmur-signing/murmur-signing.p12.base64
   gh secret set MACOS_CERT_PASSWORD --env release-signing < ~/murmur-signing/murmur-signing.password
   gh secret set MACOS_SIGNING_IDENTITY --env release-signing < ~/murmur-signing/murmur-signing.identity
   ```

3. Store `murmur-signing.p12` and its password in a password manager. If the
   certificate is lost, the next release needs a new one, and every user must
   grant permissions again.

4. Delete `~/murmur-signing`.

The release job imports the certificate into a temporary keychain, signs with
it, and deletes the keychain at the end of the job. A self-signed certificate
is not trusted by the system, so `security find-identity -v` does not list it.
`codesign` still signs with it.

To check a release, inspect its designated requirement. It names the
certificate leaf hash, not a `cdhash`, and the hash stays the same across
releases:

```bash
codesign -dr - /Applications/Murmur.app 2>&1
```

Install official builds from [GitHub Releases](https://github.com/DailyXplorer/Murmur/releases).

## Windows beta releases

A Windows beta is published by pushing a tag, from the `beta` branch. The
installer is signed with the updater key (the one of the macOS releases) and
the Windows app finds it through the `windows-beta` feed. Windows work is
described in `AGENTS.md`, "Windows beta updates".

1. **Release-prep PR into `beta`**, titled `chore(release): prepare
v1.9.0-beta.N (Windows)`. It sets `1.9.0-beta.N` in `package.json`,
   `src-tauri/Cargo.toml`, the `murmur` entry of `src-tauri/Cargo.lock` and
   `src-tauri/tauri.conf.json`. Do not add a release-notes file: the beta uses
   `src/content/release-notes/1.9.0.md`. `bun run check:versions` must pass,
   and so must the `windows beta` run on the merged head.
2. **Write the notes** in a file, in the words of a tester: what is new. The
   workflow adds the title, "Not on Windows yet", the install steps, the update
   paragraph and the SHA-256.
3. **Tag the merged head** with an annotated tag whose message is that file, and
   push it. The tag must equal `v` plus the version in `tauri.conf.json`.

   ```bash
   git tag -a v1.9.0-beta.7 -F notes.md <sha-on-beta>
   git push origin v1.9.0-beta.7
   ```

4. **Watch `windows beta release`**: `build`, `manifest`, `publish`, `verify`.
   `build` refuses a tag that is not `vX.Y.Z-beta.N`, a tag pushed by someone
   other than the repository owner, a tag that differs from the version in
   `tauri.conf.json`, a commit that is not on `beta`, and a release that
   already exists. `verify` reads what GitHub serves: the feed, the installer
   and its signature, the checksum in the notes, the macOS feed (still
   darwin-only) and the latest release (still not a beta).
5. **Check by hand** that `gh release list` shows `v1.9.0-beta.N` and
   `windows-beta` as pre-releases and the macOS release as latest.

A pull request that changes `beta-release.yml`, the feed tooling or the Tauri
configs runs the same build, signature and feed checks as a dry run and
publishes nothing. Its build uses a `-beta.0` version while the files still
carry the plain one.

**Rollback.** A bad beta cannot be taken back from the PCs that installed it:
ship the next number. To stop offering it, upload the previous beta's
`latest.json` (it is an asset of that beta's release) to the feed with
`gh release upload windows-beta latest.json --clobber`, and delete the bad
versioned release if needed.

**The first update.** Beta 6 and older have no updater. The first beta that
updates itself is installed by hand once; the next beta is the first
end-to-end update test.

The installer has no Authenticode certificate, so SmartScreen still asks on an
install by hand.

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
services. Releases signed with the stable certificate need it at most once, after the
first update from an ad hoc release.

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
