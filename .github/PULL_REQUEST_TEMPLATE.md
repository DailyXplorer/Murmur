## Summary

<!-- Why this change is needed. Link the issue. -->

## Changes

<!-- What changed, and anything deliberately left out. -->

## Testing

<!-- `bun run check`, plus `bun run test:playwright` or `cargo test --locked` when those apply. Commands and results. -->

## Screenshots or recordings

<!-- Visible UI changes need evidence. Otherwise: Not applicable. -->

## Checklist

- [ ] User-facing strings go through i18next
- [ ] A version bump updates `package.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json`, and `src/content/release-notes/<version>.md` together
- [ ] The checks listed under Testing were run
- [ ] Each real bug found in review is fixed with a test that fails first. Style suggestions are optional. If the same comment comes back after a fix, I replied on the thread instead of applying it again
