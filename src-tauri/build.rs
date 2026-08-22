fn main() {
    generate_tray_translations();
    tauri_build::build();
}

fn generate_tray_translations() {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR is set by Cargo");
    let locales_dir = Path::new("../src/i18n/locales");
    println!("cargo:rerun-if-changed=../src/i18n/locales");

    let mut translations: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    for entry in fs::read_dir(locales_dir)
        .expect("locale directory is readable")
        .flatten()
    {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(language) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let json_path = path.join("translation.json");
        println!("cargo:rerun-if-changed={}", json_path.display());
        let content = fs::read_to_string(&json_path).expect("locale file is readable");
        let parsed: serde_json::Value =
            serde_json::from_str(&content).expect("locale file contains valid JSON");
        if let Some(tray) = parsed.get("tray").cloned() {
            translations.insert(language.to_string(), tray);
        }
    }

    let english = translations
        .get("en")
        .and_then(serde_json::Value::as_object)
        .expect("English tray translations define the schema");
    let fields = english
        .keys()
        .map(|key| (camel_to_snake(key), key.clone()))
        .collect::<Vec<_>>();

    let mut output = String::from("// Auto-generated from locale files. Do not edit.\n\n");
    output.push_str("#[derive(Debug, Clone)]\npub struct TrayStrings {\n");
    for (field, _) in &fields {
        output.push_str(&format!("    pub {field}: String,\n"));
    }
    output.push_str("}\n\n");
    output.push_str(
        "pub static TRANSLATIONS: Lazy<HashMap<&'static str, TrayStrings>> = Lazy::new(|| {\n    let mut map = HashMap::new();\n",
    );
    for (language, tray) in &translations {
        output.push_str(&format!("    map.insert(\"{language}\", TrayStrings {{\n"));
        for (field, json_key) in &fields {
            let value = tray
                .get(json_key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            output.push_str(&format!(
                "        {field}: \"{}\".to_string(),\n",
                escape_string(value)
            ));
        }
        output.push_str("    });\n");
    }
    output.push_str("    map\n});\n");

    fs::write(Path::new(&out_dir).join("tray_translations.rs"), output)
        .expect("generated tray translations are writable");
}

fn camel_to_snake(value: &str) -> String {
    value
        .chars()
        .enumerate()
        .fold(String::new(), |mut output, (index, character)| {
            if character.is_uppercase() && index > 0 {
                output.push('_');
            }
            output.extend(character.to_lowercase());
            output
        })
}

fn escape_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}
