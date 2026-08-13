# 7DTD Docker-Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 7 Days to Die läuft über Docker (`vinanrra/7dtd-server`) statt SteamCMD/nativem Prozess, als erster Schritt der schrittweisen Ablösung des Hybrid-Systems.

**Architecture:** Das bestehende Docker-Schema (`GameInstall.container_mount: String`, ein einzelner Bind-Mount) wird zu `mounts: Vec<DockerMount>` verallgemeinert, weil `vinanrra/7dtd-server` mehrere getrennte Volumes braucht (Welt, Server-Config, Logs, Backups) statt eines einzigen `/data`-Mounts. Bestehende Docker-Spiele (Minecraft, Factorio) werden im selben Zug auf das neue Feld migriert (kein Parallelbetrieb zweier Formate). Broadcast/Telnet-Unterstützung ist explizit außerhalb des Scopes.

**Tech Stack:** Rust (`serde`, bestehende `games.rs`/`provisioning.rs`-Module), `games.json`-Ressourcendatei, `vinanrra/7dtd-server` Docker-Image.

## Global Constraints

- Kein Parallelbetrieb von `container_mount` (alt) und `mounts` (neu) - alle drei bestehenden Docker-Einträge (Minecraft, Factorio, Factorio-Experimental) werden im selben Commit auf das neue Feld migriert, siehe Spec Abschnitt "Multi-Mount-Unterstützung".
- `START_MODE` wird NICHT in `docker_env` gesetzt (bekannter Endlos-Neustart-Bug bei `START_MODE=0`) - Image-Default verwenden, den tatsächlich richtigen Wert erst nach Log-Analyse beim Live-Test festlegen.
- 7DTDs `install.type: "steamcmd"`-Eintrag wird komplett durch den neuen `"docker"`-Eintrag ersetzt, kein Parallelbetrieb.
- Kein Telnet-/Broadcast-Code in diesem Plan - eigenständiges Folge-Feature (siehe Spec, Abschnitt "Out of Scope").
- Dieses Repo hat keine automatisierten Tests - Verifikation läuft über `cargo check`/`npm run build` für Kompilierbarkeit und abschließendes manuelles Live-Testing gegen einen echten Server.
- Alle User-sichtbaren Strings auf Deutsch, im bestehenden Ton der Codebase.

---

### Task 1: `games.rs` - `DockerMount`-Struct, `GameInstall.mounts` statt `container_mount`

**Files:**
- Modify: `src-tauri/src/games.rs:1-27` (`GameInstall`-Struct, `default_container_mount`-Funktion)

**Interfaces:**
- Produces: `pub struct DockerMount { pub subdir: String, pub container_path: String }` (beide Felder `Deserialize`/`Serialize`, `subdir` mit `#[serde(default)]`). `GameInstall.mounts: Vec<DockerMount>` ersetzt `GameInstall.container_mount: String` vollständig - kein `#[serde(default)]`-Fallback auf das alte Feld nötig, da alle `games.json`-Einträge in Task 3 im selben Commit mitmigriert werden.

- [ ] **Step 1: `DockerMount` einführen, `container_mount` entfernen**

In `src-tauri/src/games.rs`, direkt vor `pub struct GameInstall` (Zeile 6) einfügen:

```rust
/// Ein einzelner Bind-Mount für einen Docker-Container - manche Images (z.B.
/// `vinanrra/7dtd-server`) verlangen mehrere getrennte Mounts statt eines einzigen `/data`-
/// Pfads (Weltdaten, Server-Config, Logs, Backups jeweils eigener Ordner im Container).
#[derive(Serialize, Deserialize, Clone)]
pub struct DockerMount {
    /// Unterordner relativ zum Instanz-Install-Pfad auf dem Host. Leerer String heißt "der
    /// Install-Pfad selbst" - deckt das einfache Ein-Mount-Verhalten der meisten Docker-Spiele
    /// (Minecraft, Factorio) ab, ohne dass die für die dort schon vorhandenen `pre_start_steps`
    /// erzeugten relativen Pfade (z.B. `config/rconpw`) angepasst werden müssten.
    #[serde(default)]
    pub subdir: String,
    pub container_path: String,
}
```

Dann in `GameInstall` (Zeilen 19-22) ersetzen:

```rust
    /// Bind-Mounts für `docker run -v ...` - mindestens ein Eintrag bei `install_type ==
    /// "docker"`. Siehe `DockerMount`-Dokumentation für den Ein-vs-Mehr-Mount-Unterschied.
    #[serde(default)]
    pub mounts: Vec<DockerMount>,
```

Die jetzt ungenutzte Funktion `default_container_mount` (Zeile ~123) komplett entfernen (`grep -n "fn default_container_mount" src-tauri/src/games.rs` zur genauen Zeile).

- [ ] **Step 2: Kompilierbarkeit prüfen**

Run: `cd src-tauri && OPENSSL_DIR="C:/Program Files/OpenSSL-Win64" OPENSSL_LIB_DIR="C:/Program Files/OpenSSL-Win64/lib/VC/x64/MD" OPENSSL_INCLUDE_DIR="C:/Program Files/OpenSSL-Win64/include" cargo check --no-default-features`
Expected: Fehler in `provisioning.rs` (nutzt noch `template.install.container_mount`) und `games.json` (JSON-Deserialisierung schlägt beim App-Start fehl, nicht beim `cargo check` - das JSON-Schema wird erst zur Laufzeit geparst, `cargo check` zeigt hier nur die Rust-seitigen Fehler in `provisioning.rs`). Diese Fehler sind erwartet - Task 2/3 beheben sie.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/games.rs
git commit -m "feat(games): DockerMount-Struct, mounts statt container_mount"
```

---

### Task 2: `provisioning.rs` - Mehrere `-v`-Flags statt einem

**Files:**
- Modify: `src-tauri/src/provisioning.rs:268-304` (`render_docker_systemd_unit`)
- Modify: `src-tauri/src/provisioning.rs:372-383` (Aufrufstelle in `render_docker_install_script`)

**Interfaces:**
- Consumes: `games::DockerMount { subdir: String, container_path: String }` (Task 1).
- Produces: `render_docker_systemd_unit`s Signatur ändert sich von `container_mount: &str` zu `mounts: &[games::DockerMount]`.

- [ ] **Step 1: `render_docker_systemd_unit` auf mehrere Mounts umstellen**

In `src-tauri/src/provisioning.rs`, Signatur (Zeile 268-279) `container_mount: &str` ersetzen durch `mounts: &[games::DockerMount]`. Im Funktionskörper, vor der bestehenden `env_flags`-Berechnung (Zeile 281-284), einfügen:

```rust
    let mount_flags: String = mounts
        .iter()
        .map(|m| {
            if m.subdir.is_empty() {
                format!("-v {install_path}:{} ", m.container_path)
            } else {
                format!("-v {install_path}/{}:{} ", m.subdir, m.container_path)
            }
        })
        .collect();
```

Dann in der `format!`-Zeichenkette (aktuell Zeile 296: `-v {install_path}:{container_mount} -e PUID=...`) `-v {install_path}:{container_mount} ` durch `{mount_flags}` ersetzen, sodass die Zeile lautet:

```
{mount_flags}-e PUID={gameserver_uid} -e PGID={gameserver_gid} \
```

- [ ] **Step 2: Aufrufstelle in `render_docker_install_script` anpassen**

In `src-tauri/src/provisioning.rs`, im Aufruf von `render_docker_systemd_unit` (Zeile 372-383), `&template.install.container_mount` durch `&template.install.mounts` ersetzen.

- [ ] **Step 3: Kompilierbarkeit prüfen**

Run: `cd src-tauri && OPENSSL_DIR="C:/Program Files/OpenSSL-Win64" OPENSSL_LIB_DIR="C:/Program Files/OpenSSL-Win64/lib/VC/x64/MD" OPENSSL_INCLUDE_DIR="C:/Program Files/OpenSSL-Win64/include" cargo check --no-default-features`
Expected: keine Rust-Fehler mehr. `games.json` selbst ist noch nicht angepasst (Task 3) - das schlägt erst zur Laufzeit beim App-Start fehl (JSON-Parsing), nicht bei `cargo check`.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/provisioning.rs
git commit -m "feat: mehrere Docker-Bind-Mounts statt einem einzigen unterstützen"
```

---

### Task 3: `games.json` - Bestehende Docker-Einträge migrieren, 7DTD-Docker-Eintrag ergänzen

**Files:**
- Modify: `src-tauri/resources/games.json` (Minecraft-, Factorio-, Factorio-Experimental- und 7DTD-Einträge)

**Interfaces:**
- Consumes: `mounts`-Feld-Schema aus Task 1 (`[{"subdir": "...", "container_path": "..."}]`).

- [ ] **Step 1: Minecraft-Eintrag migrieren**

In `src-tauri/resources/games.json`, im `minecraft-paper`-Eintrag - `container_mount` ist dort aktuell gar nicht gesetzt (nutzt den alten Default `/data`) - `install`-Objekt um folgendes Feld ergänzen (an beliebiger Stelle im `install`-Objekt, z.B. direkt nach `"image"`):

```json
"mounts": [{ "subdir": "", "container_path": "/data" }],
```

- [ ] **Step 2: Factorio- und Factorio-Experimental-Einträge migrieren**

Beide Einträge haben aktuell `"container_mount": "/factorio"` im `install`-Objekt. Diese Zeile in beiden Einträgen ersetzen durch:

```json
"mounts": [{ "subdir": "", "container_path": "/factorio" }],
```

- [ ] **Step 3: Alten 7DTD-`steamcmd`-Eintrag komplett durch Docker-Eintrag ersetzen**

Den kompletten bestehenden 7DTD-Eintrag (`grep -n '"id": "7dtd"' -A 50 src-tauri/resources/games.json` zur genauen Fundstelle, der Eintrag endet bei der schließenden `},` vor dem nächsten `"id": "dayz"`-Eintrag) durch folgenden Eintrag ersetzen:

```json
{
  "id": "7dtd",
  "name": "7 Days to Die",
  "subtitle": "Dedicated Server",
  "icon": "7daystodie.png",
  "requires": [],
  "tested_on": [],
  "install": {
    "type": "docker",
    "image": "vinanrra/7dtd-server:latest",
    "mounts": [
      { "subdir": "world", "container_path": "/home/sdtdserver/.local/share/7DaysToDie/" },
      { "subdir": "serverfiles", "container_path": "/home/sdtdserver/serverfiles/" },
      { "subdir": "logs", "container_path": "/home/sdtdserver/log/" },
      { "subdir": "backups", "container_path": "/home/sdtdserver/lgsm/backup/" }
    ],
    "docker_env": {},
    "pre_start_steps": []
  },
  "start_command": "",
  "default_cpu_limit_percent": 200,
  "default_ram_limit_mb": 6144,
  "config": {
    "file": "serverfiles/sdtdserver.xml",
    "format": "xml-properties",
    "fields": [
      { "key": "ServerName", "label": "Server Name", "type": "text", "default": "My Game Host" },
      { "key": "ServerPort", "label": "Port", "type": "number", "default": "26900", "opens_port_protocol": "tcp" },
      {
        "key": "GameMode",
        "label": "Spielmodus",
        "type": "select",
        "default": "GameModeSurvival",
        "options": [
          { "value": "GameModeSurvival", "label": "Survival" },
          { "value": "GameModeCreative", "label": "Creative" }
        ]
      },
      { "key": "MaxSpawnedZombies", "label": "Maximale Zombies gleichzeitig", "type": "number", "default": "64" },
      { "key": "LandClaimSize", "label": "Landanspruch-Größe (Blöcke)", "type": "number", "default": "41" },
      { "key": "ServerMaxPlayerCount", "label": "Maximale Spieleranzahl", "type": "number", "default": "8" },
      {
        "key": "SandboxCode",
        "label": "Sandbox-Code (Schwierigkeit, XP, Loot, Blutmond, ...)",
        "type": "text",
        "default": "",
        "hint": "Seit Version 1.0+ steckt die komplette Gameplay-Balance (Schwierigkeit, XP, Beute, Blutmond) in diesem einen Code statt in Einzelwerten. Erzeugen: lokal 7 Days to Die starten, 'Neues Spiel' -> Sandbox-Optionen nach Wunsch einstellen -> generierten Code hier einfügen."
      }
    ]
  },
  "ports": [
    { "port": 26900, "protocol": "tcp" },
    { "port": 26900, "protocol": "udp" }
  ]
}
```

Beachte: `requires: []` statt `requires: ["steamcmd"]` (Docker-Spiele brauchen kein SteamCMD auf dem Host), `tested_on: []` zurückgesetzt (muss neu end-to-end verifiziert werden), `TimeZone` NICHT in `docker_env` gesetzt (im Gegensatz zum Spec-Entwurf, der das als Platzhalter-Beispiel zeigte - `docker_env` bleibt hier bewusst leer, da der Spec-Abschnitt explizit sagt, dass diese Werte erst nach Log-Analyse beim Live-Test final festgelegt werden, nicht blind vorab; Task 4 aktualisiert das bei Bedarf).

- [ ] **Step 4: Kompilierbarkeit prüfen**

Run: `cd src-tauri && OPENSSL_DIR="C:/Program Files/OpenSSL-Win64" OPENSSL_LIB_DIR="C:/Program Files/OpenSSL-Win64/lib/VC/x64/MD" OPENSSL_INCLUDE_DIR="C:/Program Files/OpenSSL-Win64/include" cargo check --no-default-features`
Expected: keine Fehler (JSON wird zur Laufzeit geparst, nicht von `cargo check` validiert - trotzdem ausführen, um sicherzugehen, dass die Rust-Seite aus Task 1/2 vollständig fehlerfrei ist).

- [ ] **Step 5: JSON-Validität manuell prüfen**

Da `games.json` erst zur Laufzeit geparst wird, das JSON separat auf syntaktische Gültigkeit prüfen:

Run: `cd src-tauri && node -e "JSON.parse(require('fs').readFileSync('resources/games.json', 'utf8')); console.log('valid')"`
Expected: `valid`

- [ ] **Step 6: Commit**

```bash
git add src-tauri/resources/games.json
git commit -m "feat: 7DTD auf Docker umgestellt, bestehende Docker-Spiele auf mounts-Feld migriert"
```

---

### Task 4: Live-Verifikation (WSL + Hetzner)

**Files:** keine Code-Änderungen, reines Testing (dieses Projekt hat keine automatisierten Tests, siehe Global Constraints).

- [ ] **Step 1: App neu bauen und starten**

```bash
cd src-tauri && cargo clean -p grimmnetz
```

Dann `npm run tauri dev` starten (mit den üblichen `OPENSSL_*`-Env-Vars).

- [ ] **Step 2: Frisches 7DTD-Docker-Install auf WSL testen**

7 Days to Die über die App installieren, `journalctl -u grimmnetz-<instance-id>` beobachten. Erwartete mögliche Probleme (analog zur Minecraft/Factorio-Migration, iterativ anhand echter Logs fixen, nicht spekulativ):
- Falscher `START_MODE`-Default führt zu Neustart-Schleife → in `docker_env` explizit auf den für Docker/systemd korrekten Wert setzen (aus den Logs/Container-Doku ableiten).
- `sdtdserver.xml` hat andere Struktur/Feldnamen als erwartet → `config.fields`-Schema in `games.json` an die tatsächlich generierte Datei anpassen (`cat` der Datei nach erstem Start prüfen).
- Mount-Ownership-Probleme (Dateien nicht durch `gameserver`-User beschreibbar) → analog zum bereits bekannten PUID/PGID-Verhalten aus der Minecraft-Migration lösen.

- [ ] **Step 3: Konfiguration testen**

Über den Config-Editor `ServerName`, `ServerMaxPlayerCount` ändern, speichern, prüfen dass die Werte nach einem Neustart der Instanz tatsächlich greifen (z.B. im Server-Browser sichtbar bzw. via `cat serverfiles/sdtdserver.xml` auf dem Server).

- [ ] **Step 4: Live gegen Hetzner (Ubuntu 26.04) wiederholen**

Gleicher Ablauf wie Schritt 2/3, diesmal auf dem echten Cloud-Server - deckt distro-spezifische Unterschiede ab (gleiches Muster wie bei der Docker-Pilot- und SSH-Key-Migration).

- [ ] **Step 5: `tested_on` aktualisieren, README-Tabelle, Patch-Notes, Version bumpen**

Analog zum Vorgehen bei den vorherigen Docker-Releases: `tested_on: ["Ubuntu 24.04", "Ubuntu 26.04"]` in `games.json` für den 7DTD-Eintrag setzen, README-Tabelle (`🧟 **7 Days to Die**`-Zeile) auf `*(Docker)*` ergänzen, `src/patchNotes.ts` neuen Eintrag, Version in `package.json`/`src-tauri/tauri.conf.json`/`src-tauri/Cargo.toml` bumpen, `cargo check` zur Aktualisierung von `Cargo.lock`, dann committen.
