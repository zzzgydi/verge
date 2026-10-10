use std::{
    collections::BTreeMap,
    env, fmt, fs,
    path::{Path, PathBuf},
};

use serde::de::{Deserialize, Deserializer, Error, MapAccess, Visitor};

struct LocaleStrings(BTreeMap<String, String>);

impl<'de> Deserialize<'de> for LocaleStrings {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UniqueKeys;

        impl<'de> Visitor<'de> for UniqueKeys {
            type Value = LocaleStrings;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a JSON object of unique string translations")
            }

            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut entries = BTreeMap::new();
                while let Some((key, value)) = map.next_entry::<String, String>()? {
                    if entries.insert(key.clone(), value).is_some() {
                        return Err(M::Error::custom(format!("duplicate locale key: {key}")));
                    }
                }
                Ok(LocaleStrings(entries))
            }
        }

        deserializer.deserialize_map(UniqueKeys)
    }
}

fn main() {
    println!("cargo:rerun-if-env-changed=VERGE_BUILD_CHANNEL");
    let channel = std::env::var("VERGE_BUILD_CHANNEL").unwrap_or_else(|_| "stable".into());
    assert!(
        matches!(channel.as_str(), "stable" | "dev"),
        "VERGE_BUILD_CHANNEL must be stable or dev"
    );
    println!("cargo:rustc-env=VERGE_BUILD_CHANNEL={channel}");

    generate_locales(&PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("locales"));
}

fn generate_locales(directory: &Path) {
    println!("cargo:rerun-if-changed={}", directory.display());
    let mut locales = BTreeMap::new();
    for entry in fs::read_dir(directory).expect("read locales directory") {
        let path = entry.expect("read locale file").path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let code = path
            .file_stem()
            .unwrap()
            .to_str()
            .expect("UTF-8 locale name");
        assert!(
            !code.is_empty()
                && code.starts_with(|c: char| c.is_ascii_alphabetic())
                && code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "invalid locale code: {code}"
        );
        println!("cargo:rerun-if-changed={}", path.display());
        let content = fs::read_to_string(&path).expect("read locale JSON");
        let LocaleStrings(entries): LocaleStrings = serde_json::from_str(&content)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        for (key, text) in &entries {
            assert!(!text.is_empty(), "{code}: {key} must not be empty");
        }
        assert!(
            locales.insert(code.to_owned(), entries).is_none(),
            "duplicate locale: {code}"
        );
    }
    let english = locales.get("en").expect("locales/en.json is required");
    assert!(
        english.contains_key("language.name"),
        "English locale needs language.name"
    );
    for (code, entries) in &locales {
        assert!(
            entries.contains_key("language.name"),
            "{code}: missing language.name"
        );
        for (key, value) in entries {
            let base = english.get(key).unwrap_or_else(|| {
                panic!("{code}: unexpected key {key} (add it to en.json first)")
            });
            assert_eq!(
                fields(value),
                fields(base),
                "{code}: {key} must use the same placeholders as en.json"
            );
        }
    }

    let mut source = String::from("const LOCALES: &[Locale] = &[\n");
    for (code, entries) in &locales {
        source.push_str(&format!(
            "    Locale {{ code: {code:?}, name: {:?}, entries: &[\n",
            entries["language.name"]
        ));
        for (key, value) in entries {
            source.push_str(&format!("        ({key:?}, {value:?}),\n"));
        }
        source.push_str("    ] },\n");
    }
    source.push_str("];\n");
    let output = PathBuf::from(env::var("OUT_DIR").unwrap()).join("locales.rs");
    fs::write(output, source).expect("generate embedded locales");
}

fn fields(text: &str) -> Vec<&str> {
    let mut rest = text;
    let mut result = Vec::new();
    while let Some((_, tail)) = rest.split_once('{') {
        let (name, suffix) = tail.split_once('}').expect("unclosed locale placeholder");
        assert!(
            !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "invalid locale placeholder: {name}"
        );
        result.push(name);
        rest = suffix;
    }
    result.sort_unstable();
    result
}
