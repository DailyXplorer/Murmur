# Contributing Translations to Murmur

Thank you for helping translate Murmur! This guide explains how to improve the existing translations.

## Supported Languages

The interface ships in exactly these six languages:

| Language   | Code | Status            |
| ---------- | ---- | ----------------- |
| English    | `en` | Complete (source) |
| French     | `fr` | Complete          |
| Spanish    | `es` | Complete          |
| German     | `de` | Complete          |
| Italian    | `it` | Complete          |
| Portuguese | `pt` | Complete          |

We do not accept new interface languages. Contributions that improve these six translations are welcome.

This list covers the interface only. Dictation supports many more languages.

## File Structure

```
src/i18n/locales/
├── en/translation.json    # English (source)
├── fr/translation.json
├── es/translation.json
├── de/translation.json
├── it/translation.json
└── pt/translation.json
```

## Improving a Translation

Found a typo or a better translation?

1. Edit the relevant `translation.json` file. Translate only the **values**, never the keys.
2. Run `bun run check:translations` to confirm every locale still has the same keys as English.
3. Run the app with `bun run tauri dev`, choose your language in Settings → General → Application language, and check the text in place.
4. Submit a pull request with a brief description of the change.

## Translation Guidelines

### Do:

- Use natural, native-sounding language
- Keep translations concise (UI space is limited)
- Match the tone of the English text (friendly, clear)
- Preserve technical terms when appropriate (e.g., "API", "GPU")

### Don't:

- Translate brand names (Murmur, Codex, ChatGPT, OpenAI)
- Change or remove `{{variables}}`
- Modify JSON keys
- Add extra spaces or formatting

### Handling Variables

Some strings contain variables like `{{error}}` or `{{version}}`. Keep these exactly as-is:

```json
// English
"requestFailed": "Transcription failed: {{error}}"

// French (correct)
"requestFailed": "Échec de la transcription : {{error}}"

// French (incorrect - don't translate the variable!)
"requestFailed": "Échec de la transcription : {{erreur}}"
```

## Questions?

- Open an issue on GitHub
- Join the discussion in existing translation PRs
