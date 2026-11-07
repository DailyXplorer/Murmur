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

### Compare the macOS crate of two commits

`scripts/macos-expanded-diff.sh <base> <head>` expands the `murmur` library of
two commits (`cargo rustc --lib -- -Zunpretty=expanded`, a debug build, no
bundle) and prints the `diff -u` of the result. It runs on macOS only, takes a
full build the first time (the Swift meetings helper included), and exits 0 when
the two are identical. The manual `mac expanded diff` workflow runs it on a
runner. See "How to prove a change is a no-op on macOS" in `AGENTS.md`.

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

### Update manifest (`latest.json`)

One `latest.json` on the latest release serves every client
(`.github/scripts/publish-updater-manifest.mjs`, key rules in
`updater-targets.mjs`, signature check in `updater-signature.mjs`).

| Key                                     | Built from                           | Points at     |
| --------------------------------------- | ------------------------------------ | ------------- |
| `darwin-aarch64-app`, `darwin-aarch64`  | `Murmur_aarch64.app.tar.gz.sig`      | `.app.tar.gz` |
| `darwin-x86_64-app`, `darwin-x86_64`    | `Murmur_x64.app.tar.gz.sig`          | `.app.tar.gz` |
| `windows-x86_64-nsis`, `windows-x86_64` | `Murmur_<version>_x64-setup.exe.sig` | `-setup.exe`  |

- The four `darwin-*` keys are always required. The two `windows-*` keys are
  required when `INCLUDE_WINDOWS` is `true` (the default); with `false` the
  manifest is the macOS-only one, byte for byte, and Windows assets are ignored.
- Any other key is an error, and so is a `darwin-*` key that does not end in
  `.app.tar.gz`, a `windows-*` key that does not end in `-setup.exe`, or a URL
  outside `https://github.com/<repo>/releases/download/<tag>/`. The updater
  plugin looks up `<os>-<arch>-<bundle>`, then `<os>-<arch>`, so a Mac never
  reads a `windows-*` key and is never offered an installer.
- `--verify-public` checks the downloaded feed (`VERSION`, `MANIFEST`,
  optionally `GITHUB_REPOSITORY` and `RELEASE_TAG`). `--verify-signatures <dir>`
  checks every entry against the asset of the same name in `<dir>`, offline,
  with the `pubkey` of `src-tauri/tauri.conf.json`: key id, signature, the
  signed comment, its `version:` equal to the release and its `file:` equal to
  the asset (the Mac archive is signed as `Murmur.app.tar.gz` before the
  upload adds the architecture).
- `--emit-legacy-feed <path>` writes the Windows-only manifest for the old
  `windows-beta` feed (same installer and signature as the release), and
  `--verify-public-legacy` checks a downloaded copy.
- Tests: `bun run test:updater-manifest`.

## Releasing

One workflow, `.github/workflows/release.yml`, releases Murmur for macOS and
Windows (beta) together. Only the repository owner can start it, on `main`,
from Actions, Release, Run workflow. It builds the exact commit of `main` the
run started on.

| Input     | Value              | Effect                                                                                                                                                              |
| --------- | ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `windows` | `true` (default)   | Builds, smoke-tests and signs the Windows installer and puts the `windows-*` keys in `latest.json`. A failed Windows build stops the release before it goes public. |
| `windows` | `false`            | A Mac-only release, as before Windows joined: no Windows job, a darwin-only `latest.json`, the `windows-beta` pointer untouched.                                    |
| `channel` | `stable` (default) | The release the update feed and Latest point at. The version is `x.y.z`.                                                                                            |
| `channel` | `test`             | A prerelease on the `updater-test` feed, never Latest, never the stable feed. The version is `x.y.z-test.N`.                                                        |

**Stable release.**

1. Prepare the version in the five places `AGENTS.md` lists, with
   `src/content/release-notes/<version>.md` holding a "Windows (beta)" section
   in user terms: what a PC gets, that it is a beta, and that the installer has
   no Windows certificate. Merge it.
2. Run the workflow with `channel` `stable`. `validate-release-source` pins
   `main` and checks the version, `create-release` makes the draft `v<version>`,
   `publish-tauri` (the two Mac builds, unchanged) and `publish-windows`
   (`build-windows.yml`) run in parallel, then `publish-release` runs.
3. `publish-release` builds one `latest.json`, downloads the assets it points
   at and checks every signature against the updater `pubkey` while the release
   is still a draft. It publishes the draft as Latest, checks the public
   `releases/latest/download/latest.json`, points the legacy `windows-beta`
   feed at the same Windows installer and checks that feed too.

**Test channel.** `channel` `test` needs the version `x.y.z-test.N` in the five
version files (the release notes are those of `x.y.z`). The Mac and Windows
builds read `src-tauri/updater-test.conf.json`, whose only content is the
endpoint `releases/download/updater-test/latest.json`. The release is the
prerelease `v<version>`, and `latest.json` goes to the rolling prerelease
`updater-test`, created on the first run. To try the updater end to end:
release `x.y.z-test.1`, install it by hand on a Mac and a PC, release
`x.y.z-test.2`, and let both update from the app.
`x.y.z-test.1 < x.y.z-test.2 < x.y.z`, so the stable `x.y.z` supersedes both.
A Mac build with a pre-release version has not shipped before: run the test
channel before the first stable release of this workflow.

**Rollback.**

- The Mac side is the usual one: mark the release a draft or delete it, and
  `releases/latest` falls back to the previous release.
- The `windows-beta` pointer: each stable run uploads the file it replaces as
  the artifact `windows-beta-feed-before-<version>` (kept 90 days). To go back,
  download it and run `gh release upload windows-beta latest.json --clobber`.
  PCs that already updated cannot be downgraded: ship the next number.
- A run that fails before "Publish release" leaves a draft only. Delete the
  draft and its tag, then run again.

**Dry run.** A pull request that touches the release workflows,
`.github/scripts`, `src-tauri/tauri*.json` or `src-tauri/updater-*.json` runs
`release-dry-run.yml`. It builds a Windows installer, signs it with a key made
in the job and thrown away with it, runs the unified manifest builder on fake
Mac archives and that installer, and verifies the result. It uses no repository
secret and uploads nothing.

**The Mac path is pinned.** `bun run test:release-contract` compares
`release.yml` and `build.yml` with
`scripts/fixtures/release-contract-v1.12.1.json`, taken from the workflows of
v1.12.1: the Mac jobs, their secrets, environments and matrix, the
`make_latest` step and the signing steps of `build.yml` must not change. The
one deliberate difference from v1.12.1 is the cache step of `build.yml`: the
separate `actions/cache` step for the meetings helper is gone, and the
SwiftPM `.build` directory is cached by the `swatinem/rust-cache` step
(`cache-directories`, aarch64 only) which saves only on a push to main. This
closes a CodeQL cache-poisoning alert (no default-branch cache is written from
a caller-supplied ref); the macOS build output is unchanged. The release
workflow may add Windows and test-channel jobs and `if` guards. Changing a pin
is a deliberate edit of that file, with the reason in the pull request.

### Windows update feed

The Windows app reads the same feed as the Mac app: `tauri.windows.conf.json`
has no endpoint (`bun run check:platforms` fails if it gets one). The
`windows-beta` feed stays for installs older than the unified release (Beta 7
and 8 read it). Every stable release with `windows` on rewrites its
`latest.json` to the Windows entries of the unified manifest, with the same
installer and signature, so a beta that stayed offline for months jumps to the
newest build. The `windows-beta` and `v1.9.0-beta.*` releases are never
deleted. The feed tooling is `.github/scripts/windows-updater-manifest.mjs`
with `minisign.mjs`; its tests are `bun run test:windows-updater`.

**The first update.** Beta 6 and older have no updater, so a build with one is
installed by hand once. The installer has no Authenticode certificate, so
SmartScreen still asks on an install by hand.

## Continuous integration

Workflows live in `.github/workflows`.

### Rust caches

`swatinem/rust-cache` restores a cache on every run but **only a push to
`main` saves one** (`save-if: ${{ github.ref == 'refs/heads/main' }}`, as in
`build.yml`). A pull request reads the cache `main` saved and writes nothing.
Before this, every pull request uploaded its own 1 to 1.4 GB copy of each
cache: the repository sat above the 10 GB limit, GitHub evicted the caches of
`main` first, and about one run in five started cold (9 to 12 minutes slower).
Cargo's incremental cache is not worth uploading from a branch that is gone
after the merge.

The Windows jobs that compile the crate for tests share one cache,
`windows-dev`, which only the `rust` job of `windows.yml` saves (on `main`);
`cpu-model`, `meetings-audio` and the `meetings windows` jobs restore it. The
installer builds in the release profile with its own `windows-release` cache,
which `release dry run` reads.

`cache-on-failure: true` keeps a cache from a main run whose tests failed, so a
flaky test does not discard a compile of 10 minutes or more. A workflow that
has no run on `main` has no cache to read: `local engine` therefore also runs
on a push to `main` that changes its own paths.

### Windows workflow

`windows.yml` starts with a `changes` job that lists the files a pull request
changes (`.github/scripts/changed-paths.sh`, tested by
`bun run test:changed-paths`) and gives each job a regular expression: the Rust
jobs (`rust`, `cpu-model`, `meetings-audio`) run for `src-tauri/**`, the
locale files, `src/bindings.ts` and the Windows scripts; the release-profile
`installer` and `smoke` run for what changes the release build or the app's
start (`tauri*.json`, `Cargo.*`, `build.rs`, `lib.rs`, `main.rs`, `mcp/`,
`package.json`, the Windows scripts). Anything that is not a pull request
(push to `main`, the nightly run at 02:23 UTC, a manual run) runs every job, and
so does a diff that cannot be read. `windows-ok` fails when any job failed or
was cancelled and passes when the others were skipped, so it is the one name to
require if a branch ruleset is ever added (the leaf jobs are conditional).
`code quality` runs on every pull request, since it is the only check of
Markdown formatting and the platform rules.

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
