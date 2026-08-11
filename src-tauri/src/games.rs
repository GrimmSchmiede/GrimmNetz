use serde::{Deserialize, Serialize};

const GAMES_JSON: &str = include_str!("../resources/games.json");

#[derive(Serialize, Deserialize, Clone)]
pub struct GameInstall {
    #[serde(rename = "type")]
    pub install_type: String,
    #[serde(default)]
    pub app_id: Option<u32>,
    pub steps: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ConfigOption {
    pub value: String,
    pub label: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ConfigField {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub field_type: String, // "text" | "number" | "password" | "bool" | "select"
    pub default: String,
    /// Set to "tcp" or "udp" when this field controls the game's listen port - after saving,
    /// the new port gets opened in the server's firewall the same way the install-time ports do.
    #[serde(default)]
    pub opens_port_protocol: Option<String>,
    /// Fixed set of valid values for "select" fields, shown as a dropdown instead of free text -
    /// games like 7 Days to Die reject anything outside a specific set (e.g. GameMode) and typing
    /// a free-text value there silently breaks the server.
    #[serde(default)]
    pub options: Option<Vec<ConfigOption>>,
    /// Short explanatory text shown under the field - for values that need more context than a
    /// label alone can give (e.g. where to get 7 Days to Die's SandboxCode).
    #[serde(default)]
    pub hint: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct PortSpec {
    pub port: u16,
    pub protocol: String, // "tcp" | "udp"
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ConfigSchema {
    /// Path to the config file, relative to the instance's install directory.
    pub file: String,
    /// "properties" (key=value per line) or "json" (flat top-level object).
    pub format: String,
    pub fields: Vec<ConfigField>,
}

/// Some games (Factorio) don't reliably read a plain stdin pipe for console input - they need
/// RCON instead. When present, broadcast messages go over RCON rather than the stdin FIFO.
#[derive(Serialize, Deserialize, Clone)]
pub struct RconSpec {
    pub port: u16,
    /// Path to the file holding the RCON password, relative to the instance's install directory.
    pub password_file: String,
    /// RCON command to broadcast a message, `{message}` is substituted.
    pub broadcast_command: String,
    /// RCON command that forces an immediate save, run right before the restart/stop action so
    /// the countdown doesn't rely on the game's regular autosave interval catching the moment.
    #[serde(default)]
    pub save_command: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct GameTemplate {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub icon: String,
    pub requires: Vec<String>,
    pub install: GameInstall,
    pub start_command: String,
    pub default_cpu_limit_percent: u32,
    pub default_ram_limit_mb: u32,
    #[serde(default)]
    pub config: Option<ConfigSchema>,
    /// Ports that need to be opened in the server's firewall for the game to be reachable.
    #[serde(default)]
    pub ports: Vec<PortSpec>,
    /// Distros this template has actually been installed and run on end-to-end - empty means
    /// "should work" (steamcmd-based, same pattern as tested games) but hasn't been verified.
    /// Shown in the App-Store as a trust signal, never used to block installation.
    #[serde(default)]
    pub tested_on: Vec<String>,
    /// How to write a broadcast message into this game's stdin console. `{message}` is
    /// substituted. Most games understand a `say`-prefixed console command; Factorio's headless
    /// console instead broadcasts any plain-text line as-is, so `say ...` there just prints the
    /// literal word "say" into chat instead of announcing anything.
    #[serde(default = "default_console_say_format")]
    pub console_say_format: String,
    /// When set, broadcast messages use RCON instead of the stdin FIFO (see `RconSpec`).
    #[serde(default)]
    pub rcon: Option<RconSpec>,
}

fn default_console_say_format() -> String {
    "say {message}".to_string()
}

#[derive(Deserialize)]
struct GamesFile {
    games: Vec<GameTemplate>,
}

pub fn load_templates() -> Vec<GameTemplate> {
    let file: GamesFile = serde_json::from_str(GAMES_JSON).expect("bundled games.json must be valid");
    file.games
}

pub fn find_template(game_id: &str) -> Option<GameTemplate> {
    load_templates().into_iter().find(|g| g.id == game_id)
}

/// Substitutes `{instance_id}` / `{ram_limit_mb}` placeholders in install steps and start command.
pub fn render_step(template: &str, instance_id: &str, ram_limit_mb: u32) -> String {
    // Half of the max heap, floored at 2G - gives the JVM a solid starting heap instead of
    // growing it from a small default mid-game, which causes noticeable GC stutter on Linux.
    let xms_mb = (ram_limit_mb / 2).max(2048).min(ram_limit_mb);
    template
        .replace("{instance_id}", instance_id)
        .replace("{ram_limit_mb}", &ram_limit_mb.to_string())
        .replace("{xms_mb}", &xms_mb.to_string())
}

/// Safely wraps a shell command in single quotes for embedding inside another shell command
/// (e.g. `sudo -u gameserver bash -c '<here>'`), escaping any single quotes it contains.
pub fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Extracts the schema's known fields out of a raw config file's contents, falling back to
/// each field's default when the key is absent (e.g. file doesn't exist yet).
/// Splits Palworld's `OptionSettings=(Key=Value,Key2="a,b",...)` body on top-level commas,
/// ignoring commas inside double-quoted string values (e.g. a `ServerName` containing spaces).
fn split_ini_tuple(body: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for c in body.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                current.push(c);
            }
            ',' if !in_quotes => {
                pairs.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        pairs.push(current);
    }
    pairs
        .into_iter()
        .filter_map(|pair| pair.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
        .collect()
}

pub fn parse_config(schema: &ConfigSchema, raw: &str) -> std::collections::HashMap<String, String> {
    let mut found = std::collections::HashMap::new();
    match schema.format.as_str() {
        "json" => {
            if let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(raw) {
                for (k, v) in map {
                    let s = match v {
                        serde_json::Value::String(s) => s,
                        other => other.to_string(),
                    };
                    found.insert(k, s);
                }
            }
        }
        // 7 Days to Die's serverconfig.xml: <property name="X" value="Y"/> per line.
        "xml-properties" => {
            for line in raw.lines() {
                let Some(name_start) = line.find("name=\"") else { continue };
                let after_name = &line[name_start + 6..];
                let Some(name_end) = after_name.find('"') else { continue };
                let name = &after_name[..name_end];

                let Some(value_start) = line.find("value=\"") else { continue };
                let after_value = &line[value_start + 7..];
                let Some(value_end) = after_value.find('"') else { continue };
                let value = &after_value[..value_end];

                found.insert(name.to_string(), value.to_string());
            }
        }
        // Palworld's PalWorldSettings.ini: everything lives in one line,
        // OptionSettings=(Key=Value,Key2=Value2,...).
        "palworld-ini" => {
            if let Some(paren_start) = raw.find("OptionSettings=(") {
                let after = &raw[paren_start + "OptionSettings=(".len()..];
                if let Some(paren_end) = after.rfind(')') {
                    for (k, v) in split_ini_tuple(&after[..paren_end]) {
                        found.insert(k, v.trim_matches('"').to_string());
                    }
                }
            }
        }
        _ => {
            for line in raw.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some((k, v)) = line.split_once('=') {
                    found.insert(k.trim().to_string(), v.trim().to_string());
                }
            }
        }
    }

    schema
        .fields
        .iter()
        .map(|f| {
            let value = found.get(&f.key).cloned().unwrap_or_else(|| f.default.clone());
            (f.key.clone(), value)
        })
        .collect()
}

/// Merges the given values back into the raw config file, preserving unrelated lines/keys
/// and appending any known field that wasn't already present.
pub fn render_config(
    schema: &ConfigSchema,
    raw: &str,
    values: &std::collections::HashMap<String, String>,
) -> String {
    match schema.format.as_str() {
        // Only ever replaces the value="..." of a line whose name="..." matches a known
        // field - every other line (comments, unknown properties) passes through untouched.
        "xml-properties" => raw
            .lines()
            .map(|line| {
                let Some(name_start) = line.find("name=\"") else { return line.to_string() };
                let after_name = &line[name_start + 6..];
                let Some(name_end) = after_name.find('"') else { return line.to_string() };
                let name = &after_name[..name_end];
                let Some(field) = schema.fields.iter().find(|f| f.key == name) else { return line.to_string() };
                let Some(new_value) = values.get(&field.key) else { return line.to_string() };

                let Some(value_start) = line.find("value=\"") else { return line.to_string() };
                let after_value = &line[value_start + 7..];
                let Some(value_end) = after_value.find('"') else { return line.to_string() };
                format!(
                    "{}value=\"{new_value}\"{}",
                    &line[..value_start],
                    &after_value[value_end + 1..]
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        "palworld-ini" => {
            let Some(paren_start) = raw.find("OptionSettings=(") else { return raw.to_string() };
            let tuple_start = paren_start + "OptionSettings=(".len();
            let Some(paren_end) = raw[tuple_start..].rfind(')').map(|i| tuple_start + i) else {
                return raw.to_string();
            };

            let mut pairs = split_ini_tuple(&raw[tuple_start..paren_end]);
            for field in &schema.fields {
                if let Some(new_value) = values.get(&field.key) {
                    // String-typed values (ServerName, ServerPassword, ...) must stay quoted -
                    // Palworld's own parser rejects an unquoted value for those keys.
                    let rendered = if field.field_type == "text" || field.field_type == "password" {
                        format!("\"{new_value}\"")
                    } else {
                        new_value.clone()
                    };
                    if let Some(existing) = pairs.iter_mut().find(|(k, _)| k == &field.key) {
                        existing.1 = rendered;
                    } else {
                        pairs.push((field.key.clone(), rendered));
                    }
                }
            }
            let body = pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(",");
            format!("{}{body}{}", &raw[..tuple_start], &raw[paren_end..])
        }
        "json" => {
            let mut root: serde_json::Value =
                serde_json::from_str(raw).unwrap_or_else(|_| serde_json::json!({}));
            if !root.is_object() {
                root = serde_json::json!({});
            }
            let obj = root.as_object_mut().unwrap();
            for field in &schema.fields {
                if let Some(v) = values.get(&field.key) {
                    let json_value = match field.field_type.as_str() {
                        "number" => v
                            .parse::<f64>()
                            .map(|n| serde_json::json!(n))
                            .unwrap_or_else(|_| serde_json::Value::String(v.clone())),
                        "bool" => serde_json::json!(v == "true"),
                        _ => serde_json::Value::String(v.clone()),
                    };
                    obj.insert(field.key.clone(), json_value);
                }
            }
            serde_json::to_string_pretty(&root).unwrap_or_else(|_| raw.to_string())
        }
        _ => {
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut lines: Vec<String> = raw
                .lines()
                .map(|line| {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with('#') {
                        return line.to_string();
                    }
                    if let Some((k, _)) = trimmed.split_once('=') {
                        let key = k.trim();
                        if let Some(field) = schema.fields.iter().find(|f| f.key == key) {
                            if let Some(v) = values.get(&field.key) {
                                seen.insert(field.key.clone());
                                return format!("{key}={v}");
                            }
                        }
                    }
                    line.to_string()
                })
                .collect();

            for field in &schema.fields {
                if !seen.contains(&field.key) {
                    if let Some(v) = values.get(&field.key) {
                        lines.push(format!("{}={}", field.key, v));
                    }
                }
            }
            lines.join("\n")
        }
    }
}
