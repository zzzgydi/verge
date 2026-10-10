# Locales

`en.json` is the base catalog. Each other `<locale-code>.json` contains a flat object of string keys and translated values; missing keys fall back to English. Add a `language.name` entry for the label in Settings (for example, `"language.name": "Español"`). A new JSON file is discovered at build time, so no Rust enum or registration change is needed.

The build script rejects duplicate keys, invalid values and mismatched placeholders, then embeds sorted static tables in the executable. Locale files are not read from disk at runtime. New keys must first be added to `en.json`; other languages can translate them incrementally. For variable text use named placeholders such as `{name}` in each translation and keep the same placeholder names as English. Select a locale in Settings to use it in the window and tray. Unknown saved locale codes remain invalid.
