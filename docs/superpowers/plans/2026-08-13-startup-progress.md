# Startup-Fortschrittsanzeige Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Statt sofortigem "Online"-Badge zeigt GrimmNetz für Spiele mit definierten Startup-Meilensteinen (zunächst nur 7 Days to Die) einen Ladebalken mit Prozentangabe, solange der Docker-Container zwar läuft, der Spiel-Server-Prozess aber laut Logs noch nicht fertig gestartet ist.

**Architecture:** `GameTemplate` bekommt eine optionale, geordnete Liste von Text-Muster→Prozent-Meilensteinen. `get_instance_status` durchsucht bei aktiver Unit zusätzlich `docker logs --tail 200` nach dem höchsten erreichten Meilenstein und liefert das als neues `startup_percent`-Feld zurück. Das Frontend zeigt bei `startup_percent < 100` einen Ladebalken statt des Online-Badges.

**Tech Stack:** Rust (`serde`, bestehende `games.rs`/`lib.rs`-Module), `games.json`-Ressourcendatei, React/TypeScript (`App.tsx`).

## Global Constraints

- Spiele ohne `startup_milestones` (alle außer 7DTD in dieser Iteration) verhalten sich exakt wie bisher - keine Verhaltensänderung, kein Risiko für bestehende Installationen.
- `docker logs`-Scan ist best-effort - ein Fehlschlag darf `get_instance_status` nicht insgesamt fehlschlagen lassen, nur `startup_percent: None` liefern.
- Meilenstein-Suche ist reine Substring-Suche, keine Regex (siehe Spec, Abschnitt "games.rs").
- Dieses Repo hat keine automatisierten Tests - Verifikation läuft über `cargo check`/`npm run build` für Kompilierbarkeit und abschließendes manuelles Live-Testing gegen einen echten Server.
- Alle User-sichtbaren Strings auf Deutsch, im bestehenden Ton der Codebase.

---

### Task 1: `games.rs` - `StartupMilestone`-Struct

**Files:**
- Modify: `src-tauri/src/games.rs` (`GameTemplate`-Struct-Definition, `grep -n "pub struct GameTemplate"` zur genauen Stelle)

**Interfaces:**
- Produces: `pub struct StartupMilestone { pub pattern: String, pub percent: u8 }` (beide Felder `Serialize`/`Deserialize`/`Clone`). `GameTemplate.startup_milestones: Vec<StartupMilestone>` mit `#[serde(default)]`.

- [ ] **Step 1: `StartupMilestone` einführen**

In `src-tauri/src/games.rs`, direkt vor `pub struct GameTemplate` einfügen:

```rust
/// Ein einzelner Fortschritts-Meilenstein beim Hochfahren einer Spiel-Instanz - wird gegen die
/// letzten Docker-Log-Zeilen geprüft, um dem User einen echten Fortschritt statt eines binären
/// "läuft/läuft nicht" zu zeigen (Docker-Container können lange nach dem eigentlichen
/// Containerstart noch mit Weltgenerierung o.ä. beschäftigt sein).
#[derive(Serialize, Deserialize, Clone)]
pub struct StartupMilestone {
    /// Text, der als Teilstring (keine Regex) in einer Docker-Log-Zeile gesucht wird.
    pub pattern: String,
    /// Fortschritt in Prozent, sobald `pattern` gefunden wurde. Der höchste unter den in den
    /// gescannten Zeilen gefundenen Meilensteinen gewinnt.
    pub percent: u8,
}
```

Dann in `GameTemplate` (bei den anderen `#[serde(default)]`-Feldern wie `tested_on`/`ports`) ergänzen:

```rust
    /// Geordnete Fortschritts-Meilensteine für die Startup-Ladebalken-Anzeige - leer (Standard)
    /// bedeutet: dieses Spiel zeigt weiterhin sofort "Online", sobald die Unit aktiv ist.
    #[serde(default)]
    pub startup_milestones: Vec<StartupMilestone>,
```

- [ ] **Step 2: Kompilierbarkeit prüfen**

Run: `cd src-tauri && OPENSSL_DIR="C:/Program Files/OpenSSL-Win64" OPENSSL_LIB_DIR="C:/Program Files/OpenSSL-Win64/lib/VC/x64/MD" OPENSSL_INCLUDE_DIR="C:/Program Files/OpenSSL-Win64/include" cargo check --no-default-features`
Expected: keine Fehler.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/games.rs
git commit -m "feat(games): StartupMilestone-Struct fuer Fortschritts-Meilensteine"
```

---

### Task 2: `lib.rs` - `get_instance_status` um Log-Scan erweitern

**Files:**
- Modify: `src-tauri/src/lib.rs:1393-1432` (`InstanceStatus`-Struct, `get_instance_status`)

**Interfaces:**
- Consumes: `games::StartupMilestone { pattern: String, percent: u8 }`, `GameTemplate.startup_milestones` (Task 1). `games::find_template(game_id: &str) -> Option<&GameTemplate>` (bereits vorhanden, siehe `lib.rs` andere Aufrufstellen). `get_instance_status`s bestehende Parameter sind `server_id: String, unit_name: String` - für die Meilenstein-Suche wird zusätzlich die `game_id` gebraucht, die aktuell NICHT übergeben wird. Diese Aufgabe erweitert die Signatur um einen neuen optionalen Parameter.
- Produces: `InstanceStatus.startup_percent: Option<u8>`.

- [ ] **Step 1: `InstanceStatus` um `startup_percent` erweitern**

In `src-tauri/src/lib.rs`, `InstanceStatus`-Struct (aktuell `state`, `uptime_seconds`, `pid`, `started_at`) um ein Feld ergänzen:

```rust
pub struct InstanceStatus {
    pub state: String,
    pub uptime_seconds: i64,
    pub pid: Option<i64>,
    pub started_at: Option<String>,
    pub startup_percent: Option<u8>,
}
```

- [ ] **Step 2: `get_instance_status`-Signatur um `game_id` erweitern**

`get_instance_status` bekommt einen neuen Parameter `game_id: Option<String>` (optional, damit bestehende Frontend-Aufrufe, die diesen Parameter noch nicht mitschicken, nicht sofort brechen - wird in Task 4 überall nachgezogen, aber Rückwärtskompatibilität kostet hier nichts):

```rust
#[tauri::command]
async fn get_instance_status(
    state: State<'_, AppState>,
    server_id: String,
    unit_name: String,
    game_id: Option<String>,
) -> Result<InstanceStatus, String> {
```

- [ ] **Step 3: Meilenstein-Scan nach dem bestehenden `systemctl`-Aufruf einfügen**

Direkt vor der bestehenden letzten Zeile `Ok(InstanceStatus { state, uptime_seconds, pid, started_at })` folgenden Block einfügen (ersetzt diese letzte Zeile durch den Block plus eine neue, um `startup_percent` erweiterte Abschlusszeile):

```rust
    let startup_percent: Option<u8> = if state == "active" {
        if let Some(tpl) = game_id.as_deref().and_then(games::find_template) {
            if !tpl.startup_milestones.is_empty() {
                match session.exec(&format!("docker logs --tail 200 {unit_name} 2>&1")).await {
                    Ok(logs) => tpl
                        .startup_milestones
                        .iter()
                        .filter(|m| logs.contains(&m.pattern))
                        .map(|m| m.percent)
                        .max(),
                    Err(_) => None,
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    Ok(InstanceStatus { state, uptime_seconds, pid, started_at, startup_percent })
```

Wichtig: `session.exec(...)` erwartet `&mut SshSession` - `session` ist im bestehenden Funktionskörper bereits als `guard.as_mut().unwrap()` gebunden (siehe die Zeile, die den `systemctl`-Befehl absetzt) und bleibt gültig für diesen zusätzlichen Aufruf, da beide `.exec(...)`-Aufrufe sequenziell auf derselben `session`-Referenz laufen.

- [ ] **Step 4: Handler-Registrierung prüfen**

`get_instance_status` ist bereits im `tauri::generate_handler!`-Aufruf registriert (Signaturänderungen an bereits registrierten Commands brauchen keine erneute Registrierung) - trotzdem `grep -n "get_instance_status" src-tauri/src/lib.rs` ausführen, um zu bestätigen, dass keine zweite, jetzt inkonsistente Definition existiert.

- [ ] **Step 5: Kompilierbarkeit prüfen**

Run: `cd src-tauri && OPENSSL_DIR="C:/Program Files/OpenSSL-Win64" OPENSSL_LIB_DIR="C:/Program Files/OpenSSL-Win64/lib/VC/x64/MD" OPENSSL_INCLUDE_DIR="C:/Program Files/OpenSSL-Win64/include" cargo check --no-default-features`
Expected: keine Fehler in `lib.rs` selbst. Der Frontend-`invoke`-Aufruf von `get_instance_status` übergibt `game_id` noch nicht (Task 4 holt das nach) - das ist kein Compile-Fehler, `game_id: Option<String>` fehlt beim `invoke`-Aufruf einfach als `undefined`/nicht gesetzt, was Tauri als `None` interpretiert (optionale Parameter).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat: get_instance_status scannt Docker-Logs nach Startup-Meilensteinen"
```

---

### Task 3: `games.json` - 7DTD-Meilensteine ergänzen

**Files:**
- Modify: `src-tauri/resources/games.json` (7DTD-Eintrag)

**Interfaces:**
- Consumes: `startup_milestones`-Feld-Schema aus Task 1 (`[{"pattern": "...", "percent": N}]`).

- [ ] **Step 1: Meilensteine in den 7DTD-`install`-Block einfügen**

In `src-tauri/resources/games.json`, im 7DTD-Eintrag (`grep -n '"id": "7dtd"' -A 5 src-tauri/resources/games.json` zur genauen Stelle), direkt nach dem `"install": { ... }`-Block (auf derselben Ebene wie `"install"`, `"config"`, `"ports"` - also als eigenes Top-Level-Feld des 7DTD-Objekts, nicht innerhalb von `"install"`) folgendes Feld ergänzen:

```json
"startup_milestones": [
  { "pattern": "Starting periodic command scheduler cron", "percent": 10 },
  { "pattern": "createWorld:", "percent": 25 },
  { "pattern": "Started thread ChunkRegeneration", "percent": 40 },
  { "pattern": "Started thread GenerateChunks", "percent": 55 },
  { "pattern": "Calculating world hashes", "percent": 70 },
  { "pattern": "GameServer.LogOn successful", "percent": 85 },
  { "pattern": "StartGame done", "percent": 100 }
],
```

- [ ] **Step 2: JSON-Validität prüfen**

Run: `cd src-tauri && node -e "JSON.parse(require('fs').readFileSync('resources/games.json', 'utf8')); console.log('valid')"`
Expected: `valid`

- [ ] **Step 3: Kompilierbarkeit prüfen**

Run: `cd src-tauri && OPENSSL_DIR="C:/Program Files/OpenSSL-Win64" OPENSSL_LIB_DIR="C:/Program Files/OpenSSL-Win64/lib/VC/x64/MD" OPENSSL_INCLUDE_DIR="C:/Program Files/OpenSSL-Win64/include" cargo check --no-default-features`
Expected: keine Fehler.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/resources/games.json
git commit -m "feat: 7DTD-Startup-Meilensteine fuer Ladebalken-Anzeige"
```

---

### Task 4: Frontend - Ladebalken statt sofortigem Online-Badge

**Files:**
- Modify: `src/types.ts` (`InstanceStatus`-Type)
- Modify: `src/App.tsx:204-208, 339-340, 359-363` (drei `get_instance_status`-Aufrufstellen), `src/App.tsx:716-732` (Status-Label-Logik), `src/App.tsx:770-774` (Render)

**Interfaces:**
- Consumes: `InstanceStatus.startup_percent: number | null` (Backend liefert `Option<u8>`, das JSON-serialisiert zu `number | null` wird, keine explizite `undefined`-Behandlung nötig).

- [ ] **Step 1: `InstanceStatus`-Type erweitern**

In `src/types.ts`, `InstanceStatus`-Type um ein Feld ergänzen:

```typescript
export type InstanceStatus = {
  state: string;
  uptime_seconds: number;
  pid: number | null;
  started_at: string | null;
  startup_percent: number | null;
};
```

- [ ] **Step 2: Alle drei `get_instance_status`-Aufrufstellen um `gameId` ergänzen**

In `src/App.tsx`, jede der drei Stellen, die `invoke<InstanceStatus>("get_instance_status", { serverId: ..., unitName: ... })` aufrufen (Zeilen ~204-208, ~339-340, ~359-363 - exakte Zeilen mit `grep -n "get_instance_status" src/App.tsx` verifizieren, da sich Zeilennummern durch frühere Session-Änderungen verschoben haben könnten), um `gameId: instance.game_id` als drittes Argument im übergebenen Objekt ergänzen. Beispiel für die erste Fundstelle (Struktur der anderen beiden ist identisch, nur mit ggf. anderem Variablennamen für die Instanz - `instance.game_id` bzw. je nach Kontext `i.game_id`/o.ä., den tatsächlich im jeweiligen Scope sichtbaren Namen verwenden):

```typescript
const status = await invoke<InstanceStatus>("get_instance_status", {
  serverId: selectedServerId,
  unitName: instance.systemd_unit,
  gameId: instance.game_id,
});
```

- [ ] **Step 3: Ladebalken-Anzeige in der Instanz-Karte ergänzen**

In `src/App.tsx`, im `instances.map((instance) => { ... })`-Block: nach der bestehenden Zeile

```tsx
{status && <div className="nx-instance-card-sub">Uptime: {formatUptime(status.uptime_seconds)}</div>}
```

Folgendes ergänzen:

```tsx
{status?.startup_percent != null && status.startup_percent < 100 && (
  <div style={{ marginTop: 4 }}>
    <div
      style={{
        height: 6,
        borderRadius: 3,
        background: "var(--nx-border)",
        overflow: "hidden",
      }}
    >
      <div
        style={{
          height: "100%",
          width: `${status.startup_percent}%`,
          background: "var(--nx-accent)",
          transition: "width 0.3s ease",
        }}
      />
    </div>
    <div style={{ fontSize: 11, color: "var(--nx-text-muted)", marginTop: 2 }}>
      Startet… {status.startup_percent}%
    </div>
  </div>
)}
```

Zusätzlich in der bestehenden `statusLabel`-Berechnung (Zeilen ~722-732): wenn `status?.startup_percent != null && status.startup_percent < 100`, soll `statusLabel` NICHT "Online" anzeigen, sondern weiterhin den bisherigen "Online"-Text durch den Ladebalken darunter ergänzt/kontextualisiert werden - der Ladebalken kommt zusätzlich zum Status, ersetzt ihn nicht, da `isActive` (systemd-Zustand) und "spielbereit" zwei unterschiedliche Dinge sind, die beide sichtbar bleiben sollen. Kein Code-Änderung an der `statusLabel`-Logik selbst nötig - nur die neue Ladebalken-Anzeige darunter, wie oben gezeigt.

- [ ] **Step 4: Kompilierbarkeit/Typecheck prüfen**

Run: `npm run build` (von `D:\Eigene_Projekte\GrimmNetz\.worktrees\<worktree-name>` aus, TypeScript/Vite-Build)
Expected: keine Fehler.

- [ ] **Step 5: Commit**

```bash
git add src/types.ts src/App.tsx
git commit -m "feat(ui): Ladebalken mit Startup-Prozent statt sofortigem Online-Badge"
```

---

### Task 5: Live-Verifikation (Hetzner)

**Files:** keine Code-Änderungen, reines Testing (dieses Projekt hat keine automatisierten Tests, siehe Global Constraints).

- [ ] **Step 1: App neu bauen und starten**

```bash
cd src-tauri && cargo clean -p grimmnetz
```

Dann `npm run tauri dev` starten (mit den üblichen `OPENSSL_*`-Env-Vars).

- [ ] **Step 2: 7DTD frisch installieren und den Ladebalken live beobachten**

7 Days to Die über die App neu installieren (auf dem echten Hetzner-Testserver, IP `167.233.213.80`, Server-ID bereits in der lokalen DB vorhanden). Erwartung: nach dem Containerstart zeigt die Instanz-Karte einen Ladebalken, der sich schrittweise laut den in Task 3 definierten Prozentwerten erhöht (10 → 25 → 40 → 55 → 70 → 85 → 100), und erst bei 100% (bzw. danach) verschwindet der Balken und die Karte zeigt normal "Online".

- [ ] **Step 3: Regressionscheck für Spiele ohne Meilensteine**

Minecraft oder Factorio (haben kein `startup_milestones`-Feld) neu starten/stoppen und prüfen, dass sich am "Online"/"Gestoppt"-Verhalten nichts geändert hat - kein Ladebalken darf dort auftauchen.

- [ ] **Step 4: Patch-Notes, Version bumpen, committen**

Analog zum bisherigen Vorgehen dieser Session: `src/patchNotes.ts` neuen Eintrag, Version in `package.json`/`src-tauri/tauri.conf.json`/`src-tauri/Cargo.toml` bumpen, `cargo check` zur Aktualisierung von `Cargo.lock`, dann committen, PR erstellen, mergen, taggen, Release veröffentlichen (gleiches Muster wie bei den vorherigen Releases dieser Session).
