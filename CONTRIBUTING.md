# Contributing to Murmur

Keep each change focused. Explain why it is needed, test the behavior it affects, and link the issue it closes. By contributing, you agree that your contribution is licensed under the [MIT License](LICENSE).

## Setup

Install [Rust](https://rustup.rs/) (the repo pins 1.96.0 in `rust-toolchain.toml`), [Bun](https://bun.sh/), and the macOS tools in [BUILD.md](BUILD.md).

```bash
git clone git@github.com:YOUR_USERNAME/Murmur.git
cd Murmur
bun install
```

`bun install` installs dependencies and points the clone at `.githooks/pre-commit`.

Run the app with `bun run tauri dev`. If CMake rejects an old policy: `CMAKE_POLICY_VERSION_MINIMUM=3.5 bun run tauri dev`.

## Checks

`bun run check` runs the same checks as the CI `code quality` job, in the same order:

- `bun audit`
- translation consistency, the macOS-only guard, the updater manifest test, and the release-version check
- ESLint, Prettier, and `cargo fmt --check`
- the frontend build and the Playwright typecheck

On macOS it also runs Clippy with the flags from the macOS `test` job (`cargo clippy --locked --all-targets --all-features -- -D warnings`). On Linux that step is skipped, because the crate compiles on macOS only. CI still runs Clippy on the macOS runner.

Two other CI jobs are separate:

- UI changes: `bun run test:playwright` (the Playwright job).
- Rust behavior: from `src-tauri`, `cargo test --locked` (the macOS test job, which also checks icons and runs `cargo audit`).

The pre-commit hook runs a faster subset (lint, Prettier, translations, the macOS-only guard, versions, and `cargo fmt --check` when a Rust file is staged). Let it run. If you skip it, say why in the pull request.

## What to test

Cover the behavior you changed. A bug fix needs a test that fails without the fix. UI changes need a Playwright spec or an update to one that already covers that screen. Rust logic needs a `cargo test`. Say which commands you ran and what happened.

## Branches and commits

Branch from `main` as `type/short-description`, using a Conventional Commits type: `feat/`, `fix/`, `docs/`, `refactor/`, `test/`, or `chore/`.

Commit titles use the same types (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`) and say why.

Open an issue before a large pull request: a new feature, a change that crosses several modules, or anything you cannot describe in a few sentences. A fix for a bug you can reproduce can go straight to a pull request that links the report.

Use the [bug report form](.github/ISSUE_TEMPLATE/bug_report.yml) and the [feature request form](.github/ISSUE_TEMPLATE/feature_request.yml). Open an issue when you want feedback before deciding.

## Good first issues

Issues labeled [good first issue](https://github.com/DailyXplorer/Murmur/issues?q=is%3Aopen+is%3Aissue+label%3A%22good+first+issue%22) are scoped for someone new to the repo. [Help wanted](https://github.com/DailyXplorer/Murmur/issues?q=is%3Aopen+is%3Aissue+label%3A%22help+wanted%22) is the wider list. Comment on the issue before you start so two people do not take the same one.

## macOS on main, Windows on beta

`main` builds for macOS only. The `compile_error!` at the top of `src-tauri/src/lib.rs` keeps it that way. Windows work stays on the `beta` branch. Do not add Windows targets, `cfg(target_os)`, or a Windows CI job on `main`.

## Pull requests

The template asks for why (Summary, with the issue link), what (Changes), how you tested it (Testing, including `bun run check`), and screenshots or recordings for a visible UI change.

- User-facing JSX text goes through i18next. Add the English key and update the other locales. See [CONTRIBUTING_TRANSLATIONS.md](CONTRIBUTING_TRANSLATIONS.md).
- A version bump updates `package.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json`, and `src/content/release-notes/<version>.md` together.
- Leave generated `src/bindings.ts` and unrelated lockfile churn alone.
- Comments should say why (an invariant, a platform quirk, FFI ownership). Update them in the same change so they do not drift.

## Review rounds

Address review comments in one pass, then push once. A bug fix in that pass needs a test that fails without the fix. Style suggestions do not need a commit.

After two completed rounds (a review, your fix, a review, your fix, and another review), stop. Summarize what is still open and wait for a maintainer. If the same point comes back after you fixed it, reply on the thread instead of pushing another patch. The `review-loop` label, when the workflow applies it, means a maintainer should decide.

Do not ask CodeRabbit to push commits to your branch.

## Security

Report a vulnerability privately. See [SECURITY.md](SECURITY.md).
