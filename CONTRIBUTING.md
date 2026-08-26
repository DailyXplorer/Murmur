# Contributing to Murmur

Keep each change focused. Explain why it is needed and test the behavior it affects.

## Before you start

- Search existing [issues](https://github.com/DailyXplorer/Murmur/issues) and [pull requests](https://github.com/DailyXplorer/Murmur/pulls).
- Use the [bug report template](.github/ISSUE_TEMPLATE/bug_report.md) for reproducible bugs.
- Start a [discussion](https://github.com/DailyXplorer/Murmur/discussions) for feature ideas that would benefit from feedback before implementation.

## Set up the project

Install the latest stable Rust toolchain, [Bun](https://bun.sh/), and the platform-specific dependencies from [BUILD.md](BUILD.md).

Clone your fork after replacing `YOUR_USERNAME` with your GitHub username:

```bash
git clone git@github.com:YOUR_USERNAME/Murmur.git
cd Murmur
bun install
bun run tauri dev
```

On macOS, use this command if CMake rejects an old dependency policy:

```bash
CMAKE_POLICY_VERSION_MINIMUM=3.5 bun run tauri dev
```

## Make a change

- Follow the existing Rust, TypeScript, React, and Tailwind patterns described in [AGENTS.md](AGENTS.md).
- Add user-facing text to the i18next locale files instead of hardcoding it in JSX.
- Keep generated files and lockfiles in sync with their source changes.
- Update documentation when behavior, setup, or public interfaces change.

Use conventional commit prefixes such as `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, and `chore:`.

## Validate the change

Run the checks relevant to the files and behavior you changed. Common frontend checks are:

```bash
bun run lint
bun run format:check
bun run check:translations
bun run build
bun run test:playwright
```

For Rust changes, run the backend formatter, tests, and Clippy from `src-tauri`:

```bash
cargo fmt -- --check
cargo test
cargo clippy
```

Use `bun run tauri build` when a production build or native packaging behavior is affected.

## Open a pull request

The pull request template asks for three things:

- A summary of what changed and why.
- The exact checks run and their results.
- Screenshots or recordings for visible changes.

Keep the description specific enough for a reviewer to understand both the implementation and its limits.

## Translations

See [CONTRIBUTING_TRANSLATIONS.md](CONTRIBUTING_TRANSLATIONS.md) for locale structure and translation checks.

## License

By contributing, you agree that your contribution is licensed under the [MIT License](LICENSE).
