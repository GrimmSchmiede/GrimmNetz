# Docker als Install-/Betriebs-Grundlage (Pilot: Minecraft + Factorio) - Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Minecraft und Factorio laufen komplett über Docker (Installation + Betrieb) statt über SteamCMD/direkte Downloads, als Pilot für den vollständigen Umstieg aller Spiele-Templates.

**Architecture:** Docker-Templates nutzen dieselbe Decoupled-Install-Pipeline (server-seitiges Skript als systemd-oneshot, Tail-Streaming, Crash-Sicherheit) wie die bestehenden SteamCMD-Templates - nur der generierte Skriptinhalt (`docker pull` statt Download) und die Game-systemd-Unit (`docker run --network host` statt rohem Binary) ändern sich. Host-Networking + Bind-Mount halten Firewall/SFTP/Config-Editor unverändert funktionsfähig.

**Tech Stack:** Rust (Tauri-Backend), Docker Engine (`get-docker.sh`), `itzg/minecraft-server`, `factoriotools/factorio`.

## Global Constraints

- Ersetzt SteamCMD/direct-download für Minecraft und Factorio vollständig - kein Parallelbetrieb, kein Backward-Compat-Shim (Projektregel: noch keine echten Nutzer/Installationen).
- `cargo check` / `npx tsc --noEmit` müssen nach jeder Aufgabe sauber sein.
- Keine automatisierten Tests im Projekt - Verifikation über `cargo check`/`tsc` + Live-Tests gegen den WSL-Server (etabliertes Muster).
- Deutsche Fehlermeldungen/UI-Texte.
- Host-Networking (`--network host`), Bind-Mount (`-v <instanz>:/data`), `PUID`/`PGID` auf den bestehenden `gameserver`-User gesetzt - keine Abweichung von diesen drei Design-Entscheidungen ohne Rücksprache.

---

### Task 1: `GameInstall`-Schema um Docker-Felder erweitern

**Files:**
- Modify: `src-tauri/src/games.rs:6-12` (struct `GameInstall`)

**Interfaces:**
- Consumes: nichts Neues
- Produces: `GameInstall.image: Option<String>`, `GameInstall.docker_env: std::collections::BTreeMap<String, String>`, `GameInstall.pre_start_steps: Vec<String>` - von Task 2/3 (Skript-Generator) und Task 6 (games.json) verwendet.

- [ ] **Step 1: Struct erweitern**

In `src-tauri/src/games.rs` die bestehende `GameInstall`-Struct ersetzen:

```rust
#[derive(Serialize, Deserialize, Clone)]
pub struct GameInstall {
    #[serde(rename = "type")]
    pub install_type: String,
    #[serde(default)]
    pub app_id: Option<u32>,
    #[serde(default)]
    pub steps: Vec<String>,
    /// Docker-Image mit Tag (z. B. "itzg/minecraft-server:latest") - nur bei `install_type == "docker"`.
    #[serde(default)]
    pub image: Option<String>,
    /// Umgebungsvariablen für `docker run -e KEY=VALUE ...`.
    #[serde(default)]
    pub docker_env: std::collections::BTreeMap<String, String>,
    /// Shell-Schritte, die vor dem ersten Containerstart im Bind-Mount laufen (z. B. Factorios
    /// server-settings.json) - laufen wie die klassischen `steps` als `gameserver`-User.
    #[serde(default)]
    pub pre_start_steps: Vec<String>,
}
```

Hinweis: `steps` hatte vorher kein `#[serde(default)]` - das ist notwendig, da Docker-Templates dieses Feld nicht mehr setzen (JSON ohne das Feld würde sonst beim Deserialisieren fehlschlagen).

- [ ] **Step 2: `cargo check` ausführen**

```bash
cd "D:/Eigene_Projekte/GrimmNetz/src-tauri" && OPENSSL_DIR="C:/Program Files/OpenSSL-Win64" OPENSSL_LIB_DIR="C:/Program Files/OpenSSL-Win64/lib/VC/x64/MD" OPENSSL_INCLUDE_DIR="C:/Program Files/OpenSSL-Win64/include" cargo check
```

Erwartet: kompiliert sauber (bestehende games.json-Einträge ohne die neuen Felder funktionieren weiter dank `#[serde(default)]`).

- [ ] **Step 3: Commit**

```bash
cd "D:/Eigene_Projekte/GrimmNetz" && git add src-tauri/src/games.rs && git commit -m "Add docker fields to GameInstall schema"
```

---

### Task 2: Docker-Systemd-Unit-Generator

**Files:**
- Modify: `src-tauri/src/provisioning.rs` (neue Funktion nach `render_systemd_unit`, ca. Zeile 231)

**Interfaces:**
- Consumes: nichts Neues (reiner String-Builder)
- Produces: `pub fn render_docker_systemd_unit(instance_id: &str, install_path: &str, unit_name: &str, image: &str, docker_env: &std::collections::BTreeMap<String, String>, ram_limit_mb: u32, cpu_limit_percent: u32, gameserver_uid: u32, gameserver_gid: u32) -> String` - von Task 3 verwendet.

- [ ] **Step 1: Funktion schreiben**

Direkt nach der bestehenden `render_systemd_unit`-Funktion in `src-tauri/src/provisioning.rs` einfügen:

```rust
/// Docker-Variante von `render_systemd_unit` - Container statt rohem Binary/Java-Prozess.
/// Host-Networking (keine Port-Mapping-Konfiguration nötig, Firewall-Logik bleibt identisch),
/// Bind-Mount nach `/data` (Config-Editor/SFTP funktionieren unveraendert, da Dateien direkt im
/// bestehenden Instanzordner liegen), Ressourcenlimits ueber `docker run --memory/--cpus` statt
/// systemd-Cgroups, da Docker-Container standardmaessig NICHT unter dem Cgroup der systemd-Unit
/// laufen - `MemoryMax=`/`CPUQuota=` in der Unit selbst wuerden hier ins Leere greifen.
pub fn render_docker_systemd_unit(
    instance_id: &str,
    install_path: &str,
    unit_name: &str,
    image: &str,
    docker_env: &std::collections::BTreeMap<String, String>,
    ram_limit_mb: u32,
    cpu_limit_percent: u32,
    gameserver_uid: u32,
    gameserver_gid: u32,
) -> String {
    let cpus = cpu_limit_percent as f64 / 100.0;
    let env_flags: String = docker_env
        .iter()
        .map(|(k, v)| format!("-e {}={} ", k, games::shell_single_quote(v)))
        .collect();
    format!(
        "[Unit]\n\
         Description=GrimmNetz Gameserver Instance {instance_id}\n\
         After=network.target docker.service\n\
         Requires=docker.service\n\n\
         [Service]\n\
         Type=simple\n\
         WorkingDirectory={install_path}\n\
         ExecStartPre=-/usr/bin/docker rm -f {unit_name}\n\
         ExecStart=/usr/bin/docker run --rm --name {unit_name} \
--network host --memory={ram_limit_mb}m --cpus={cpus} \
-v {install_path}:/data -e PUID={gameserver_uid} -e PGID={gameserver_gid} \
{env_flags}{image}\n\
         ExecStop=/usr/bin/docker stop -t 30 {unit_name}\n\
         Restart=on-failure\n\
         RestartSec=5\n\n\
         [Install]\n\
         WantedBy=multi-user.target\n"
    )
}
```

Wichtig: `ExecStart` muss als eine einzige Zeile im generierten String stehen (kein `\` Zeilenumbruch-Fortsetzung im finalen Unit-File nötig, `format!` erzeugt das schon als durchgehenden String - die Aufteilung im Rust-Quellcode oben ist nur Lesbarkeit, `\` am Zeilenende in einem Rust-String-Literal wird beim Kompilieren entfernt).

- [ ] **Step 2: `cargo check` ausführen** (gleicher Befehl wie Task 1 Step 2)

Erwartet: kompiliert sauber (Funktion zu diesem Zeitpunkt noch unbenutzt, höchstens eine `dead_code`-Warnung, kein Fehler).

- [ ] **Step 3: Commit**

```bash
cd "D:/Eigene_Projekte/GrimmNetz" && git add src-tauri/src/provisioning.rs && git commit -m "Add render_docker_systemd_unit"
```

---

### Task 3: Docker-Install-Skript-Generator

**Files:**
- Modify: `src-tauri/src/provisioning.rs` (neue Funktion nach `render_docker_systemd_unit` aus Task 2)

**Interfaces:**
- Consumes: `render_docker_systemd_unit` (Task 2), `games::render_step`, `games::shell_single_quote` (beide bereits vorhanden)
- Produces: `pub fn render_docker_install_script(instance_id: &str, install_path: &str, unit_name: &str, template: &games::GameTemplate, gameserver_uid: u32, gameserver_gid: u32) -> String` - von Task 4 (`start_install`) verwendet.

- [ ] **Step 1: Funktion schreiben**

Direkt nach `render_docker_systemd_unit` einfügen:

```rust
/// Docker-Variante von `render_install_script` (Task 1 des vorherigen Branches) - `docker pull`
/// statt SteamCMD/curl-Download, `pre_start_steps` statt der vollen Install-Step-Liste (z.B.
/// Factorios server-settings.json), sonst identisches Muster: GRIMMNETZ_STEP/DONE/FAILED-Marker,
/// fail() räumt bei Fehler auf, Verzeichnis bleibt bis zum Selbst-chown root-only (siehe
/// render_install_script für die Begründung - gilt hier identisch).
pub fn render_docker_install_script(
    instance_id: &str,
    install_path: &str,
    unit_name: &str,
    template: &games::GameTemplate,
    gameserver_uid: u32,
    gameserver_gid: u32,
) -> String {
    let mut script = String::new();
    script.push_str("#!/bin/bash\n");
    script.push_str(&format!("cd {install_path} || exit 1\n"));
    script.push_str(&format!(
        "fail() {{ echo \"GRIMMNETZ_FAILED:$1\"; sudo rm -rf {install_path}; exit 1; }}\n"
    ));
    script.push_str(&format!("chown gameserver:gameserver {install_path}\n"));

    let image = template.install.image.as_deref().unwrap_or_default();
    let total = 2; // Schritt 1: Image ziehen, Schritt 2: Dienst einrichten - pre_start_steps zaehlen nicht separat mit

    script.push_str(&format!("echo \"GRIMMNETZ_STEP 1/{total}\"\n"));
    script.push_str(&format!(
        "docker pull {} || fail \"Image-Download fehlgeschlagen\"\n",
        games::shell_single_quote(image)
    ));

    for step in &template.install.pre_start_steps {
        let rendered = games::render_step(step, instance_id, template.default_ram_limit_mb);
        let quoted = games::shell_single_quote(&rendered);
        script.push_str(&format!(
            "sudo -u gameserver bash -c {quoted} || fail \"Konfiguration konnte nicht vorbereitet werden\"\n"
        ));
    }

    let unit_contents = render_docker_systemd_unit(
        instance_id,
        install_path,
        unit_name,
        image,
        &template.install.docker_env,
        template.default_ram_limit_mb,
        template.default_cpu_limit_percent,
        gameserver_uid,
        gameserver_gid,
    );
    let escaped_unit = unit_contents.replace('\'', "'\\''");
    script.push_str(&format!("echo \"GRIMMNETZ_STEP 2/{total}\"\n"));
    script.push_str(&format!(
        "echo '{escaped_unit}' | sudo tee /etc/systemd/system/{unit_name}.service > /dev/null || fail \"Systemd-Unit konnte nicht geschrieben werden\"\n"
    ));
    script.push_str("sudo systemctl daemon-reload || fail \"daemon-reload fehlgeschlagen\"\n");
    script.push_str(&format!(
        "sudo systemctl enable --now {unit_name} || fail \"Dienst konnte nicht gestartet werden\"\n"
    ));

    for p in &template.ports {
        script.push_str(&format!(
            "(sudo ufw status 2>/dev/null | grep -q 'Status: active' && sudo ufw allow {}/{}) || \
             (systemctl is-active --quiet firewalld 2>/dev/null && sudo firewall-cmd --permanent --add-port={}/{} && sudo firewall-cmd --reload) || true\n",
            p.port, p.protocol, p.port, p.protocol
        ));
    }

    script.push_str("echo \"GRIMMNETZ_DONE\"\n");
    script
}
```

- [ ] **Step 2: `cargo check` ausführen**

Erwartet: kompiliert sauber.

- [ ] **Step 3: Commit**

```bash
cd "D:/Eigene_Projekte/GrimmNetz" && git add src-tauri/src/provisioning.rs && git commit -m "Add render_docker_install_script"
```

---

### Task 4: `start_install` auf Docker-Zweig erweitern

**Files:**
- Modify: `src-tauri/src/lib.rs:487` (Aufruf von `render_install_script` innerhalb von `start_install`)

**Interfaces:**
- Consumes: `provisioning::render_docker_install_script` (Task 3), `template.install.install_type` (Task 1)
- Produces: keine neue öffentliche Schnittstelle - `start_install`s Rückgabewert (`Result<String, String>`, die `instance_id`) bleibt unverändert, nur der interne Skript-Inhalt unterscheidet sich je nach `install_type`.

- [ ] **Step 1: gameserver UID/GID ermitteln und Skript-Aufruf verzweigen**

In `src-tauri/src/lib.rs`, Funktion `start_install`, die Zeile

```rust
    let script = provisioning::render_install_script(&instance_id, &install_path, &unit_name, &template);
```

ersetzen durch:

```rust
    let script = if template.install.install_type == "docker" {
        let uid_out = session.exec("id -u gameserver").await.map_err(|e| e.to_string())?;
        let gid_out = session.exec("id -g gameserver").await.map_err(|e| e.to_string())?;
        let gameserver_uid: u32 = uid_out.trim().parse().map_err(|_| "Konnte gameserver-UID nicht ermitteln".to_string())?;
        let gameserver_gid: u32 = gid_out.trim().parse().map_err(|_| "Konnte gameserver-GID nicht ermitteln".to_string())?;
        provisioning::render_docker_install_script(&instance_id, &install_path, &unit_name, &template, gameserver_uid, gameserver_gid)
    } else {
        provisioning::render_install_script(&instance_id, &install_path, &unit_name, &template)
    };
```

Alles danach (Skript-Schreiben, Install-Unit-Erzeugung, `systemctl start --no-block`) bleibt exakt unverändert - der Docker-Zweig läuft durch dieselbe Install-Unit-Wrapper-Logik wie der bestehende Pfad.

- [ ] **Step 2: `cargo check` ausführen**

Erwartet: kompiliert sauber.

- [ ] **Step 3: Commit**

```bash
cd "D:/Eigene_Projekte/GrimmNetz" && git add src-tauri/src/lib.rs && git commit -m "start_install: branch to Docker install script for docker-type templates"
```

---

### Task 5: Docker-Bootstrap in `bootstrap_server`

**Files:**
- Modify: `src-tauri/src/provisioning.rs:70-129` (Funktion `bootstrap_server`)

**Interfaces:**
- Consumes: nichts Neues
- Produces: nichts Neues (interner Bootstrap-Schritt) - stellt sicher, dass `docker`/`docker.service` auf jedem neu hinzugefügten Server vorhanden sind, bevor Task 4's Docker-Install-Pfad je aufgerufen wird.

- [ ] **Step 1: Docker-Installations-Schritt einfügen**

In `src-tauri/src/provisioning.rs`, Funktion `bootstrap_server`, direkt nach dem SteamCMD-Installations-Block (nach der Zeile mit `.await?;` die auf den `steamcmd_linux.tar.gz`-Download folgt, vor `ensure_swap(ssh).await?;`) einfügen:

```rust
    // Docker via Valves... nein, via Dockers eigenes Installationsskript - deckt alle
    // unterstuetzten Distros ab (erkennt Debian/Fedora/etc. selbst), spart uns die manuelle
    // Paketverwaltungs-Fallunterscheidung wie bei den apt/dnf-Bloecken oben.
    ssh.exec(
        "command -v docker >/dev/null || \
         (curl -fsSL https://get.docker.com -o /tmp/get-docker.sh && sudo sh /tmp/get-docker.sh && rm -f /tmp/get-docker.sh)",
    )
    .await?;
    ssh.exec("sudo systemctl enable --now docker").await?;
```

- [ ] **Step 2: `cargo check` ausführen**

Erwartet: kompiliert sauber.

- [ ] **Step 3: Commit**

```bash
cd "D:/Eigene_Projekte/GrimmNetz" && git add src-tauri/src/provisioning.rs && git commit -m "bootstrap_server: install Docker if missing"
```

---

### Task 6: Minecraft- und Factorio-Templates auf Docker umstellen

**Files:**
- Modify: `src-tauri/resources/games.json` (Einträge `minecraft-paper` und `factorio`)

**Interfaces:**
- Consumes: `install.type: "docker"`, `install.image`, `install.docker_env`, `install.pre_start_steps` (Task 1)
- Produces: nichts Neues - reine Konfigurationsänderung, kein Code.

- [ ] **Step 1: Vor dem Schreiben - Image-Dokumentation verifizieren**

**Wichtig, nicht überspringen:** Bevor die folgenden JSON-Änderungen übernommen werden, die tatsächliche Doku der beiden Images prüfen (Docker Hub / GitHub README):
- `itzg/minecraft-server`: welche Env-Var-Namen für EULA/Server-Typ/RAM-Limit/Port sind aktuell gültig, welcher Pfad im Container entspricht unserem `/data`-Mount für `server.properties` (sollte direkt `/data/server.properties` sein, aber verifizieren).
- `factoriotools/factorio`: welcher Mount-Pfad wird erwartet (könnte `/factorio` statt `/data` sein - falls so, den Bind-Mount-Zielpfad in `render_docker_systemd_unit`-Aufruf für dieses Template entsprechend anpassen oder die Config-Pfade in diesem Template-Eintrag anpassen), welche Env-Vars für RCON-Port/Passwort existieren.

Die untenstehenden JSON-Blöcke sind der Ausgangspunkt basierend auf aktuellem Wissensstand über diese Images - falls die tatsächliche Doku abweicht (Env-Var-Namen, Mount-Pfad), an die echten Werte anpassen, bevor Task 8 (Live-Test) startet. Dieser Verifikationsschritt ist kein optionales Detail - ein falscher Env-Var-Name führt zu einem Container, der zwar startet, aber falsch/gar nicht funktioniert, ohne dass unser Fehlerpfad das erkennt.

- [ ] **Step 2: `minecraft-paper`-Eintrag ersetzen**

In `src-tauri/resources/games.json` den bestehenden `minecraft-paper`-Eintrag (Zeilen 4-61 laut aktuellem Stand) durch folgenden ersetzen, alle anderen Felder (`config`, `ports`) unverändert übernehmen:

```json
{
  "id": "minecraft-paper",
  "name": "Minecraft",
  "subtitle": "Paper (aktuellste Version)",
  "icon": "minecraft.png",
  "requires": [],
  "tested_on": [],
  "install": {
    "type": "docker",
    "image": "itzg/minecraft-server:latest",
    "docker_env": {
      "EULA": "TRUE",
      "TYPE": "PAPER",
      "MEMORY": "{ram_limit_mb}M"
    },
    "pre_start_steps": []
  },
  "start_command": "",
  "default_cpu_limit_percent": 200,
  "default_ram_limit_mb": 4096,
  "config": {
    "file": "server.properties",
    "format": "properties",
    "fields": [
      { "key": "motd", "label": "Server Name (MOTD)", "type": "text", "default": "A Minecraft Server" },
      {
        "key": "gamemode",
        "label": "Spielmodus",
        "type": "select",
        "default": "survival",
        "options": [
          { "value": "survival", "label": "Überleben" },
          { "value": "creative", "label": "Kreativ" },
          { "value": "adventure", "label": "Abenteuer" },
          { "value": "spectator", "label": "Zuschauer" }
        ]
      },
      {
        "key": "difficulty",
        "label": "Schwierigkeitsgrad",
        "type": "select",
        "default": "easy",
        "options": [
          { "value": "peaceful", "label": "Friedlich" },
          { "value": "easy", "label": "Leicht" },
          { "value": "normal", "label": "Normal" },
          { "value": "hard", "label": "Hart" }
        ]
      },
      { "key": "max-players", "label": "Maximale Spieleranzahl", "type": "number", "default": "20" },
      { "key": "server-port", "label": "Port", "type": "number", "default": "25565", "opens_port_protocol": "tcp" },
      { "key": "white-list", "label": "White-List aktivieren", "type": "bool", "default": "false" },
      { "key": "pvp", "label": "PvP aktivieren", "type": "bool", "default": "true" },
      { "key": "view-distance", "label": "Sichtweite (Chunks)", "type": "number", "default": "10" },
      { "key": "online-mode", "label": "Online-Mode (Authentifizierung)", "type": "bool", "default": "true" },
      { "key": "hardcore", "label": "Hardcore-Modus", "type": "bool", "default": "false" },
      { "key": "spawn-protection", "label": "Spawn-Schutz (Radius in Blöcken)", "type": "number", "default": "16" },
      { "key": "allow-flight", "label": "Fliegen erlauben", "type": "bool", "default": "false" }
    ]
  },
  "ports": [{ "port": 25565, "protocol": "tcp" }]
}
```

Hinweise zur Änderung:
- `requires: ["java21"]` entfernt - Java läuft jetzt im Container, nicht mehr auf dem Host.
- `tested_on` auf `[]` zurückgesetzt - die alte Verifikation galt für den SteamCMD/direct-download-Pfad, nicht für Docker. Erst nach Task 8 (Live-Test) wieder auf `["Ubuntu 24.04"]` setzen.
- `start_command: ""` - wird für `install.type == "docker"` nirgends mehr verwendet (siehe Task 3/4), bleibt als leerer String stehen, da `GameTemplate.start_command` kein `Option` ist und das Feld im JSON vorhanden sein muss.
- `MEMORY`-Wert `"{ram_limit_mb}M"` - dieser Platzhalter wird NICHT automatisch ersetzt (nur `render_step` für `steps`/`pre_start_steps`/`start_command` macht das, nicht für `docker_env`-Werte). Falls das Memory-Limit dynamisch aus `default_ram_limit_mb` befüllt werden soll, das in Task 4 als zusätzliche Ersetzung ergänzen (`games::render_step` auf jeden `docker_env`-Wert anwenden, bevor `render_docker_systemd_unit` aufgerufen wird) - andernfalls einen festen Wert eintragen, der zum `default_ram_limit_mb` passt (z. B. `"4096M"`). Diese Entscheidung beim Implementieren treffen und im Code-Kommentar festhalten.

- [ ] **Step 3: `factorio`-Eintrag ersetzen**

Den bestehenden `factorio`-Eintrag ersetzen (Konfigurationsfelder unverändert übernehmen):

```json
{
  "id": "factorio",
  "name": "Factorio",
  "subtitle": "Headless Server",
  "icon": "factorio.png",
  "console_say_format": "{message}",
  "requires": [],
  "tested_on": [],
  "install": {
    "type": "docker",
    "image": "factoriotools/factorio:latest",
    "docker_env": {},
    "pre_start_steps": [
      "echo '{\"name\":\"GrimmNetz Factorio\",\"description\":\"\",\"tags\":[],\"max_players\":0,\"visibility\":{\"public\":false,\"lan\":true},\"username\":\"\",\"password\":\"\",\"token\":\"\",\"game_password\":\"\",\"require_user_verification\":true,\"max_upload_in_kilobytes_per_second\":0,\"max_upload_slots\":5,\"minimum_latency_in_ticks\":0,\"ignore_player_limit_for_returning_players\":false,\"allow_commands\":\"admins-only\",\"autosave_interval\":10,\"autosave_slots\":5,\"afk_autokick_interval\":0,\"auto_pause\":true,\"only_admins_can_pause_the_game\":true,\"autosave_only_on_server\":true}' > /home/gameserver/instances/{instance_id}/server-settings.json",
      "openssl rand -hex 16 > /home/gameserver/instances/{instance_id}/rcon-password.txt && chmod 600 /home/gameserver/instances/{instance_id}/rcon-password.txt"
    ]
  },
  "start_command": "",
  "rcon": { "port": 27015, "password_file": "rcon-password.txt", "broadcast_command": "{message}", "save_command": "server-save" },
  "config": {
    "file": "server-settings.json",
    "format": "json",
    "fields": [
      { "key": "name", "label": "Server Name", "type": "text", "default": "GrimmNetz Factorio" },
      { "key": "max_players", "label": "Spieler Slots (0 = unbegrenzt)", "type": "number", "default": "0" },
      { "key": "game_password", "label": "Passwort", "type": "password", "default": "" }
    ]
  },
  "default_cpu_limit_percent": 150,
  "default_ram_limit_mb": 2048,
  "ports": [{ "port": 34197, "protocol": "udp" }]
}
```

Hinweis: `docker_env` ist hier leer - RCON-Port/Passwort müssen laut Image-Doku entweder als Env-Vars ODER (wie hier angenommen) über die `server-settings.json`/eine `rcon-password.txt`-Datei im Mount konfiguriert werden. **Das ist genau der Punkt aus Step 1, der gegen die echte Image-Doku verifiziert werden muss** - falls `factoriotools/factorio` RCON stattdessen über Env-Vars erwartet (z.B. `RCON_PASSWORD`), müssen diese in `docker_env` statt in `pre_start_steps` landen, und `render_docker_systemd_unit`s generierter Unit ggf. um `--rcon-port`/`--rcon-bind`-Startparameter ergänzt werden (falls das Image das nicht selbst aus Env-Vars ableitet).

- [ ] **Step 4: JSON validieren**

```bash
cd "D:/Eigene_Projekte/GrimmNetz" && node -e "JSON.parse(require('fs').readFileSync('src-tauri/resources/games.json','utf8')); console.log('valid json')"
```

Erwartet: `valid json`.

- [ ] **Step 5: `cargo check` ausführen** (games.json wird per `include_str!` eingebettet, Deserialisierungsfehler zeigen sich erst zur Laufzeit, aber `cargo check` stellt sicher, dass sich am Rust-Code selbst nichts gebrochen hat)

Erwartet: kompiliert sauber.

- [ ] **Step 6: Commit**

```bash
cd "D:/Eigene_Projekte/GrimmNetz" && git add src-tauri/resources/games.json && git commit -m "Convert Minecraft and Factorio templates to Docker"
```

---

### Task 7: `docker_env`-Werte durch `render_step` laufen lassen (Platzhalter-Ersetzung)

**Files:**
- Modify: `src-tauri/src/provisioning.rs` (`render_docker_systemd_unit`, Task 2) ODER `src-tauri/src/lib.rs`/`render_docker_install_script` (Task 3) - je nachdem, wo die Ersetzung sauberer reinpasst (siehe Step 1)

**Interfaces:**
- Consumes: `games::render_step` (bereits vorhanden)
- Produces: `docker_env`-Werte mit `{ram_limit_mb}`/`{instance_id}`/`{xms_mb}`-Platzhaltern werden vor dem Einbau in die systemd-Unit ersetzt (löst die in Task 6 offen gelassene Frage zum `MEMORY`-Platzhalter).

- [ ] **Step 1: Ersetzung einbauen**

In `render_docker_install_script` (`src-tauri/src/provisioning.rs`, Task 3), vor dem Aufruf von `render_docker_systemd_unit`, die `docker_env`-Map durch eine neue Map mit ersetzten Werten ersetzen:

```rust
    let rendered_env: std::collections::BTreeMap<String, String> = template
        .install
        .docker_env
        .iter()
        .map(|(k, v)| (k.clone(), games::render_step(v, instance_id, template.default_ram_limit_mb)))
        .collect();

    let unit_contents = render_docker_systemd_unit(
        instance_id,
        install_path,
        unit_name,
        image,
        &rendered_env,
        template.default_ram_limit_mb,
        template.default_cpu_limit_percent,
        gameserver_uid,
        gameserver_gid,
    );
```

(ersetzt die bisherige `&template.install.docker_env`-Übergabe aus Task 3 Step 1).

- [ ] **Step 2: `cargo check` ausführen**

Erwartet: kompiliert sauber.

- [ ] **Step 3: Commit**

```bash
cd "D:/Eigene_Projekte/GrimmNetz" && git add src-tauri/src/provisioning.rs && git commit -m "Render {ram_limit_mb}/{instance_id} placeholders in docker_env values"
```

---

### Task 8: Live-Verifikation gegen den WSL-Testserver

**Files:** keine Code-Änderungen - reine Verifikation (Projekt-Konvention, siehe Global Constraints).

- [ ] **Step 1: Dev-Build starten**

```bash
taskkill //F //IM grimmnetz.exe 2>/dev/null; taskkill //F //IM cargo.exe 2>/dev/null; taskkill //F //IM node.exe 2>/dev/null
cd "D:/Eigene_Projekte/GrimmNetz" && OPENSSL_DIR="C:/Program Files/OpenSSL-Win64" OPENSSL_LIB_DIR="C:/Program Files/OpenSSL-Win64/lib/VC/x64/MD" OPENSSL_INCLUDE_DIR="C:/Program Files/OpenSSL-Win64/include" npm run tauri dev
```

- [ ] **Step 2: Docker-Bootstrap verifizieren**

Falls der WSL-Testserver noch kein Docker hat: einen neuen Server in der App hinzufügen (oder den bestehenden neu verbinden/`bootstrap_server` erneut anstoßen lassen, falls die App das bei Reconnect wiederholt - sonst direkt in WSL prüfen):

```bash
wsl -e sudo bash -c "command -v docker && docker --version"
```

Erwartet: Docker ist installiert und `docker.service` läuft.

- [ ] **Step 3: Minecraft komplett installieren**

Über den App-Store Minecraft installieren, Fortschritt beobachten (sollte jetzt "Schritt 1/2"/"Schritt 2/2" statt SteamCMD-Prozentanzeige zeigen, da kein `parse_steamcmd_progress`-Match mehr auftritt). Nach Abschluss:

```bash
wsl -e sudo bash -c "docker ps --filter name=grimmnetz- ; systemctl status grimmnetz-<instance_id> --no-pager"
```

Erwartet: Container läuft, systemd-Unit `active (running)`. Im Spiel verbinden (Minecraft-Client, LAN-Adresse des WSL-Servers, Port 25565), Config über den bestehenden Config-Editor ändern (z. B. `motd`), Server über die App neu starten, prüfen dass die Änderung übernommen wurde.

- [ ] **Step 4: Factorio komplett installieren**

Gleicher Ablauf, zusätzlich: RCON-Ansage/Neustart-Countdown-Feature (aus dem vorherigen Branch) testen - prüfen, ob die Nachricht im Spiel ankommt (verifiziert, dass `--network host` RCON auf `127.0.0.1:27015` weiterhin korrekt erreichbar macht).

- [ ] **Step 5: Absturz-/Wiederverbindungstest**

Ein weiteres Spiel installieren, App während `docker pull` (Task 3 Step "GRIMMNETZ_STEP 1/2") hart schließen. Prüfen:

```bash
wsl -e sudo bash -c "systemctl status grimmnetz-install-<instance_id> --no-pager"
```

Erwartet: Install läuft server-seitig weiter. App neu öffnen, prüfen dass sie automatisch erkannt/reattached wird (identischer Flow wie im vorherigen Branch).

- [ ] **Step 6: Deinstallation prüfen**

Eine der Test-Instanzen über die App komplett deinstallieren. Prüfen:

```bash
wsl -e sudo bash -c "docker ps -a --filter name=grimmnetz- ; systemctl list-units --all 'grimmnetz-*' --no-pager ; ls /home/gameserver/instances/"
```

Erwartet: kein Container (auch nicht gestoppt/`docker ps -a`), keine systemd-Unit, kein Instanzordner mehr vorhanden.

- [ ] **Step 7: `tested_on` in games.json aktualisieren**

Nach erfolgreichem Abschluss aller obigen Schritte für beide Spiele: `tested_on` in `src-tauri/resources/games.json` für `minecraft-paper` und `factorio` zurück auf `["Ubuntu 24.04"]` setzen, README-Tabelle entsprechend aktualisieren (`✅`-Status), committen.

```bash
cd "D:/Eigene_Projekte/GrimmNetz" && git add src-tauri/resources/games.json README.md && git commit -m "Mark Minecraft/Factorio Docker migration as tested"
```

- [ ] **Step 8: Ergebnis berichten**

Kurze Zusammenfassung der 6 Testfälle mit Ergebnis - kein weiterer Commit für diesen Task.

---

## Self-Review-Notizen (bereits eingearbeitet)

- **Spec-Abdeckung:** Architektur (Task 2-4), games.json-Schema (Task 1, 6), Systemd-Unit (Task 2), Fehlerbehandlung (Task 3's `fail()`, unverändert aus dem Vorgänger-Branch übernommen), alle 4 Testfälle aus der Spec (Task 8) - vollständig abgedeckt.
- **Placeholder-Scan:** Der einzige bewusst offen gelassene Punkt ist Task 6 Step 1 (Image-Doku verifizieren) - das ist kein Platzhalter im verbotenen Sinn, sondern eine explizite Anweisung, echte externe Fakten (Env-Var-Namen eines Drittanbieter-Images) vor dem Festschreiben zu prüfen, da diese ohne Internetzugriff beim Schreiben dieses Plans nicht verifizierbar waren. Task 6 Step 3 benennt das Risiko konkret (RCON könnte über Env-Vars statt Datei laufen) und sagt genau, was in diesem Fall zu tun ist.
- **Typkonsistenz geprüft:** `render_docker_systemd_unit`s Signatur in Task 2 und ihr Aufruf in Task 3/7 stimmen überein; `render_docker_install_script`s Signatur in Task 3 und ihr Aufruf in Task 4 stimmen überein; `GameInstall.image`/`docker_env`/`pre_start_steps` werden in Task 1 definiert und in Task 3/6/7 exakt so verwendet.
