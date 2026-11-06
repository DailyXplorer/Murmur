# Release Notes

Add user-facing release notes as Markdown files named by app version:

```text
src/content/release-notes/0.8.4.md
```

The update modal shows the highest bundled release note newer than the
persisted `whats_new_last_seen_version` and not newer than the running app
version.

Keep these files focused on headline user-facing changes. Release notes support
paragraphs, headings, lists, links, code, quotes, separators, hard line breaks,
and local images under `/release-notes/...`. Raw HTML is ignored before
rendering.

Place image assets in `public/release-notes/{version}/` and reference them from
Markdown with absolute paths:

```md
![Streaming transcription preview](/release-notes/0.9.0/streaming-transcription.webp)
```

## Windows (beta)

Murmur for Windows ships in the same release as the Mac app, under the same
version number. Every release's notes end with a "Windows (beta)" section in
user terms:

```md
## Windows (beta)

Murmur for Windows is a beta and comes with this release: download
`Murmur_<version>_x64-setup.exe` from the same page as the Mac app. The
installer has no Windows certificate, so SmartScreen asks once. PCs on Beta 7
or 8 update by themselves. Nothing changes for Mac users.
```

A release built with `windows` off (a Mac-only fix) says so in one line. Test
releases (`x.y.z-test.N`) use the notes of `x.y.z`.
