# Installationen von der App-Laufzeit entkoppeln - Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Game-Server-Installationen laufen als eigenständiger systemd-oneshot-Dienst auf dem Server statt über die offene SSH-Session der App, überstehen App-Crash/Verbindungsabbruch und sind von jedem PC aus, der Zugriff auf denselben Server hat, sichtbar/fortsetzbar.

**Architecture:** Ein generiertes Bash-Skript führt alle Install-Schritte + Erzeugen/Starten der Spiele-systemd-Unit + Firewall-Port-Öffnung serverseitig aus und schreibt strukturierte Marker-Zeilen (`GRIMMNETZ_STEP`, `GRIMMNETZ_DONE`, `GRIMMNETZ_FAILED:<msg>`) in eine Log-Datei. Die App startet den Dienst (`start_install`) und hängt sich per `tail -f` (`attach_install_stream`) daran an, exakt wie das bestehende Live-Terminal für Logs. `list_active_installs` scannt den Server nach laufenden/kürzlich abgeschlossenen Installs, unabhängig von der lokalen DB.

**Tech Stack:** Rust (Tauri-Backend, `russh`), React/TypeScript-Frontend, Bash (generiertes Server-Skript), systemd.

## Global Constraints

- Kein Backward-Compat-Shim für den alten `install_game`-Pfad - vollständiger Cutover, da es noch keine echten Nutzer/Installationen gibt (etablierte Regel aus diesem Projekt).
- Jede neue/geänderte Datei muss `cargo check` (mit `OPENSSL_DIR`/`OPENSSL_LIB_DIR`/`OPENSSL_INCLUDE_DIR`) bzw. `npx tsc --noEmit` sauber durchlaufen, bevor eine Aufgabe als fertig gilt.
- Keine automatisierten Rust/TS-Tests existieren in diesem Projekt (Verifikation erfolgt bisher ausschließlich über `cargo check`/`tsc` + manuelles Live-Testen gegen den WSL-Testserver) - dieser Plan folgt demselben etablierten Muster statt eine neue Test-Infrastruktur einzuführen.
- Deutsche Fehlermeldungen/UI-Texte, passend zum Rest der Codebase.
- Kein Kommentar-Overhead: nur WHY-Kommentare bei nicht offensichtlichen Constraints, wie im Rest der Codebase üblich.

---

### Task 1: Skript-Generator in `provisioning.rs`

**Files:**
- Modify: `src-tauri/src/provisioning.rs` (neue Funktion, ans Ende der Datei anhängen)

**Interfaces:**
- Consumes: `games::GameTemplate` (Felder: `install.steps: Vec<String>`, `default_ram_limit_mb: u32`, `default_cpu_limit_percent: u32`, `start_command: String`, `ports: Vec<PortSpec>` mit `port: u16`, `protocol: String`), `games::render_step(template: &str, instance_id: &str, ram_limit_mb: u32) -> String`, `games::shell_single_quote(s: &str) -> String`, `render_systemd_unit(instance_id, working_dir, start_command, ram_limit_mb, cpu_limit_percent) -> String` (bereits in dieser Datei, Zeile 191)
- Produces: `pub fn render_install_script(instance_id: &str, install_path: &str, unit_name: &str, template: &games::GameTemplate) -> String` - reiner String-Builder, keine SSH-Interaktion, von Task 2 verwendet.

- [ ] **Step 1: Funktion schreiben**

Ans Ende von `src-tauri/src/provisioning.rs` anhängen:

```rust
/// Builds the full install script that runs entirely server-side as a systemd oneshot unit -
/// every install step, the game's own systemd unit (write + enable + start), and best-effort
/// firewall port opening. Writes `GRIMMNETZ_STEP`/`GRIMMNETZ_DONE`/`GRIMMNETZ_FAILED:<msg>`
/// marker lines to stdout (captured into `install.log` via the systemd unit) so the app can
/// tail the file from any machine and reconstruct progress, independent of its own SSH session.
pub fn render_install_script(
    instance_id: &str,
    install_path: &str,
    unit_name: &str,
    template: &games::GameTemplate,
) -> String {
    let total = template.install.steps.len();
    let mut script = String::new();
    script.push_str("#!/bin/bash\n");
    script.push_str(&format!("cd {install_path} || exit 1\n"));
    // fail() centralises the same cleanup-on-failure behaviour install_game already has today -
    // remove the half-finished instance dir so failed attempts never silently eat disk space.
    script.push_str(&format!(
        "fail() {{ echo \"GRIMMNETZ_FAILED:$1\"; sudo rm -rf {install_path}; exit 1; }}\n"
    ));

    for (idx, step) in template.install.steps.iter().enumerate() {
        let rendered = games::render_step(step, instance_id, template.default_ram_limit_mb);
        let quoted = games::shell_single_quote(&rendered);
        script.push_str(&format!("echo \"GRIMMNETZ_STEP {}/{total}\"\n", idx + 1));
        script.push_str(&format!(
            "sudo -u gameserver bash -c {quoted} || fail \"Schritt {} fehlgeschlagen\"\n",
            idx + 1
        ));
    }

    // Game's own systemd unit - written and started here, not by the app after the fact, so
    // the game is actually running even if the app never reattaches to see GRIMMNETZ_DONE.
    let start_command = games::render_step(&template.start_command, instance_id, template.default_ram_limit_mb);
    let unit_contents = render_systemd_unit(
        instance_id,
        install_path,
        &start_command,
        template.default_ram_limit_mb,
        template.default_cpu_limit_percent,
    );
    let escaped_unit = unit_contents.replace('\'', "'\\''");
    script.push_str("echo \"GRIMMNETZ_STEP unit\"\n");
    script.push_str(&format!(
        "echo '{escaped_unit}' | sudo tee /etc/systemd/system/{unit_name}.service > /dev/null || fail \"Systemd-Unit konnte nicht geschrieben werden\"\n"
    ));
    script.push_str("sudo systemctl daemon-reload || fail \"daemon-reload fehlgeschlagen\"\n");
    script.push_str(&format!(
        "sudo systemctl enable --now {unit_name} || fail \"Dienst konnte nicht gestartet werden\"\n"
    ));

    // Best-effort port opening - self-detects whichever firewall is active, mirrors the logic
    // in `open_port()` but expressed as shell so it runs without any Rust round-trip. A single
    // port failing to open must never fail the whole install (server is already up by now).
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

```bash
cd "D:/Eigene_Projekte/GrimmNetz/src-tauri" && OPENSSL_DIR="C:/Program Files/OpenSSL-Win64" OPENSSL_LIB_DIR="C:/Program Files/OpenSSL-Win64/lib/VC/x64/MD" OPENSSL_INCLUDE_DIR="C:/Program Files/OpenSSL-Win64/include" cargo check
```

Erwartet: kompiliert sauber (Funktion ist zu diesem Zeitpunkt noch unbenutzt - `#[allow(dead_code)]` ist NICHT nötig, da Task 2 sie sofort verwendet; falls zwischen den Tasks committed wird, `cargo check` gibt höchstens eine `dead_code`-Warnung aus, kein Fehler).

- [ ] **Step 3: Commit**

```bash
cd "D:/Eigene_Projekte/GrimmNetz" && git add src-tauri/src/provisioning.rs && git commit -m "Add render_install_script for decoupled server-side installs"
```

---

### Task 2: `start_install`-Command in `lib.rs`

**Files:**
- Modify: `src-tauri/src/lib.rs` (neue Funktion direkt vor der bestehenden `install_game`-Funktion, ca. Zeile 434)

**Interfaces:**
- Consumes: `provisioning::render_install_script` (Task 1), `games::find_template(game_id: &str) -> Option<GameTemplate>`, `acquire_session(state, server_id) -> Result<OwnedMutexGuard<Option<SshSession>>, String>` (bereits vorhanden)
- Produces: `#[tauri::command] async fn start_install(state, server_id: String, game_id: String) -> Result<String, String>` - gibt die neu erzeugte `instance_id` zurück. Wird von Task 6 (Frontend) aufgerufen.

- [ ] **Step 1: Funktion schreiben**

Direkt vor `async fn install_game(` in `src-tauri/src/lib.rs` einfügen:

```rust
/// Kicks off a game install as a detached, server-side systemd-oneshot unit and returns
/// immediately - the install itself survives the app closing or the connection dropping.
/// Progress is observed separately via `attach_install_stream`.
#[tauri::command]
async fn start_install(state: State<'_, AppState>, server_id: String, game_id: String) -> Result<String, String> {
    let template = games::find_template(&game_id).ok_or_else(|| format!("Unbekanntes Spiel: {game_id}"))?;

    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();

    let instance_id = uuid::Uuid::new_v4().to_string();
    let install_path = format!("/home/gameserver/instances/{instance_id}");
    let unit_name = format!("grimmnetz-{instance_id}");
    let install_unit_name = format!("grimmnetz-install-{instance_id}");

    // Same ownership fix as the old install_game: /home/gameserver is root-owned, the install
    // script itself runs each step as `gameserver`, so the dir needs to belong to that user first.
    session
        .exec(&format!("sudo mkdir -p {install_path} && sudo chown gameserver:gameserver {install_path}"))
        .await
        .map_err(|e| e.to_string())?;

    let script = provisioning::render_install_script(&instance_id, &install_path, &unit_name, &template);
    let escaped_script = script.replace('\'', "'\\''");
    session
        .exec(&format!(
            "echo '{escaped_script}' | sudo tee {install_path}/install.sh > /dev/null && sudo chmod +x {install_path}/install.sh"
        ))
        .await
        .map_err(|e| e.to_string())?;

    // The install unit's own stdout/stderr (captured by systemd/journald AND redirected into
    // install.log) is what attach_install_stream tails - RemainAfterExit keeps `systemctl
    // is-active` truthful about "still running" for list_active_installs after the script exits.
    let install_unit_contents = format!(
        "[Unit]\nDescription=GrimmNetz Install {instance_id}\n\n\
         [Service]\nType=oneshot\nRemainAfterExit=yes\nUser=gameserver\nWorkingDirectory={install_path}\n\
         ExecStart=/bin/bash -c '/bin/bash {install_path}/install.sh > {install_path}/install.log 2>&1'\n\n\
         [Install]\nWantedBy=multi-user.target\n"
    );
    provisioning::install_systemd_unit(session, &install_unit_name, &install_unit_contents)
        .await
        .map_err(|e| e.to_string())?;
    session
        .exec(&format!("sudo systemctl start {install_unit_name}"))
        .await
        .map_err(|e| e.to_string())?;

    Ok(instance_id)
}
```

- [ ] **Step 2: `cargo check` ausführen** (gleicher Befehl wie Task 1 Step 2)

Erwartet: kompiliert sauber.

- [ ] **Step 3: Command registrieren**

In `src-tauri/src/lib.rs` im `tauri::generate_handler![...]`-Makroaufruf (suche nach `install_game,`) direkt daneben `start_install,` ergänzen.

- [ ] **Step 4: Commit**

```bash
cd "D:/Eigene_Projekte/GrimmNetz" && git add src-tauri/src/lib.rs && git commit -m "Add start_install command - runs installs as detached systemd units"
```

---

### Task 3: `attach_install_stream`-Command in `lib.rs`

**Files:**
- Modify: `src-tauri/src/lib.rs` (neue Funktion direkt nach `start_install`)

**Interfaces:**
- Consumes: `InstallEvent` (bereits definiert, Zeile 107-112: `Step { label: String }`, `Progress { percent: f32, phase: String }`), `parse_steamcmd_progress` (bereits vorhanden, Zeile 118), `acquire_session`, `games::find_template`, `InstanceRecord` (Felder wie in Zeile 543 verwendet: `id, server_id, game_id, display_name, install_path, systemd_unit, cpu_limit_percent, ram_limit_mb`), `db.insert_instance` (bereits verwendet vom alten `install_game`, gleiche Signatur beibehalten)
- Produces: `#[tauri::command] async fn attach_install_stream(state, server_id: String, instance_id: String, game_id: String, display_name: String, on_event: Channel<InstallEvent>) -> Result<InstanceRecord, String>` - wird von Task 6 (Frontend) direkt nach `start_install` und bei jedem Reconnect erneut aufgerufen.

- [ ] **Step 1: Funktion schreiben**

Direkt nach der `start_install`-Funktion aus Task 2 einfügen:

```rust
/// Tails the running (or already-finished) install's log from the start and turns each line
/// back into the same InstallEvent stream the old live-streaming install_game produced -
/// reconnect-safe because re-reading the whole (short) log file from byte 0 is cheap and just
/// re-derives the same UI state, whether this is the first attach or a reconnect after a drop.
#[tauri::command]
async fn attach_install_stream(
    state: State<'_, AppState>,
    server_id: String,
    instance_id: String,
    game_id: String,
    display_name: String,
    on_event: Channel<InstallEvent>,
) -> Result<InstanceRecord, String> {
    let template = games::find_template(&game_id).ok_or_else(|| format!("Unbekanntes Spiel: {game_id}"))?;
    let install_path = format!("/home/gameserver/instances/{instance_id}");
    let unit_name = format!("grimmnetz-{instance_id}");

    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();

    // awk exits (closing its stdin pipe) the moment it sees the DONE/FAILED marker, which sends
    // tail -f a SIGPIPE and ends the whole pipeline - that's what makes exec_stream_lines return
    // instead of blocking forever the way a bare `tail -f` would.
    let tail_cmd = format!(
        "tail -f -n +1 {install_path}/install.log 2>/dev/null | \
         awk '{{ print; fflush(); if ($0 ~ /^GRIMMNETZ_DONE/ || $0 ~ /^GRIMMNETZ_FAILED/) exit }}'"
    );

    let mut failure: Option<String> = None;
    let mut total_steps = template.install.steps.len();
    session
        .exec_stream_lines(&tail_cmd, |line| {
            if let Some(rest) = line.strip_prefix("GRIMMNETZ_STEP ") {
                if rest == "unit" {
                    let _ = on_event.send(InstallEvent::Step { label: "Dienst wird eingerichtet".to_string() });
                } else if let Some((n, total)) = rest.split_once('/') {
                    total_steps = total.parse().unwrap_or(total_steps);
                    let _ = on_event.send(InstallEvent::Step { label: format!("Schritt {n}/{total}") });
                }
            } else if let Some(msg) = line.strip_prefix("GRIMMNETZ_FAILED:") {
                failure = Some(msg.to_string());
            } else if let Some((percent, phase)) = parse_steamcmd_progress(&line) {
                let _ = on_event.send(InstallEvent::Progress { percent, phase });
            }
        })
        .await
        .map_err(|e| e.to_string())?;

    if let Some(msg) = failure {
        return Err(msg);
    }

    let _ = total_steps; // only used to enrich the label above, no further use once done

    let record = InstanceRecord {
        id: instance_id,
        server_id,
        game_id,
        display_name,
        install_path,
        systemd_unit: unit_name,
        cpu_limit_percent: template.default_cpu_limit_percent,
        ram_limit_mb: template.default_ram_limit_mb,
    };
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.insert_instance(&record).map_err(|e| e.to_string())?;
    Ok(record)
}
```

- [ ] **Step 2: `cargo check` ausführen** - falls `InstanceRecord`-Feldnamen oder `db.insert_instance`-Signatur vom hier angenommenen Stand abweichen, an die tatsächliche Signatur in `src-tauri/src/db.rs` anpassen (mit `grep -n "fn insert_instance" src-tauri/src/db.rs` prüfen).

Erwartet: kompiliert sauber.

- [ ] **Step 3: Command registrieren**

In `tauri::generate_handler![...]` neben `start_install,` (aus Task 2) `attach_install_stream,` ergänzen.

- [ ] **Step 4: Commit**

```bash
cd "D:/Eigene_Projekte/GrimmNetz" && git add src-tauri/src/lib.rs && git commit -m "Add attach_install_stream command - tails install.log for progress"
```

---

### Task 4: `list_active_installs`-Command in `lib.rs`

**Files:**
- Modify: `src-tauri/src/lib.rs` (neue Funktion direkt nach `attach_install_stream`)

**Interfaces:**
- Consumes: `acquire_session`
- Produces: `#[tauri::command] async fn list_active_installs(state, server_id: String) -> Result<Vec<ActiveInstall>, String>` mit neuem `pub struct ActiveInstall { instance_id: String, game_id: String, running: bool }` - wird von Task 7 (Frontend, App-Store-Tab-Öffnen) verwendet.

- [ ] **Step 1: `ActiveInstall`-Struct + Funktion schreiben**

Direkt vor `attach_install_stream` (oder danach, Reihenfolge egal) in `src-tauri/src/lib.rs` einfügen:

```rust
#[derive(Serialize, Deserialize, Clone)]
pub struct ActiveInstall {
    pub instance_id: String,
    pub game_id: String,
    pub running: bool,
}

/// Server-side discovery of in-progress or just-finished installs, independent of the local
/// DB - this is what lets a second PC (or the same PC after being closed for days) see and
/// reattach to an install it never started itself. `game_id` is read back out of the rendered
/// install.sh (the game_id isn't otherwise recorded anywhere before GRIMMNETZ_DONE lands).
#[tauri::command]
async fn list_active_installs(state: State<'_, AppState>, server_id: String) -> Result<Vec<ActiveInstall>, String> {
    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();
    let output = session
        .exec(
            "for d in /home/gameserver/instances/*/; do \
               id=$(basename \"$d\"); \
               log=\"$d/install.log\"; \
               [ -f \"$log\" ] || continue; \
               tail -c 4096 \"$log\" | grep -qE 'GRIMMNETZ_DONE|GRIMMNETZ_FAILED' && continue; \
               unit=\"grimmnetz-install-$id\"; \
               active=$(systemctl is-active \"$unit\" 2>/dev/null); \
               [ \"$active\" = \"active\" ] || continue; \
               echo \"$id\"; \
             done",
        )
        .await
        .map_err(|e| e.to_string())?;

    Ok(output
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|id| ActiveInstall { instance_id: id.trim().to_string(), game_id: String::new(), running: true })
        .collect())
}
```

Hinweis für Task 7: `game_id` kommt hier bewusst leer zurück (das Spiel-Icon/-Name lässt sich beim Reattach ohnehin erst über `attach_install_stream`/den bereits offenen App-Store-Dialog zuordnen) - das Frontend zeigt für unbekannte `game_id` einen generischen "Läuft ein Install..."-Platzhalter mit `instance_id`, kein Blocker für Task 7.

- [ ] **Step 2: `cargo check` ausführen**

Erwartet: kompiliert sauber.

- [ ] **Step 3: Command registrieren**

In `tauri::generate_handler![...]` `list_active_installs,` ergänzen.

- [ ] **Step 4: Commit**

```bash
cd "D:/Eigene_Projekte/GrimmNetz" && git add src-tauri/src/lib.rs && git commit -m "Add list_active_installs command for cross-PC install discovery"
```

---

### Task 5: Alten `install_game`-Pfad entfernen

**Files:**
- Modify: `src-tauri/src/lib.rs` - `install_game`-Funktion (ca. Zeile 438-556, exakten Bereich vor dem Löschen mit `grep -n "async fn install_game\|^}" src-tauri/src/lib.rs` verifizieren) komplett entfernen, `install_game,` aus `generate_handler!` entfernen.

**Interfaces:**
- Consumes: nichts Neues
- Produces: nichts - reines Aufräumen, kein Backward-Compat-Shim (Projektregel, siehe Global Constraints).

- [ ] **Step 1: Funktion und Handler-Eintrag entfernen**

Gesamte `async fn install_game(...) { ... }`-Funktion aus `src-tauri/src/lib.rs` löschen. In `generate_handler![...]` die Zeile `install_game,` löschen.

- [ ] **Step 2: `cargo check` ausführen** - stellt sicher, dass nichts anderes im Projekt noch `install_game` referenziert.

Erwartet: kompiliert sauber, keine `unused`-Warnungen zu `InstallEvent`/`parse_steamcmd_progress` (die werden noch von `attach_install_stream` gebraucht).

- [ ] **Step 3: Commit**

```bash
cd "D:/Eigene_Projekte/GrimmNetz" && git add src-tauri/src/lib.rs && git commit -m "Remove old live-streaming install_game - replaced by start_install/attach_install_stream"
```

---

### Task 6: Frontend - `GameStoreDialog.tsx` auf neuen Install-Flow umstellen

**Files:**
- Modify: `src/GameStoreDialog.tsx:78-101` (die `install`-Funktion)

**Interfaces:**
- Consumes: `start_install` (Task 2, Rückgabe: `string` = instance_id), `attach_install_stream` (Task 3, Rückgabe: `InstanceRecord`), bestehende `InstallEvent`/`InstanceRecord`-Typen aus `src/types.ts` (unverändert)
- Produces: gleiche `install(game: GameTemplate)`-Funktionssignatur wie vorher, ruft `onInstalled`/`onInstallProgress`/`onInstallDone` wie bisher auf - keine Änderung an den Eltern-Komponenten nötig.

- [ ] **Step 1: `install`-Funktion ersetzen**

In `src/GameStoreDialog.tsx` die bestehende `install`-Funktion (Zeile 78-101) ersetzen durch:

```tsx
  async function install(game: GameTemplate) {
    setInstallingId(game.id);
    setError("");
    onInstallStart(game);
    try {
      const instanceId = await invoke<string>("start_install", { serverId, gameId: game.id });
      await attachAndWait(instanceId, game.id, game.name);
    } catch (err) {
      setError(String(err));
    } finally {
      setInstallingId(null);
      onInstallDone();
    }
  }

  // The install itself runs entirely server-side now, so a dropped SSH/tail connection is not
  // a failed install - just reattach and keep going. Only a real GRIMMNETZ_FAILED (surfaced as
  // a rejected attach_install_stream call whose message does NOT look like a connection drop)
  // should stop the retry loop and surface as an actual error to the user.
  async function attachAndWait(instanceId: string, gameId: string, displayName: string) {
    for (;;) {
      try {
        const onEvent = new Channel<InstallEvent>();
        onEvent.onmessage = (event) => {
          if (event.event === "step") onInstallProgress(event.label);
          else onInstallProgress(`${event.phase}: ${event.percent.toFixed(0)}%`);
        };
        const instance = await invoke<InstanceRecord>("attach_install_stream", {
          serverId,
          instanceId,
          gameId,
          displayName,
          onEvent,
        });
        onInstalled(instance);
        return;
      } catch (err) {
        const message = String(err);
        const looksLikeDrop = message.includes("Zeitüberschreitung") || message.includes("Verbindung");
        if (!looksLikeDrop) throw err;
        onInstallProgress("Verbindung unterbrochen, verbinde erneut...");
        await new Promise((resolve) => setTimeout(resolve, 2000));
      }
    }
  }
```

- [ ] **Step 2: `npx tsc --noEmit` ausführen**

```bash
cd "D:/Eigene_Projekte/GrimmNetz" && npx tsc --noEmit
```

Erwartet: keine Fehler.

- [ ] **Step 3: Commit**

```bash
cd "D:/Eigene_Projekte/GrimmNetz" && git add src/GameStoreDialog.tsx && git commit -m "Frontend: use start_install/attach_install_stream with reconnect loop"
```

---

### Task 7: Frontend - laufende Installs beim Öffnen des App-Stores anzeigen

**Files:**
- Modify: `src/GameStoreDialog.tsx` (neuer `useEffect`, neuer State, kleine Anzeige-Erweiterung)

**Interfaces:**
- Consumes: `list_active_installs` (Task 4, Rückgabe: `ActiveInstall[]` mit `instance_id: string; game_id: string; running: boolean`), bestehende `attachAndWait` aus Task 6
- Produces: nichts Neues für andere Dateien - rein visuelle Ergänzung innerhalb des Dialogs.

- [ ] **Step 1: Typ in `src/types.ts` ergänzen**

Ans Ende von `src/types.ts` anhängen:

```ts
export type ActiveInstall = {
  instance_id: string;
  game_id: string;
  running: boolean;
};
```

- [ ] **Step 2: State + Discovery-Effect in `GameStoreDialog.tsx`**

Am Anfang der Komponente (bei den bestehenden `useState`-Aufrufen) ergänzen:

```tsx
  const [activeInstalls, setActiveInstalls] = useState<ActiveInstall[]>([]);
```

Und `ActiveInstall` zum bestehenden Type-Import ergänzen (`import type { GameTemplate, InstallEvent, InstanceRecord, ActiveInstall } from "./types";`).

Neuer `useEffect` (z. B. direkt nach dem bestehenden Games-Lade-`useEffect`):

```tsx
  useEffect(() => {
    invoke<ActiveInstall[]>("list_active_installs", { serverId })
      .then(setActiveInstalls)
      .catch(() => setActiveInstalls([]));
  }, [serverId]);
```

- [ ] **Step 3: Reattach-Button für gefundene Installs anzeigen**

Direkt über dem bestehenden `.nx-store-scroll`-Grid (vor der Spiele-Kachel-Liste) einfügen:

```tsx
      {activeInstalls.length > 0 && (
        <div style={{ marginBottom: 16 }}>
          {activeInstalls.map((install) => (
            <div key={install.instance_id} className="nx-fact-card" style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 8 }}>
              <span>Läuft: Installation {install.instance_id.slice(0, 8)}...</span>
              <button
                className="nx-btn nx-btn-primary"
                onClick={() => {
                  setInstallingId(install.instance_id);
                  onInstallStart({ id: install.game_id, name: install.instance_id } as GameTemplate);
                  attachAndWait(install.instance_id, install.game_id, install.instance_id).finally(() => {
                    setInstallingId(null);
                    onInstallDone();
                  });
                }}
              >
                Anzeigen
              </button>
            </div>
          ))}
        </div>
      )}
```

- [ ] **Step 4: `npx tsc --noEmit` ausführen**

Erwartet: keine Fehler.

- [ ] **Step 5: Commit**

```bash
cd "D:/Eigene_Projekte/GrimmNetz" && git add src/types.ts src/GameStoreDialog.tsx && git commit -m "Frontend: show and reattach to installs discovered server-side"
```

---

### Task 8: Live-Verifikation gegen den WSL-Testserver

**Files:** keine Code-Änderungen - reine Verifikation, entspricht dem in diesem Projekt etablierten Muster (kein automatisiertes Test-Framework vorhanden, siehe Global Constraints).

- [ ] **Step 1: Dev-Build starten**

```bash
taskkill //F //IM grimmnetz.exe 2>/dev/null; taskkill //F //IM cargo.exe 2>/dev/null; taskkill //F //IM node.exe 2>/dev/null
cd "D:/Eigene_Projekte/GrimmNetz" && OPENSSL_DIR="C:/Program Files/OpenSSL-Win64" OPENSSL_LIB_DIR="C:/Program Files/OpenSSL-Win64/lib/VC/x64/MD" OPENSSL_INCLUDE_DIR="C:/Program Files/OpenSSL-Win64/include" npm run tauri dev
```

- [ ] **Step 2: Normalfall verifizieren**

Ein Spiel installieren (z. B. Palworld, klein genug für einen schnellen Test), App-Fenster offen lassen, prüfen dass die Fortschrittsanzeige (Step-Label + ggf. Prozent) wie bisher erscheint und die Instanz am Ende als Kachel auftaucht.

- [ ] **Step 3: Abbruch-/Wiederverbindungsfall verifizieren**

Während eines laufenden Installs (idealerweise während des SteamCMD-Downloads) `grimmnetz.exe` hart beenden (Task-Manager oder `taskkill //F //IM grimmnetz.exe`). App neu starten, direkt in den App-Store-Tab des betroffenen Servers gehen, prüfen:
- `list_active_installs` zeigt den laufenden Install als "Läuft: Installation ..."
- Klick auf "Anzeigen" hängt sich per `attach_install_stream` an und zeigt den aktuellen Fortschritt (nicht bei 0% neu startend)
- Install wird nach Abschluss sauber fertig, Instanz erscheint als Kachel

Zusätzlich direkt auf dem WSL-Server verifizieren, dass der Dienst während der App-Downtime weiterlief:
```bash
wsl -e sudo bash -c "systemctl status grimmnetz-install-<instance_id> --no-pager"
```

- [ ] **Step 4: Fehlerfall verifizieren**

Einen Install mit einer absichtlich ungültigen `app_id` im lokal getesteten `games.json` erzwingen (temporär, nicht committen) ODER einen Netzwerkfehler simulieren (WLAN kurz trennen während `steamcmd` läuft, dann direkt einen zweiten, garantiert fehlschlagenden Install eines Spiels mit falscher `app_id` starten). Prüfen:
- `GRIMMNETZ_FAILED:<meldung>` kommt im Frontend als Fehlermeldung an
- Instanzordner wurde auf dem Server aufgeräumt (`wsl -e sudo bash -c "ls /home/gameserver/instances/"` zeigt den Ordner nicht mehr)

- [ ] **Step 5: Ergebnis dem Nutzer berichten**

Kurze Zusammenfassung der drei Testfälle (Normalfall/Abbruch/Fehlerfall) mit Ergebnis - kein Commit für diesen Task, reine Verifikation.

---

## Self-Review-Notizen (bereits eingearbeitet)

- **Spec-Abdeckung:** Skript-Generator (Task 1), `start_install` (Task 2), `attach_install_stream` (Task 3), `list_active_installs` (Task 4), Cutover des alten Pfads (Task 5), Frontend-Umstellung inkl. Reconnect (Task 6), Multi-PC-Anzeige (Task 7), alle drei Testfälle aus der Spec (Task 8) - vollständig abgedeckt.
- **Typkonsistenz geprüft:** `InstanceRecord`-Feldnamen in Task 3 exakt aus dem bestehenden Code (Zeile 543 in `lib.rs`) übernommen; `ActiveInstall` in Task 4 und Task 7 identisch definiert; `attachAndWait`-Signatur in Task 6 und deren Aufruf in Task 7 stimmen überein.
- **Bekannte Unschärfe, die beim Implementieren zu prüfen ist:** Die exakte Fehlertext-Erkennung für "Verbindung abgebrochen vs. echter Fehler" in Task 6 (`looksLikeDrop`) beruht auf den aktuellen deutschen Fehlermeldungen aus `ssh.rs` (`"Zeitüberschreitung beim Ausführen des Befehls"`) - falls sich dieser Text ändert, muss die Prüfung mitgezogen werden. Alternative, robustere Lösung (falls sich das beim Testen als zu fragil erweist): einen eigenen Rust-seitigen Fehlertyp/-präfix einführen, der SSH-Drops explizit von `GRIMMNETZ_FAILED` unterscheidet.
