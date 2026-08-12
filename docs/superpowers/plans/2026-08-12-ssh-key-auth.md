# SSH-Key-Authentifizierung Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** GrimmNetz verbindet sich bevorzugt per SSH-Key statt Passwort - pro Server ein eigenes ED25519-Schlüsselpaar, automatisch generiert und verwaltet, nie vom User gesehen (nur der Public Key wird ihm bei Bedarf gezeigt).

**Architecture:** Neues Modul `ssh_keys.rs` kapselt Key-Erzeugung/-Formatierung/-Ausrollen. `ssh.rs` bekommt einen zweiten Auth-Pfad (`connect_key` neben `connect_password`) über einen gemeinsamen internen Connect-Helfer. `lib.rs` bevorzugt beim Verbinden immer zuerst einen im OS-Keyring hinterlegten Key, fällt bei Fehlschlag auf Passwort zurück und rollt danach automatisch einen (neuen) Key aus. Zwei neue Tauri-Commands decken den "neuer Cloud-Server"-Weg ab, bei dem der Key schon vor der VM-Existenz generiert wird.

**Tech Stack:** Rust (`russh` 0.45, `russh-keys` 0.45 - beide bereits Abhängigkeiten, kein neuer Crate nötig), bestehendes `keyring`-Crate für Secret-Storage, React/TypeScript (Tauri `invoke`).

## Global Constraints

- Kein neuer Crate für Schlüsselerzeugung/-Formatierung - `russh_keys::key::KeyPair::generate_ed25519()` und `russh_keys::PublicKeyBase64::public_key_base64()` sind bereits vorhanden (siehe Spec, Abschnitt "Key-Erzeugung & Format").
- Private Keys werden wie das Passwort ausschließlich über den OS-Keyring gespeichert (`keyring_store.rs`-Muster), nie auf Disk.
- Pro-Server-Keys, kein globaler App-Key (Blast-Radius-Prinzip, siehe Spec).
- Dieses Repo hat keine automatisierten Tests (verifiziert: kein `#[test]` im gesamten `src-tauri/src`) - Verifikation läuft wie im übrigen Projekt über `cargo check`/`cargo build` für Kompilierbarkeit und abschließendes manuelles Live-Testing gegen einen echten Server, nicht über eine neu eingeführte Unit-Test-Suite.
- Alle User-sichtbaren Strings auf Deutsch, im bestehenden Ton der Codebase (siehe vorhandene Fehlermeldungen in `lib.rs`/`ssh.rs`).

---

### Task 1: `ssh_keys.rs` - Key-Erzeugung, Serialisierung, Ausroll-Kommando

**Files:**
- Create: `src-tauri/src/ssh_keys.rs`
- Modify: `src-tauri/src/lib.rs:1-20` (Modul registrieren: `mod ssh_keys;` neben den bestehenden `mod`-Deklarationen)

**Interfaces:**
- Produces:
  - `pub fn generate_and_format(server_id: &str) -> anyhow::Result<(russh_keys::key::KeyPair, String, String)>` - gibt `(keypair, private_pem, public_line)` zurück. `private_pem` ist das über `russh_keys::encode_pkcs8_pem` erzeugte, im Keyring speicherbare String-Format. `public_line` ist `ssh-ed25519 <base64> grimmnetz-{server_id}`.
  - `pub fn load_keypair(private_pem: &str) -> anyhow::Result<russh_keys::key::KeyPair>` - Gegenstück zum Laden aus dem Keyring.
  - `pub fn install_command(public_line: &str, username: &str) -> String` - baut die idempotente `authorized_keys`-Shell-Pipeline als fertigen Befehlsstring.

- [ ] **Step 1: Modul-Grundgerüst mit Key-Erzeugung schreiben**

```rust
use anyhow::{anyhow, Result};
use russh_keys::key::KeyPair;
use russh_keys::PublicKeyBase64;

/// Erzeugt ein neues ED25519-Schlüsselpaar für einen Server und liefert es in den zwei
/// Formen, die der Rest der App braucht: `private_pem` zum Ablegen im OS-Keyring (gleiches
/// Muster wie das Passwort in `keyring_store.rs`), `public_line` zum Ausrollen in
/// `authorized_keys` bzw. zum Anzeigen im "Neuer Cloud-Server"-Dialog.
pub fn generate_and_format(server_id: &str) -> Result<(KeyPair, String, String)> {
    let keypair = KeyPair::generate_ed25519().ok_or_else(|| anyhow!("Schlüsselerzeugung fehlgeschlagen"))?;
    let private_pem = encode_private_pem(&keypair)?;
    let public_line = format!("ssh-ed25519 {} grimmnetz-{server_id}", keypair.public_key_base64());
    Ok((keypair, private_pem, public_line))
}

/// Serialisiert ein Schlüsselpaar als PKCS8-PEM-String - textbasiert, damit es sich wie das
/// Passwort im OS-Keyring (das nur Strings speichert) ablegen lässt.
fn encode_private_pem(keypair: &KeyPair) -> Result<String> {
    let mut buf = Vec::new();
    russh_keys::encode_pkcs8_pem(keypair, &mut buf).map_err(|e| anyhow!("Key-Serialisierung fehlgeschlagen: {e}"))?;
    Ok(String::from_utf8(buf)?)
}

/// Lädt ein zuvor mit `generate_and_format` erzeugtes und im Keyring abgelegtes Schlüsselpaar
/// wieder ein.
pub fn load_keypair(private_pem: &str) -> Result<KeyPair> {
    russh_keys::decode_secret_key(private_pem, None).map_err(|e| anyhow!("Key-Laden fehlgeschlagen: {e}"))
}

/// Baut die idempotente Shell-Pipeline, die den Public Key in `authorized_keys` des Zielnutzers
/// einträgt - läuft nach erfolgreichem Passwort-Login über die offene SSH-Session. `grep -qxF`
/// verhindert Duplikate bei wiederholten Ausroll-Versuchen (z.B. nach einem App-Absturz
/// zwischen Ausrollen und Speichern des Erfolgsstatus). Die `chmod`-Aufrufe sind nötig, da SSH
/// Keys in zu offen berechtigten `.ssh`-Verzeichnissen/Dateien ignoriert.
pub fn install_command(public_line: &str, username: &str) -> String {
    let home = if username == "root" { "/root".to_string() } else { format!("/home/{username}") };
    let quoted = crate::games::shell_single_quote(public_line);
    format!(
        "mkdir -p {home}/.ssh && chmod 700 {home}/.ssh && \
         (grep -qxF {quoted} {home}/.ssh/authorized_keys 2>/dev/null || echo {quoted} >> {home}/.ssh/authorized_keys) && \
         chmod 600 {home}/.ssh/authorized_keys && chown -R {username}:{username} {home}/.ssh"
    )
}
```

- [ ] **Step 2: Modul registrieren**

In `src-tauri/src/lib.rs`, bei den bestehenden `mod`-Deklarationen (direkt neben `mod ssh;` bzw. `mod keyring_store;`) ergänzen:

```rust
mod ssh_keys;
```

- [ ] **Step 3: Kompilierbarkeit prüfen**

Run: `cd src-tauri && cargo check --no-default-features` (mit den üblichen `OPENSSL_*`-Env-Vars, siehe README)
Expected: keine Fehler. `install_command`/`load_keypair`/`generate_and_format` werden noch nirgends aufgerufen - `dead_code`-Warnungen sind an dieser Stelle normal und werden mit Task 4/5 verschwinden.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/ssh_keys.rs src-tauri/src/lib.rs
git commit -m "feat: ssh_keys-Modul für Key-Erzeugung, Serialisierung und Ausroll-Kommando"
```

---

### Task 2: `ssh.rs` - `connect_key` als zweiter Auth-Pfad

**Files:**
- Modify: `src-tauri/src/ssh.rs:53-133` (bestehende `connect_password`/`connect_password_inner`)

**Interfaces:**
- Consumes: `russh_keys::key::KeyPair` aus Task 1.
- Produces: `pub async fn connect_key(host: &str, port: u16, username: &str, keypair: std::sync::Arc<russh_keys::key::KeyPair>, expected_fingerprint: Option<&str>) -> anyhow::Result<Self>` - gleiche Timeout-/Retry-/Host-Key-Pinning-Semantik wie `connect_password`.

Refactort die bestehende Retry-Schleife und Handshake-Logik in einen gemeinsamen internen Pfad, damit beide Auth-Methoden (Passwort/Key) dieselbe Timeout-, Retry- und Host-Key-Pinning-Behandlung durchlaufen, statt sie zu duplizieren.

- [ ] **Step 1: `AuthMethod`-Enum einführen und bestehenden Code darauf umstellen**

Ersetze in `src-tauri/src/ssh.rs` die Funktionen `connect_password` und `connect_password_inner` (Zeilen 53-133) durch:

```rust
enum AuthMethod {
    Password(String),
    PublicKey(Arc<key::KeyPair>),
}

impl SshSession {
    pub async fn connect_password(
        host: &str,
        port: u16,
        username: &str,
        password: &str,
        expected_fingerprint: Option<&str>,
    ) -> Result<Self> {
        Self::connect(host, port, username, AuthMethod::Password(password.to_string()), expected_fingerprint).await
    }

    /// Wie `connect_password`, nur per SSH-Public-Key-Authentifizierung statt Passwort - siehe
    /// `ssh_keys.rs` für Erzeugung/Laden des Schlüsselpaars. Wird von `lib.rs` immer zuerst
    /// versucht, wenn für einen Server ein Key im OS-Keyring hinterlegt ist.
    pub async fn connect_key(
        host: &str,
        port: u16,
        username: &str,
        keypair: Arc<key::KeyPair>,
        expected_fingerprint: Option<&str>,
    ) -> Result<Self> {
        Self::connect(host, port, username, AuthMethod::PublicKey(keypair), expected_fingerprint).await
    }

    async fn connect(
        host: &str,
        port: u16,
        username: &str,
        auth: AuthMethod,
        expected_fingerprint: Option<&str>,
    ) -> Result<Self> {
        // Fresh TCP connects sometimes get an immediate "connection refused" through
        // transient local network hiccups (e.g. WSL2's localhost port-forwarding relay
        // blipping) even though the remote is fine a moment later - a couple of quick
        // retries absorb that instead of failing the whole action on a one-off glitch.
        let mut last_err = None;
        for attempt in 0..3 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
            match tokio::time::timeout(
                CONNECT_TIMEOUT,
                Self::connect_inner(host, port, username, &auth, expected_fingerprint),
            )
            .await
            {
                Ok(Ok(session)) => return Ok(session),
                Ok(Err(e)) => {
                    // A host key mismatch is a security-relevant refusal, not a transient
                    // hiccup - never retry past it, and never let a later attempt's generic
                    // error paper over what actually happened.
                    let is_mismatch = e.to_string().contains("Host-Key");
                    last_err = Some(e);
                    if is_mismatch {
                        break;
                    }
                }
                Err(_) => last_err = Some(anyhow!("Zeitüberschreitung beim Verbindungsaufbau (Server nicht erreichbar?)")),
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow!("Verbindung fehlgeschlagen")))
    }

    async fn connect_inner(
        host: &str,
        port: u16,
        username: &str,
        auth: &AuthMethod,
        expected_fingerprint: Option<&str>,
    ) -> Result<Self> {
        let observed = Arc::new(StdMutex::new(None));
        let mismatch = Arc::new(StdMutex::new(false));
        let handler = ClientHandler {
            expected: expected_fingerprint.map(str::to_string),
            observed: observed.clone(),
            mismatch: mismatch.clone(),
        };
        let config = Arc::new(client::Config::default());
        let mut handle = match client::connect(config, (host, port), handler).await {
            Ok(h) => h,
            Err(e) => {
                if *mismatch.lock().unwrap() {
                    let seen = observed.lock().unwrap().clone().unwrap_or_default();
                    return Err(anyhow!(
                        "Host-Key hat sich geändert! Erwartet: {}, jetzt gesehen: {seen}. Das ist entweder ein neu \
                         aufgesetzter Server oder ein möglicher Man-in-the-Middle-Angriff - Verbindung abgebrochen. \
                         Falls der Server absichtlich neu aufgesetzt wurde, bestätige das über \"Server bearbeiten\".",
                        expected_fingerprint.unwrap_or("?")
                    ));
                }
                return Err(e.into());
            }
        };
        let authenticated = match auth {
            AuthMethod::Password(password) => handle.authenticate_password(username, password).await?,
            AuthMethod::PublicKey(keypair) => handle.authenticate_publickey(username, keypair.clone()).await?,
        };
        if !authenticated {
            return Err(anyhow!("SSH-Authentifizierung fehlgeschlagen"));
        }
        let host_fingerprint = observed
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| anyhow!("Kein Host-Key vom Server empfangen"))?;
        Ok(Self { handle, host_fingerprint })
    }
```

Der Rest von `impl SshSession` (ab `pub async fn exec`) bleibt unverändert.

- [ ] **Step 2: Kompilierbarkeit prüfen**

Run: `cd src-tauri && cargo check --no-default-features`
Expected: keine Fehler. `connect_key` wird noch nirgends aufgerufen - `dead_code`-Warnung erwartet, verschwindet mit Task 4.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/ssh.rs
git commit -m "refactor: ssh.rs auf gemeinsamen Connect-Pfad für Passwort/Key umgestellt, connect_key ergänzt"
```

---

### Task 3: `db.rs` - `host` nullable, `auth_method` tatsächlich schreiben

**Files:**
- Modify: `src-tauri/src/db.rs:1-170` (Schema-Migration, `ServerRecord.host`, `insert_server`, `update_server`, `find_server_by_host_port`, `get_server`)

**Interfaces:**
- Produces: `ServerRecord.host: Option<String>` (bisher `String`). `insert_server` akzeptiert weiterhin einen vollständigen `ServerRecord` (unverändert in der Signatur, aber `host` jetzt optional). Neue Funktion `pub fn set_host(&self, id: &str, host: &str, port: u16) -> rusqlite::Result<()>` zum Nachtragen der IP für Weg-B-Server.

Die Spalte `host` ist aktuell `NOT NULL` - Weg B (Task 6) legt einen Server-Eintrag an, bevor die IP bekannt ist. SQLite kann eine `NOT NULL`-Constraint nicht per `ALTER TABLE` entfernen, daher eine einmalige Tabellen-Neuerstellung, nach demselben Best-Effort-Migrationsmuster wie die bestehenden `ALTER TABLE ... ADD COLUMN`-Zeilen in `Db::open`.

- [ ] **Step 1: Migration in `Db::open` ergänzen**

In `src-tauri/src/db.rs`, nach den bestehenden Migrations-Zeilen (nach `let _ = conn.execute("ALTER TABLE servers ADD COLUMN known_host_fingerprint TEXT", []);`, vor `Ok(Self { conn })`):

```rust
        // Migration: `host` muss nullable sein, damit Weg B (Server-Key vor VM-Existenz
        // generieren, siehe ssh_keys.rs/prepare_key_only_server) einen Eintrag ohne IP anlegen
        // kann. SQLite kann NOT NULL nicht per ALTER entfernen, daher Tabellen-Neuerstellung -
        // nur nötig, wenn die alte NOT-NULL-Variante noch besteht (idempotent über PRAGMA-Check).
        let host_is_not_null: bool = conn
            .prepare("SELECT \"notnull\" FROM pragma_table_info('servers') WHERE name = 'host'")?
            .query_row([], |row| row.get::<_, i64>(0))
            .unwrap_or(0)
            == 1;
        if host_is_not_null {
            conn.execute_batch(
                "
                CREATE TABLE servers_new (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    host TEXT,
                    port INTEGER NOT NULL DEFAULT 22,
                    username TEXT NOT NULL,
                    auth_method TEXT NOT NULL,
                    os_info TEXT,
                    known_host_fingerprint TEXT,
                    created_at TEXT NOT NULL
                );
                INSERT INTO servers_new SELECT id, name, host, port, username, auth_method, os_info, known_host_fingerprint, created_at FROM servers;
                DROP TABLE servers;
                ALTER TABLE servers_new RENAME TO servers;
                ",
            )?;
        }
```

- [ ] **Step 2: `ServerRecord.host` auf `Option<String>` umstellen**

In `src-tauri/src/db.rs:10-23`, `pub host: String,` ändern zu `pub host: Option<String>,`.

- [ ] **Step 3: `insert_server`, `find_server_by_host_port`, `get_server` an das optionale Feld anpassen**

`insert_server` (Zeile 76-90) übergibt `record.host` bereits per `params!`, was mit `Option<String>` automatisch funktioniert (rusqlite unterstützt `Option<T>` nativ als NULL/Wert) - keine Änderung am Funktionskörper nötig, nur der Typ in `ServerRecord` ändert sich.

`find_server_by_host_port` (Zeile 118) und `get_server` (Zeile 157) lesen `host` per `row.get(...)` in einen `ServerRecord` - prüfe, dass dort `row.get::<_, Option<String>>(...)` (oder das Äquivalent über `?` mit dem jetzt geänderten Feldtyp) verwendet wird; falls die bestehende Query per Spaltennamen/Index ohne expliziten Typ liest (typisch `row.get(2)?`), reicht die Typänderung in `ServerRecord` bereits aus, da `rusqlite` den Zieltyp aus der Struct-Definition ableitet - keine SQL-Änderung nötig, nur sicherstellen, dass kein `.unwrap()` direkt auf dem gelesenen Host-Wert hängt.

- [ ] **Step 4: `set_host` ergänzen**

Nach `update_server` (Zeile 106-117) ergänzen:

```rust
    /// Trägt Host/Port für einen per Weg B (Key-vor-VM) angelegten Server nach, sobald die VM
    /// beim Provider existiert und der User die IP in der App einträgt.
    pub fn set_host(&self, id: &str, host: &str, port: u16) -> rusqlite::Result<()> {
        self.conn
            .execute("UPDATE servers SET host = ?1, port = ?2 WHERE id = ?3", params![host, port, id])?;
        Ok(())
    }
```

- [ ] **Step 5: Kompilierbarkeit prüfen und Fehler an den Call-Sites beheben**

Run: `cd src-tauri && cargo check --no-default-features`

Erwartete Fehler: jede Stelle in `lib.rs`, die `server.host` als `&str`/`String` ohne Entpacken verwendet, schlägt jetzt fehl (Typ ist jetzt `Option<String>`). Betroffene Stellen (per `grep -n "server\.host\|record\.host" src-tauri/src/lib.rs` verifiziert):
- `lib.rs:166` (`connect_fresh`, `&server.host`)
- `lib.rs:248` (`add_server`, `host: input.host` - `input.host` bleibt `String` aus dem UI-Formular, hier nur `Some(input.host)` setzen)
- `lib.rs:280` (`enable_firewall`, Destrukturierung `(server.host, ...)`)
- `lib.rs:931` (`db.get_server(...)?.host`)

Diese Stellen werden in Task 4/5 ohnehin angepasst (sie brauchen die Fallback-Logik aus `connect_with_key_or_password`) - an dieser Stelle reicht ein `.ok_or_else(|| "Server hat noch keine IP-Adresse".to_string())?` bzw. `.expect(...)` als Platzhalter-Fix, damit Task 3 für sich kompiliert; Task 4/5 ersetzen die betroffenen Funktionskörper ohnehin vollständig.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/db.rs
git commit -m "feat(db): host-Spalte nullable für Weg-B-Server (Key vor VM-Existenz), set_host ergänzt"
```

---

### Task 4: `lib.rs` - Key-first-Verbindungslogik + Auto-Enroll für Bestandsserver

**Files:**
- Modify: `src-tauri/src/lib.rs:159-202` (`connect_fresh`, `acquire_session`)
- Modify: `src-tauri/src/lib.rs:204-258` (`add_server`)

**Interfaces:**
- Consumes: `ssh_keys::generate_and_format`, `ssh_keys::load_keypair`, `ssh_keys::install_command` (Task 1); `ssh::SshSession::connect_key` (Task 2); `keyring_store::get_secret`/`store_secret` (bestehend); `ServerRecord.host: Option<String>` (Task 3).
- Produces: `async fn connect_with_key_or_password(host: &str, port: u16, username: &str, server_id: &str, known_fingerprint: Option<&str>) -> Result<ssh::SshSession, String>` - zentraler Verbindungs-Helfer, den `connect_fresh` und `add_server` beide nutzen.

Namenskonvention für den Keyring-Eintrag des Private Keys: `format!("{server_id}::ssh_key")` - eigener Eintrag getrennt vom Passwort (das weiterhin unter `server_id` selbst liegt), über die bestehenden generischen `keyring_store::store_secret`/`get_secret`-Funktionen (keine Änderung an `keyring_store.rs` nötig, die Funktionen sind bereits generisch über den `account`-String).

- [ ] **Step 1: `connect_with_key_or_password` einführen**

In `src-tauri/src/lib.rs`, direkt vor `connect_fresh` (Zeile 159) einfügen:

```rust
fn ssh_key_keyring_name(server_id: &str) -> String {
    format!("{server_id}::ssh_key")
}

/// Zentraler Verbindungsweg für alle Server: versucht zuerst SSH-Key-Auth (falls ein Key für
/// diesen Server im OS-Keyring liegt), fällt bei Fehlschlag oder fehlendem Key auf Passwort
/// zurück. Nach einem erfolgreichen Passwort-Login wird - falls noch kein Key existiert oder
/// der vorhandene gerade fehlgeschlagen ist (z.B. auf dem Server gelöscht) - automatisch ein
/// (neuer) Key generiert und in `authorized_keys` ausgerollt, damit der nächste Connect wieder
/// per Key läuft. Best-effort: schlägt das Ausrollen fehl, bleibt die Session trotzdem nutzbar.
async fn connect_with_key_or_password(
    host: &str,
    port: u16,
    username: &str,
    server_id: &str,
    known_fingerprint: Option<&str>,
) -> Result<ssh::SshSession, String> {
    let key_name = ssh_key_keyring_name(server_id);
    if let Ok(pem) = keyring_store::get_secret(&key_name) {
        if let Ok(keypair) = ssh_keys::load_keypair(&pem) {
            if let Ok(session) =
                ssh::SshSession::connect_key(host, port, username, Arc::new(keypair), known_fingerprint).await
            {
                return Ok(session);
            }
        }
    }

    let password = keyring_store::get_secret(server_id).map_err(|e| e.to_string())?;
    let mut session = ssh::SshSession::connect_password(host, port, username, &password, known_fingerprint)
        .await
        .map_err(|e| e.to_string())?;

    if let Ok((_, private_pem, public_line)) = ssh_keys::generate_and_format(server_id) {
        if session.exec(&ssh_keys::install_command(&public_line, username)).await.is_ok() {
            let _ = keyring_store::store_secret(&key_name, &private_pem);
        }
    }

    Ok(session)
}
```

- [ ] **Step 2: `connect_fresh` auf den neuen Helfer umstellen**

`connect_fresh` (Zeile 159-184) ersetzen durch:

```rust
async fn connect_fresh(state: &State<'_, AppState>, server_id: &str) -> Result<ssh::SshSession, String> {
    let server = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_server(server_id).map_err(|e| e.to_string())?
    };
    let host = server
        .host
        .as_deref()
        .ok_or_else(|| "Server hat noch keine IP-Adresse - erst in den Server-Einstellungen eintragen.".to_string())?;
    let mut session = connect_with_key_or_password(
        host,
        server.port,
        &server.username,
        server_id,
        server.known_host_fingerprint.as_deref(),
    )
    .await?;
    // First connect after this server was added (pre-pinning support, or a DB migrated from
    // before host-key pinning existed) - pin whatever key we just saw as the trusted baseline.
    if server.known_host_fingerprint.is_none() {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let _ = db.set_host_fingerprint(server_id, &session.host_fingerprint);
    }
    let password = keyring_store::get_secret(server_id).unwrap_or_default();
    if !password.is_empty() {
        provisioning::ensure_passwordless_sudo(&mut session, &server.username, &password)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(session)
}
```

`acquire_session` (Zeile 186-202) bleibt unverändert - ruft weiterhin `connect_fresh` auf.

- [ ] **Step 3: `add_server` (Weg A) auf sofortiges Key-Ausrollen nach dem ersten Login umstellen**

In `src-tauri/src/lib.rs:208-258`, den Block ab `let mut session = ssh::SshSession::connect_password(...)` (Zeile 222) bis `provisioning::bootstrap_server(...)` (Zeile 229-231) unverändert lassen (Erst-Login läuft für Weg A immer per Passwort, das ist der einzige Zeitpunkt, an dem noch keine Server-ID existiert). Direkt danach, vor dem `os_raw`-Block (Zeile 233), ergänzen:

```rust
    let id = uuid::Uuid::new_v4().to_string();

    if let Ok((_, private_pem, public_line)) = ssh_keys::generate_and_format(&id) {
        if session.exec(&ssh_keys::install_command(&public_line, &input.username)).await.is_ok() {
            let _ = keyring_store::store_secret(&ssh_key_keyring_name(&id), &private_pem);
        }
    }
```

Die bestehende Zeile `let id = uuid::Uuid::new_v4().to_string();` (aktuell Zeile 242, direkt vor `keyring_store::store_secret(&id, &input.password)`) entfernen, da `id` jetzt weiter oben erzeugt wird. Der Rest von `add_server` (Passwort-Storage, `ServerRecord`-Konstruktion, `db.insert_server`) bleibt unverändert bis auf `host: input.host` (Zeile 248), das zu `host: Some(input.host)` wird.

- [ ] **Step 4: `enable_firewall` und die `.host`-Stelle bei Zeile 931 an `Option<String>` anpassen**

`enable_firewall` (Zeile 276-304): Destrukturierung `(server.host, server.port, server.username, server.known_host_fingerprint)` (Zeile 280) ersetzen durch:

```rust
        let server = db.get_server(&server_id).map_err(|e| e.to_string())?;
        let host = server
            .host
            .clone()
            .ok_or_else(|| "Server hat noch keine IP-Adresse".to_string())?;
        (host, server.port, server.username, server.known_host_fingerprint)
```

Die Stelle bei Zeile 931 (`db.get_server(&server_id).map_err(|e| e.to_string())?.host`) im jeweiligen Funktionskontext prüfen (`grep -n "931" src-tauri/src/lib.rs` nach Task-3-Änderungen erneut ausführen, da sich Zeilennummern durch vorherige Edits verschoben haben können) und `.ok_or_else(|| "Server hat noch keine IP-Adresse".to_string())?` anhängen, analog zu den anderen Stellen.

- [ ] **Step 5: `Arc` importieren, falls noch nicht vorhanden**

Prüfen, ob `use std::sync::Arc;` bereits in `src-tauri/src/lib.rs` importiert ist (wahrscheinlich ja, da `AppState` bereits `Arc<...>`-Felder wie den SSH-Pool nutzt); falls nicht, ergänzen.

- [ ] **Step 6: Kompilierbarkeit prüfen**

Run: `cd src-tauri && cargo check --no-default-features`
Expected: keine Fehler mehr. Alle `Option<String>`-Stellen aus Task 3 sind jetzt korrekt behandelt.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat: Key-first-Verbindungslogik mit Passwort-Fallback und automatischem Key-Ausrollen"
```

---

### Task 5: `lib.rs` - Neue Commands für Weg B (Key vor VM-Existenz)

**Files:**
- Modify: `src-tauri/src/lib.rs` (zwei neue `#[tauri::command]`-Funktionen, Registrierung im `tauri::generate_handler!`-Makro-Aufruf)

**Interfaces:**
- Consumes: `ssh_keys::generate_and_format`, `ssh_keys::install_command`, `ServerRecord`/`db.insert_server`/`db.set_host` (Task 1/3), `provisioning::bootstrap_server`, `ssh::SshSession::connect_key`.
- Produces (Tauri-Commands, vom Frontend in Task 6 aufgerufen):
  - `prepare_key_only_server(name: String, username: String) -> Result<PendingServer, String>` mit `struct PendingServer { server_id: String, public_key: String }`
  - `finalize_pending_server(state, server_id: String, host: String, port: u16, timezone: Option<String>) -> Result<ServerRecord, String>`

- [ ] **Step 1: `PendingServer`-Typ und `prepare_key_only_server` ergänzen**

Direkt nach `add_server` (nach Zeile 258, dem schließenden `}` der Funktion) einfügen:

```rust
#[derive(Serialize)]
pub struct PendingServer {
    pub server_id: String,
    pub public_key: String,
}

/// Modul A, Weg B: generiert einen Key und legt einen unvollständigen Server-Eintrag (ohne
/// Host) an, bevor die VM beim Provider überhaupt existiert - für Cloud-Server, bei denen der
/// User den Public Key direkt beim Anlegen der VM im Provider-Panel hinterlegt (z.B. Hetzners
/// "SSH-Key hinzufügen"-Feld). Umgeht damit Provider, die beim ersten Passwort-Login eine
/// Passwortänderung erzwingen (siehe ssh_keys.rs/connect_with_key_or_password-Doku).
#[tauri::command]
async fn prepare_key_only_server(state: State<'_, AppState>, name: String, username: String) -> Result<PendingServer, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let (_, private_pem, public_line) = ssh_keys::generate_and_format(&id).map_err(|e| e.to_string())?;
    keyring_store::store_secret(&ssh_key_keyring_name(&id), &private_pem).map_err(|e| e.to_string())?;

    let record = ServerRecord {
        id: id.clone(),
        name,
        host: None,
        port: 22,
        username,
        os_info: None,
        known_host_fingerprint: None,
    };
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.insert_server(&record).map_err(|e| e.to_string())?;

    Ok(PendingServer { server_id: id, public_key: public_line })
}
```

- [ ] **Step 2: `finalize_pending_server` ergänzen**

Direkt danach:

```rust
/// Modul A, Weg B: erste tatsächliche Verbindung, sobald die VM existiert und der User die IP
/// eingetragen hat - verbindet per Key (kein Passwort im Spiel), provisioniert wie beim
/// klassischen Weg A (gameserver-User, Basis-Abhängigkeiten) und trägt Host/Port + OS-Info
/// nach.
#[tauri::command]
async fn finalize_pending_server(
    state: State<'_, AppState>,
    server_id: String,
    host: String,
    port: u16,
    timezone: Option<String>,
) -> Result<ServerRecord, String> {
    let (username, private_pem) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let server = db.get_server(&server_id).map_err(|e| e.to_string())?;
        let pem = keyring_store::get_secret(&ssh_key_keyring_name(&server_id)).map_err(|e| e.to_string())?;
        (server.username, pem)
    };
    let keypair = ssh_keys::load_keypair(&private_pem).map_err(|e| e.to_string())?;

    let mut session = ssh::SshSession::connect_key(&host, port, &username, Arc::new(keypair), None)
        .await
        .map_err(|e| e.to_string())?;
    let host_fingerprint = session.host_fingerprint.clone();

    provisioning::bootstrap_server(&mut session, timezone.as_deref())
        .await
        .map_err(|e| e.to_string())?;

    let os_raw = session
        .exec("grep PRETTY_NAME /etc/os-release | cut -d'\"' -f2")
        .await
        .unwrap_or_default();
    let os_info = {
        let trimmed = os_raw.trim();
        if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
    };

    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.set_host(&server_id, &host, port).map_err(|e| e.to_string())?;
    db.set_host_fingerprint(&server_id, &host_fingerprint).map_err(|e| e.to_string())?;
    if let Some(info) = &os_info {
        db.set_os_info(&server_id, info).map_err(|e| e.to_string())?;
    }
    db.get_server(&server_id).map_err(|e| e.to_string())
}
```

Prüfe, ob `Db::set_os_info(&self, id: &str, os_info: &str) -> rusqlite::Result<()>` bereits existiert (`grep -n "fn set_os_info" src-tauri/src/db.rs`); falls nicht, in `db.rs` neben `set_host_fingerprint` ergänzen:

```rust
    pub fn set_os_info(&self, id: &str, os_info: &str) -> rusqlite::Result<()> {
        self.conn.execute("UPDATE servers SET os_info = ?1 WHERE id = ?2", params![os_info, id])?;
        Ok(())
    }
```

- [ ] **Step 3: Beide Commands im `tauri::generate_handler!`-Aufruf registrieren**

In `src-tauri/src/lib.rs`, im `tauri::generate_handler![...]`-Makro-Aufruf (bei `add_server` suchen, `grep -n "generate_handler" src-tauri/src/lib.rs`), `prepare_key_only_server` und `finalize_pending_server` direkt neben `add_server` ergänzen.

- [ ] **Step 4: Kompilierbarkeit prüfen**

Run: `cd src-tauri && cargo check --no-default-features`
Expected: keine Fehler.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/src/db.rs
git commit -m "feat: prepare_key_only_server/finalize_pending_server für Weg B (Key vor VM-Existenz)"
```

---

### Task 6: Frontend - `AddServerDialog.tsx` Tabs für Weg A/Weg B

**Files:**
- Modify: `src/types.ts` (`host: string` → `host: string | null`)
- Modify: `src/AddServerDialog.tsx` (komplett überarbeitet: Tab-Umschalter)
- Modify: `src/App.tsx:565,622` (Anzeige für `host === null`)

**Interfaces:**
- Consumes: Tauri-Commands `add_server` (bestehend, unverändert), `prepare_key_only_server`, `finalize_pending_server` (Task 5).

- [ ] **Step 1: `ServerRecord.host` im Frontend-Typ optional machen**

In `src/types.ts:4`, `host: string;` ändern zu `host: string | null;`.

- [ ] **Step 2: `AddServerDialog.tsx` um Tab-Umschalter und Weg-B-Flow erweitern**

`src/AddServerDialog.tsx` komplett ersetzen durch:

```tsx
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ServerRecord } from "./types";

type Props = {
  onClose: () => void;
  onCreated: (server: ServerRecord) => void;
};

type Mode = "existing" | "new-cloud";

export default function AddServerDialog({ onClose, onCreated }: Props) {
  const [mode, setMode] = useState<Mode>("new-cloud");

  // Weg A: bestehender Server, Passwort
  const [name, setName] = useState("");
  const [host, setHost] = useState("");
  const [port, setPort] = useState("22");
  const [username, setUsername] = useState("root");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  // Weg B: neuer Cloud-Server, Key zuerst
  const [pendingName, setPendingName] = useState("");
  const [pendingUsername, setPendingUsername] = useState("root");
  const [pendingServerId, setPendingServerId] = useState("");
  const [publicKey, setPublicKey] = useState("");
  const [pendingHost, setPendingHost] = useState("");
  const [pendingPort, setPendingPort] = useState("22");
  const [preparing, setPreparing] = useState(false);
  const [finalizing, setFinalizing] = useState(false);
  const [pendingError, setPendingError] = useState("");

  async function submitExisting(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError("");
    try {
      const server = await invoke<ServerRecord>("add_server", {
        input: {
          name,
          host,
          port: Number(port),
          username,
          password,
          timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
        },
      });
      onCreated(server);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  async function prepareNewCloud(e: React.FormEvent) {
    e.preventDefault();
    setPreparing(true);
    setPendingError("");
    try {
      const result = await invoke<{ server_id: string; public_key: string }>("prepare_key_only_server", {
        name: pendingName,
        username: pendingUsername,
      });
      setPendingServerId(result.server_id);
      setPublicKey(result.public_key);
    } catch (err) {
      setPendingError(String(err));
    } finally {
      setPreparing(false);
    }
  }

  async function finalizeNewCloud(e: React.FormEvent) {
    e.preventDefault();
    setFinalizing(true);
    setPendingError("");
    try {
      const server = await invoke<ServerRecord>("finalize_pending_server", {
        serverId: pendingServerId,
        host: pendingHost,
        port: Number(pendingPort),
        timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
      });
      onCreated(server);
    } catch (err) {
      setPendingError(String(err));
    } finally {
      setFinalizing(false);
    }
  }

  return (
    <div className="nx-modal-overlay" onClick={onClose}>
      <div className="nx-modal" onClick={(e) => e.stopPropagation()}>
        <h2>Server hinzufügen</h2>

        <div className="nx-tab-row">
          <button type="button" className={mode === "new-cloud" ? "nx-tab-active" : "nx-tab"} onClick={() => setMode("new-cloud")}>
            Neuer Cloud-Server (empfohlen)
          </button>
          <button type="button" className={mode === "existing" ? "nx-tab-active" : "nx-tab"} onClick={() => setMode("existing")}>
            Bestehender Server
          </button>
        </div>

        {mode === "existing" && (
          <form onSubmit={submitExisting}>
            <label>
              Name
              <input value={name} onChange={(e) => setName(e.target.value)} required placeholder="z.B. Hetzner-VPS-01" />
            </label>
            <label>
              IP-Adresse / Host
              <input value={host} onChange={(e) => setHost(e.target.value)} required placeholder="88.198.23.45" />
            </label>
            <div className="nx-modal-row">
              <label style={{ flex: 1 }}>
                SSH-Port
                <input value={port} onChange={(e) => setPort(e.target.value)} required />
              </label>
              <label style={{ flex: 2 }}>
                Benutzername
                <input value={username} onChange={(e) => setUsername(e.target.value)} required />
              </label>
            </div>
            <label>
              Passwort
              <input type="password" value={password} onChange={(e) => setPassword(e.target.value)} required />
            </label>
            <p style={{ color: "var(--nx-text-muted)", fontSize: 11, marginTop: -4 }}>
              GrimmNetz richtet automatisch einen Brute-Force-Schutz (fail2ban) ein und rollt danach automatisch einen
              SSH-Key aus, sodass zukünftige Verbindungen ohne Passwort auskommen.
            </p>
            {error && <div className="nx-update-error">{error}</div>}
            {busy && (
              <div style={{ color: "var(--nx-text-muted)", fontSize: 12 }}>
                Verbinde per SSH und richte Server ein (gameserver-User, Abhängigkeiten)…
              </div>
            )}
            <div className="nx-modal-actions">
              <button type="button" onClick={onClose} disabled={busy}>
                Abbrechen
              </button>
              <button type="submit" className="nx-update-btn" disabled={busy}>
                {busy ? "Verbinde…" : "Server hinzufügen"}
              </button>
            </div>
          </form>
        )}

        {mode === "new-cloud" && !publicKey && (
          <form onSubmit={prepareNewCloud}>
            <label>
              Name
              <input value={pendingName} onChange={(e) => setPendingName(e.target.value)} required placeholder="z.B. Hetzner-VPS-01" />
            </label>
            <label>
              Benutzername (beim Provider meist "root")
              <input value={pendingUsername} onChange={(e) => setPendingUsername(e.target.value)} required />
            </label>
            <p style={{ color: "var(--nx-text-muted)", fontSize: 11, marginTop: -4 }}>
              GrimmNetz erzeugt jetzt einen SSH-Key nur für diesen Server. Kein Passwort nötig - umgeht auch
              Anbieter (z.B. Hetzner), die beim ersten Passwort-Login eine Passwortänderung erzwingen.
            </p>
            {pendingError && <div className="nx-update-error">{pendingError}</div>}
            <div className="nx-modal-actions">
              <button type="button" onClick={onClose} disabled={preparing}>
                Abbrechen
              </button>
              <button type="submit" className="nx-update-btn" disabled={preparing}>
                {preparing ? "Erzeuge Key…" : "Key erzeugen"}
              </button>
            </div>
          </form>
        )}

        {mode === "new-cloud" && publicKey && (
          <form onSubmit={finalizeNewCloud}>
            <ol style={{ fontSize: 13, paddingLeft: 18 }}>
              <li>Cloud-Panel deines Anbieters öffnen (z.B. Hetzner Cloud)</li>
              <li>Beim Server-Erstellen diesen Key einfügen:</li>
            </ol>
            <div className="nx-modal-row">
              <textarea readOnly value={publicKey} rows={3} style={{ flex: 1, fontFamily: "monospace", fontSize: 11 }} />
              <button type="button" onClick={() => navigator.clipboard.writeText(publicKey)}>
                Kopieren
              </button>
            </div>
            <label>
              IP-Adresse, sobald die VM läuft
              <input value={pendingHost} onChange={(e) => setPendingHost(e.target.value)} required placeholder="88.198.23.45" />
            </label>
            <label>
              SSH-Port
              <input value={pendingPort} onChange={(e) => setPendingPort(e.target.value)} required />
            </label>
            {pendingError && <div className="nx-update-error">{pendingError}</div>}
            {finalizing && (
              <div style={{ color: "var(--nx-text-muted)", fontSize: 12 }}>
                Verbinde per Key und richte Server ein…
              </div>
            )}
            <div className="nx-modal-actions">
              <button type="button" onClick={onClose} disabled={finalizing}>
                Später fertigstellen
              </button>
              <button type="submit" className="nx-update-btn" disabled={finalizing}>
                {finalizing ? "Verbinde…" : "Verbinden"}
              </button>
            </div>
          </form>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 3: `.nx-tab-row`/`.nx-tab`/`.nx-tab-active`-Styles ergänzen, falls nicht vorhanden**

Prüfen (`grep -n "nx-tab" src/*.css`), ob diese Klassen bereits existieren (die App nutzt an anderer Stelle evtl. schon Tabs, z.B. im Instanz-Detail). Falls nicht, im Haupt-Stylesheet (dort wo `.nx-modal`/`.nx-modal-actions` definiert sind) ergänzen, im bestehenden Türkis/Anthrazit-Farbschema (`var(--nx-accent)` für den aktiven Tab, siehe `feedback_no_default_buttons`-Konvention: kein unstyled Standard-Button).

- [ ] **Step 4: `App.tsx` für `host === null` (Weg-B-Server ohne IP) absichern**

`src/App.tsx:565` (`<div className="nx-server-ip">{server.host}</div>`) ändern zu:

```tsx
<div className="nx-server-ip">{server.host ?? "Warte auf IP…"}</div>
```

`src/App.tsx:622` (`{selectedServer.host}:{selectedServer.port}`) ändern zu:

```tsx
{selectedServer.host ?? "Warte auf IP…"}:{selectedServer.port}
```

- [ ] **Step 5: Frontend-Typcheck**

Run: `npm run build` (oder `tsc --noEmit`, je nachdem was `package.json` als Skript bereitstellt - `grep -n "\"build\"\|\"typecheck\"" package.json` prüfen)
Expected: keine TypeScript-Fehler.

- [ ] **Step 6: Commit**

```bash
git add src/types.ts src/AddServerDialog.tsx src/App.tsx
git commit -m "feat(ui): Tab-Umschalter Bestehender-Server/Neuer-Cloud-Server im Server-hinzufügen-Dialog"
```

---

### Task 7: Live-Verifikation (WSL + Hetzner)

**Files:** keine Code-Änderungen, reines Testing (dieses Projekt hat keine automatisierten Tests, siehe Global Constraints).

- [ ] **Step 1: App neu bauen und starten**

```bash
cd src-tauri && cargo clean -p grimmnetz
```

Dann `npm run tauri dev` im Worktree starten (mit den üblichen `OPENSSL_*`-Env-Vars).

- [ ] **Step 2: Weg A live testen (Bestandsserver-Upgrade)**

Einen bereits laufenden Test-Server (Passwort-basiert, z.B. der bestehende Hetzner-Testserver) über "Server bearbeiten" neu verbinden lassen oder einen neuen Server klassisch per Passwort hinzufügen. Danach per SSH auf dem Server selbst prüfen:

```bash
cat ~/.ssh/authorized_keys
```

Erwartet: eine neue Zeile `ssh-ed25519 ... grimmnetz-<server-id>`. Anschließend die GrimmNetz-App neu starten und eine beliebige Aktion auf dem Server ausführen (z.B. Instanz-Liste laden) - die Verbindung muss weiterhin funktionieren, jetzt per Key (kein sichtbarer Unterschied für den User, aber intern kein Passwort-Handshake mehr nötig).

- [ ] **Step 3: Weg B live gegen einen frischen Hetzner-Server testen**

"Server hinzufügen" → Tab "Neuer Cloud-Server" → Key erzeugen lassen → angezeigten Public Key kopieren → beim Anlegen einer neuen Hetzner-Cloud-VM ins SSH-Key-Feld einfügen → nach VM-Start die IP in GrimmNetz eintragen → "Verbinden". Erwartet: Verbindung gelingt sofort per Key, **kein** erzwungener Passwort-Change-Dialog (das ursprüngliche Problem tritt gar nicht erst auf, da nie Passwort-Auth verwendet wird).

- [ ] **Step 4: Regressionscheck bestehender Passwort-Flows**

`update_server` (Passwort ändern) und `enable_firewall` weiterhin über die bestehenden manuellen Testpfade prüfen (Passwort-Feld in "Server bearbeiten" leer lassen → alter Wert bleibt; neues Passwort eintragen → wird übernommen). Diese beiden Flows nutzen bewusst weiterhin ausschließlich Passwort-Auth mit explizit übergebenen Zugangsdaten (siehe Spec, Abschnitt "Fehlerfälle" - Key-Rollout betrifft nur den regulären Verbindungsweg, nicht das explizite Credential-Update).

- [ ] **Step 5: README/Roadmap/Patch-Notes aktualisieren, Version bumpen, committen**

Analog zum Vorgehen beim Docker-Migration-Release: `README.md` Security-Abschnitt um SSH-Key-Auth ergänzen, `src/patchNotes.ts` neuen Eintrag, Version in `package.json`/`src-tauri/tauri.conf.json`/`src-tauri/Cargo.toml` bumpen, `cargo check` zur Aktualisierung von `Cargo.lock`, dann committen.
