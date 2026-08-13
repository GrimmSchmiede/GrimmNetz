mod db;
mod games;
mod keyring_store;
mod mc_ping;
mod provisioning;
mod ssh;
mod ssh_keys;

use db::{Db, InstanceRecord, ServerRecord};
use games::GameTemplate;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::ipc::Channel;
use tauri::{Manager, State};
use tokio::sync::{Mutex as TokioMutex, OwnedMutexGuard};

type SessionSlot = Arc<TokioMutex<Option<ssh::SshSession>>>;

pub struct AppState {
    pub db: Mutex<Db>,
    /// One reused SSH connection per server, instead of a fresh handshake + auth for every
    /// single poll (CPU/RAM every 8s, instance status every 5s) - that reconnect overhead is
    /// what made the UI feel laggy under frequent polling.
    pub ssh_pool: Mutex<HashMap<String, SessionSlot>>,
    /// Kept alive across polls (not re-created per call) so sysinfo's CPU/network deltas are
    /// measured against the previous poll instead of needing an artificial sleep every time.
    pub local_sys: Mutex<LocalSystemMonitor>,
    /// If true, closing the window hides it to the system tray instead of quitting - only the
    /// tray menu's "Beenden" (or the frontend explicitly asking to quit) actually exits.
    pub close_to_tray: Mutex<bool>,
    pub settings_path: std::path::PathBuf,
}

pub struct LocalSystemMonitor {
    pub sys: sysinfo::System,
    pub networks: sysinfo::Networks,
    pub last_net_bytes: (u64, u64), // (received, transmitted)
    pub last_poll: std::time::Instant,
}

impl LocalSystemMonitor {
    fn new() -> Self {
        Self {
            sys: sysinfo::System::new_all(),
            networks: sysinfo::Networks::new_with_refreshed_list(),
            last_net_bytes: (0, 0),
            last_poll: std::time::Instant::now(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct HardwareStats {
    pub cpu_percent: f32,
    pub ram_used_mb: u64,
    pub ram_total_mb: u64,
    pub disk_used_gb: u64,
    pub disk_total_gb: u64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct LocalSystemStats {
    pub cpu_percent: f32,
    pub ram_used_mb: u64,
    pub ram_total_mb: u64,
    pub net_up_kbps: f64,
    pub net_down_kbps: f64,
}

/// Reads CPU/RAM/network usage of the machine GrimmNetz itself is running on
/// (not a managed server) for the sidebar's local System Status widget.
#[tauri::command]
fn get_local_system_stats(state: State<'_, AppState>) -> Result<LocalSystemStats, String> {
    let mut monitor = state.local_sys.lock().map_err(|e| e.to_string())?;

    monitor.sys.refresh_cpu_usage();
    monitor.sys.refresh_memory();
    monitor.networks.refresh();

    let cpu_percent = monitor.sys.global_cpu_usage();
    let ram_used_mb = monitor.sys.used_memory() / 1024 / 1024;
    let ram_total_mb = monitor.sys.total_memory() / 1024 / 1024;

    let (received, transmitted) = monitor
        .networks
        .values()
        .fold((0u64, 0u64), |(r, t), n| (r + n.total_received(), t + n.total_transmitted()));

    let elapsed = monitor.last_poll.elapsed().as_secs_f64().max(0.001);
    let (last_r, last_t) = monitor.last_net_bytes;
    let net_down_kbps = if last_r > 0 { (received.saturating_sub(last_r)) as f64 / 1024.0 / elapsed } else { 0.0 };
    let net_up_kbps = if last_t > 0 { (transmitted.saturating_sub(last_t)) as f64 / 1024.0 / elapsed } else { 0.0 };

    monitor.last_net_bytes = (received, transmitted);
    monitor.last_poll = std::time::Instant::now();

    Ok(LocalSystemStats { cpu_percent, ram_used_mb, ram_total_mb, net_up_kbps, net_down_kbps })
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum LogEvent {
    Line { text: String },
    Closed,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum InstallEvent {
    Step { label: String },
    Progress { percent: f32, phase: String },
}

/// Pulls the percentage and phase out of steamcmd's redrawn-in-place status line, e.g.
/// `Update state (0x61) downloading, progress: 42.30 (123456789 / 291234567)`. steamcmd
/// runs a download pass and then a separate verify pass, each counting 0-100% on its own -
/// without the phase name that looks to the user like the whole install restarted at 0%.
fn parse_steamcmd_progress(line: &str) -> Option<(f32, String)> {
    let after = line.split("progress:").nth(1)?;
    let number = after.trim().split(|c: char| c == ' ' || c == '%').next()?;
    let percent = number.trim().parse::<f32>().ok()?;
    let phase = if line.contains("verifying") {
        "Überprüfung"
    } else if line.contains("downloading") {
        "Download"
    } else {
        "Installation"
    };
    Some((percent, phase.to_string()))
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum UploadEvent {
    Progress {
        #[serde(rename = "bytesSent")]
        bytes_sent: u64,
        #[serde(rename = "totalBytes")]
        total_bytes: u64,
    },
}

#[derive(Deserialize)]
pub struct AddServerInput {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    /// IANA timezone (e.g. "Europe/Berlin") of the machine running the app - passed through
    /// so the server's clock matches, which backup timestamps and log timelines depend on.
    #[serde(default)]
    pub timezone: Option<String>,
}

fn ssh_key_keyring_name(server_id: &str) -> String {
    format!("{server_id}::ssh_key")
}

/// Erzeugt ein internes Zufallspasswort für die automatische Rotation. Ein v4-UUID-String wäre
/// nur Kleinbuchstaben/Ziffern/Bindestriche und scheitert damit an `pam_pwquality`-Regeln, die
/// mehrere Zeichenklassen verlangen. Deshalb: 24 Zeichen aus Groß-/Kleinbuchstaben, Ziffern und
/// wenigen Symbolen (bewusst ohne `'`, `` ` ``, `$`, `\` und `"`, da das Passwort an anderer
/// Stelle in Shell-Kommandos interpoliert wird), mit garantiert je einem Zeichen pro Klasse.
fn generate_random_password() -> String {
    use rand::seq::SliceRandom;
    use rand::Rng;

    const UPPER: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";
    const LOWER: &[u8] = b"abcdefghijkmnpqrstuvwxyz";
    const DIGIT: &[u8] = b"23456789";
    const SYMBOL: &[u8] = b"!@#%^&*-_=+";

    let mut rng = rand::thread_rng();
    let pool: Vec<u8> = [UPPER, LOWER, DIGIT, SYMBOL].concat();
    let mut chars: Vec<u8> = [UPPER, LOWER, DIGIT, SYMBOL]
        .iter()
        .map(|class| class[rng.gen_range(0..class.len())])
        .collect();
    while chars.len() < 24 {
        chars.push(pool[rng.gen_range(0..pool.len())]);
    }
    chars.shuffle(&mut rng);
    String::from_utf8(chars).expect("ASCII-Pool")
}

/// Cloud-Provider (u.a. Hetzner) markieren ein frisch vergebenes/zurückgesetztes root-Passwort
/// als sofort abgelaufen - sshd/PAM verweigert dann JEDEN nicht-interaktiven Befehl mit
/// "Password change required but no TTY available", nicht nur `sudo` (siehe
/// `ssh::SshSession::change_expired_password`-Doku). Prüft das per günstigem Test-Exec direkt
/// nach dem Passwort-Login und spielt bei Bedarf automatisiert den erzwungenen Passwortwechsel
/// durch - mit einem intern generierten, dem User nie gezeigten Zufallspasswort, da GrimmNetz
/// ab dem nächsten Connect ohnehin auf Key-Auth wechselt. Gibt bei einer Rotation die neue
/// Session UND das neue Passwort zurück, damit der Aufrufer es im Keyring aktualisieren kann.
/// Das rotierte Passwort wird SOFORT nach dem erfolgreichen Wechsel unter `server_id` im Keyring
/// abgelegt - noch vor dem Reconnect. Sonst wäre der Server nach einem Fehlschlag eines der
/// Folgeschritte dauerhaft unerreichbar: remote gilt das neue Passwort, gespeichert wäre das alte.
/// Schlägt dieses Speichern fehl, ist das ein harter Fehler (die einzige Kopie eines lebenden
/// Credentials), kein still verschluckter Best-effort-Versuch.
async fn ensure_password_current(
    mut session: ssh::SshSession,
    host: &str,
    port: u16,
    username: &str,
    password: &str,
    known_fingerprint: Option<&str>,
    server_id: Option<&str>,
) -> Result<(ssh::SshSession, Option<String>), String> {
    // sshd/PAM verweigert bei einem abgelaufenen Passwort nicht zwingend mit einem SSH-Protokoll-
    // Fehler - die Warnung kommt oft als ganz normale (Extended-Data-)Ausgabe des Befehls selbst
    // zurück, `exec` liefert also trotzdem `Ok(...)`. Deshalb den Text sowohl im Erfolgs- als
    // auch im Fehlerfall prüfen, statt uns auf `Err` allein zu verlassen.
    let needs_change = |text: &str| text.to_lowercase().contains("password change required");
    let probe = session.exec("true").await;
    let is_expired = match &probe {
        Ok(out) => needs_change(out),
        Err(e) => needs_change(&e.to_string()),
    };
    if !is_expired {
        return probe.map(|_| (session, None)).map_err(|e| e.to_string());
    }

    let new_password = generate_random_password();
    session
        .change_expired_password(password, &new_password)
        .await
        .map_err(|e| e.to_string())?;
    // Ab hier gilt remote NUR noch das neue Passwort - sofort persistieren, bevor irgendein
    // weiterer (potenziell scheiternder) Schritt folgt.
    if let Some(id) = server_id {
        keyring_store::store_secret(id, &new_password).map_err(|e| {
            format!(
                "Das Passwort wurde auf dem Server geändert, konnte aber nicht im Keyring gespeichert werden: {e}"
            )
        })?;
    }
    // Der Server trennt die Verbindung nach einem erzwungenen Passwortwechsel ohnehin -
    // frischer Connect mit dem neuen Passwort statt die alte Session weiterzuverwenden.
    drop(session);
    let fresh = ssh::SshSession::connect_password(host, port, username, &new_password, known_fingerprint)
        .await
        .map_err(|e| e.to_string())?;
    Ok((fresh, Some(new_password)))
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
    let mut key_error: Option<String> = None;
    if let Ok(pem) = keyring_store::get_secret(&key_name) {
        if let Ok(keypair) = ssh_keys::load_keypair(&pem) {
            match ssh::SshSession::connect_key(host, port, username, Arc::new(keypair), known_fingerprint).await {
                Ok(session) => return Ok(session),
                // Fehler merken: Gibt es für diesen Server gar kein Passwort (reiner Weg-B-Server),
                // ist dieser Grund für den User die eigentlich hilfreiche Information - nicht das
                // "Keyring-Eintrag nicht gefunden" der Passwortsuche unten.
                Err(e) => key_error = Some(e.to_string()),
            }
        }
    }

    let password = match keyring_store::get_secret(server_id) {
        Ok(p) => p,
        Err(e) => return Err(key_error.unwrap_or_else(|| e.to_string())),
    };
    let session = ssh::SshSession::connect_password(host, port, username, &password, known_fingerprint)
        .await
        .map_err(|e| e.to_string())?;
    // Ein rotiertes Passwort hat `ensure_password_current` bereits selbst im Keyring abgelegt.
    let (mut session, _rotated_password) =
        ensure_password_current(session, host, port, username, &password, known_fingerprint, Some(server_id)).await?;

    if let Ok((_, private_pem, public_line)) = ssh_keys::generate_and_format(server_id) {
        // `exec` liefert keinen Exit-Code des Remote-Befehls, deshalb prüfen wir den Marker in
        // der Ausgabe - sonst würde ein fehlgeschlagenes Ausrollen (z.B. volle Platte) als Erfolg
        // gelten und wir hinterlegten einen Key, der auf dem Server gar nicht existiert.
        if let Ok(out) = session.exec(&ssh_keys::install_command(&public_line)).await {
            if out.contains(ssh_keys::INSTALL_SUCCESS_MARKER) {
                let _ = keyring_store::store_secret(&key_name, &private_pem);
            }
        }
    }

    Ok(session)
}

/// Establishes a brand-new authenticated SSH session for a stored server, looking up
/// host/port/username from the DB and the password from the OS keyring, and makes sure
/// passwordless sudo is set up (self-healing for servers added before that existed).
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

/// Returns the server's pooled SSH session, connecting (and running the one-time sudo
/// setup) only if there isn't already a live one. The caller gets exclusive access via the
/// returned guard for as long as they hold it; on any exec failure they should set the slot
/// back to `None` so the next call reconnects instead of reusing a dead connection.
async fn acquire_session(state: &State<'_, AppState>, server_id: &str) -> Result<OwnedMutexGuard<Option<ssh::SshSession>>, String> {
    let slot = {
        let mut pool = state.ssh_pool.lock().map_err(|e| e.to_string())?;
        pool.entry(server_id.to_string())
            .or_insert_with(|| Arc::new(TokioMutex::new(None)))
            .clone()
    };
    let mut guard = slot.lock_owned().await;
    if guard.is_none() {
        *guard = Some(connect_fresh(state, server_id).await?);
    }
    Ok(guard)
}

/// Module A: registers a new server. Connects via SSH, provisions the isolated
/// `gameserver` user + base deps, then persists metadata (SQLite) and the
/// password in the OS keyring — never in plaintext.
#[tauri::command]
async fn add_server(state: State<'_, AppState>, input: AddServerInput) -> Result<ServerRecord, String> {
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        if let Some(existing) = db
            .find_server_by_host_port(&input.host, input.port)
            .map_err(|e| e.to_string())?
        {
            return Err(format!(
                "{}:{} ist bereits als \"{}\" eingetragen - Server können nicht doppelt hinzugefügt werden.",
                input.host, input.port, existing.name
            ));
        }
    }

    // Die Server-ID wird schon hier erzeugt (statt erst nach dem Key-Rollout), damit ein
    // automatisch rotiertes Passwort sofort unter dem endgültigen Keyring-Schlüssel landen kann.
    let id = uuid::Uuid::new_v4().to_string();

    let session = ssh::SshSession::connect_password(&input.host, input.port, &input.username, &input.password, None)
        .await
        .map_err(|e| e.to_string())?;
    let host_fingerprint = session.host_fingerprint.clone();
    let (mut session, rotated_password) =
        ensure_password_current(session, &input.host, input.port, &input.username, &input.password, Some(&host_fingerprint), Some(&id)).await?;
    let effective_password = rotated_password.unwrap_or_else(|| input.password.clone());
    provisioning::ensure_passwordless_sudo(&mut session, &input.username, &effective_password)
        .await
        .map_err(|e| e.to_string())?;
    provisioning::bootstrap_server(&mut session, input.timezone.as_deref())
        .await
        .map_err(|e| e.to_string())?;

    if let Ok((_, private_pem, public_line)) = ssh_keys::generate_and_format(&id) {
        // Marker statt `is_ok()` - siehe `connect_with_key_or_password`.
        if let Ok(out) = session.exec(&ssh_keys::install_command(&public_line)).await {
            if out.contains(ssh_keys::INSTALL_SUCCESS_MARKER) {
                let _ = keyring_store::store_secret(&ssh_key_keyring_name(&id), &private_pem);
            }
        }
    }

    let os_raw = session
        .exec("grep PRETTY_NAME /etc/os-release | cut -d'\"' -f2")
        .await
        .unwrap_or_default();
    let os_info = {
        let trimmed = os_raw.trim();
        if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
    };

    keyring_store::store_secret(&id, &effective_password).map_err(|e| e.to_string())?;

    let record = ServerRecord {
        id,
        name: input.name,
        host: Some(input.host),
        port: input.port,
        username: input.username,
        os_info,
        known_host_fingerprint: Some(host_fingerprint),
    };

    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.insert_server(&record).map_err(|e| e.to_string())?;
    Ok(record)
}

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
        // Gleicher Doppel-Eintrag-Schutz wie in `add_server` (Weg A): zwei Einträge auf dieselbe
        // Host+Port-Kombination führen später z.B. in `discover_instances` zu einer Kollision mit
        // dem Unique-Constraint der DB.
        if let Some(existing) = db.find_server_by_host_port(&host, port).map_err(|e| e.to_string())? {
            if existing.id != server_id {
                return Err(format!(
                    "{}:{} ist bereits als \"{}\" eingetragen - Server können nicht doppelt hinzugefügt werden.",
                    host, port, existing.name
                ));
            }
        }
        let server = db.get_server(&server_id).map_err(|e| e.to_string())?;
        let pem = keyring_store::get_secret(&ssh_key_keyring_name(&server_id)).map_err(|e| e.to_string())?;
        (server.username, pem)
    };

    // Für Weg B liegt kein Passwort vor, mit dem `ensure_passwordless_sudo` den einmaligen
    // sudo-Prompt beantworten könnte. Als `root` ist das egal (dessen `sudo -n` läuft ohne
    // Passwort, der Fast-Path in `ensure_passwordless_sudo` greift), für jeden anderen Login-User
    // würde dagegen jedes `sudo` in `bootstrap_server` still fehlschlagen - deshalb hier ein
    // klarer Guard statt eines halb provisionierten Servers.
    if username != "root" {
        return Err(format!(
            "Für einen reinen Key-Server (neuer Cloud-Server) wird aktuell der Benutzername \"root\" benötigt - \
             \"{username}\" funktioniert nicht, weil ohne Passwort kein passwortloses sudo eingerichtet werden kann. \
             Lege die VM mit dem Standard-Benutzer \"root\" an oder füge den Server über den klassischen Weg mit Passwort hinzu."
        ));
    }

    let keypair = ssh_keys::load_keypair(&private_pem).map_err(|e| e.to_string())?;

    let mut session = ssh::SshSession::connect_key(&host, port, &username, Arc::new(keypair), None)
        .await
        .map_err(|e| e.to_string())?;
    let host_fingerprint = session.host_fingerprint.clone();

    // Konsistent mit `add_server`/`connect_fresh`: prüft (und verifiziert) passwortloses sudo.
    // Der leere String genügt hier, weil für root der Check `sudo -n true` bereits erfolgreich
    // ist und die Funktion sofort zurückkehrt, ohne das Passwort zu benutzen.
    provisioning::ensure_passwordless_sudo(&mut session, &username, "")
        .await
        .map_err(|e| e.to_string())?;

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

/// Module A: checks whether a supported firewall is currently active on the server - called
/// right after a server is added so the UI can prompt "your server is unprotected, want to
/// turn a firewall on?" for the common case of a brand-new VPS with no local firewall at all.
#[tauri::command]
async fn check_firewall_active(state: State<'_, AppState>, server_id: String) -> Result<bool, String> {
    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();
    let family = provisioning::detect_distro_family(session).await.map_err(|e| e.to_string())?;
    provisioning::is_firewall_active(session, family).await.map_err(|e| e.to_string())
}

/// Module A: turns the firewall on for a server that doesn't have one active yet, with a
/// 2-minute auto-rollback safety net - see `provisioning::enable_firewall_with_rollback`. Only
/// ever called after explicit user confirmation in the UI (this is the single most consequential
/// automated action in the app: a mistake can lock the user out of their own server).
#[tauri::command]
async fn enable_firewall(state: State<'_, AppState>, server_id: String) -> Result<(), String> {
    let (host, port, username, known_fingerprint) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let server = db.get_server(&server_id).map_err(|e| e.to_string())?;
        let host = server
            .host
            .ok_or_else(|| "Server hat noch keine IP-Adresse".to_string())?;
        (host, server.port, server.username, server.known_host_fingerprint)
    };
    let job_id = {
        let mut guard = acquire_session(&state, &server_id).await?;
        let session = guard.as_mut().unwrap();
        let family = provisioning::detect_distro_family(session).await.map_err(|e| e.to_string())?;
        provisioning::enable_firewall_with_rollback(session, family, port)
            .await
            .map_err(|e| e.to_string())?
    };

    // Verify with a brand-new connection (never the pooled one, which might just be reusing
    // an already-open TCP socket that predates the firewall change) that we're not locked out.
    // Über den zentralen Weg, damit das auch für reine Key-Server (Weg B, kein Passwort im
    // Keyring) funktioniert.
    match connect_with_key_or_password(&host, port, &username, &server_id, known_fingerprint.as_deref()).await {
        Ok(mut fresh) => {
            let _ = provisioning::cancel_firewall_rollback(&mut fresh, &job_id).await;
            Ok(())
        }
        Err(e) => Err(format!(
            "Verbindungstest nach dem Aktivieren der Firewall fehlgeschlagen ({e}). Zu deiner Sicherheit wird die Firewall automatisch in ca. 2 Minuten wieder deaktiviert."
        )),
    }
}

/// Module A: updates a server's connection details (name/host/port/username, optionally a
/// new password) without re-provisioning anything on the server itself - for when the
/// provider changes the IP, or the user wants to fix a typo, without starting over. Verifies
/// the new details actually connect before persisting them, and drops the pooled SSH session
/// (if any) so the next command reconnects fresh with the updated details.
#[tauri::command]
async fn update_server(
    state: State<'_, AppState>,
    server_id: String,
    name: String,
    host: String,
    port: u16,
    username: String,
    password: Option<String>,
) -> Result<ServerRecord, String> {
    // Deliberately not checking the pinned fingerprint here - editing and re-saving a server's
    // details is the app's explicit "I trust this host key" action (e.g. after a legitimate
    // server reinstall changed it), same as re-adding it from scratch would.
    //
    // Key-Auth hat Vorrang: Ein reiner Weg-B-Server hat gar kein Passwort im Keyring, wäre also
    // über den Passwort-Pfad nie editierbar (und hätte damit auch keinen Weg, einen legitim
    // geänderten Host-Key wieder zu bestätigen).
    let existing_keypair = keyring_store::get_secret(&ssh_key_keyring_name(&server_id))
        .ok()
        .and_then(|pem| ssh_keys::load_keypair(&pem).ok());

    let session = match existing_keypair {
        Some(keypair) => ssh::SshSession::connect_key(&host, port, &username, Arc::new(keypair), None)
            .await
            .map_err(|e| format!("Verbindung mit den neuen Zugangsdaten fehlgeschlagen: {e}"))?,
        None => {
            let effective_password = match &password {
                Some(p) if !p.is_empty() => p.clone(),
                _ => keyring_store::get_secret(&server_id).map_err(|e| e.to_string())?,
            };
            ssh::SshSession::connect_password(&host, port, &username, &effective_password, None)
                .await
                .map_err(|e| format!("Verbindung mit den neuen Zugangsdaten fehlgeschlagen: {e}"))?
        }
    };

    if let Some(p) = &password {
        if !p.is_empty() {
            keyring_store::store_secret(&server_id, p).map_err(|e| e.to_string())?;
        }
    }

    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.update_server(&server_id, &name, &host, port, &username)
            .map_err(|e| e.to_string())?;
        db.set_host_fingerprint(&server_id, &session.host_fingerprint)
            .map_err(|e| e.to_string())?;
    }

    // Force a reconnect on the next command instead of reusing a session that was opened
    // against the old host/port/credentials.
    {
        let mut pool = state.ssh_pool.lock().map_err(|e| e.to_string())?;
        pool.remove(&server_id);
    }

    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_server(&server_id).map_err(|e| e.to_string())
}

/// Reboots the underlying VPS/root server. Destructive/disruptive - the frontend
/// must confirm with the user before calling this.
#[tauri::command]
async fn reboot_server(state: State<'_, AppState>, server_id: String) -> Result<(), String> {
    let mut guard = acquire_session(&state, &server_id).await?;
    // The connection drops as the machine reboots - that's expected, not an error.
    let _ = guard.as_mut().unwrap().exec("sudo reboot").await;
    *guard = None;
    Ok(())
}

/// Module A: lists all registered servers from the local encrypted database.
#[tauri::command]
fn list_servers(state: State<'_, AppState>) -> Result<Vec<ServerRecord>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.list_servers().map_err(|e| e.to_string())
}

/// Removes a server: deletes its DB row and its stored keyring secret.
#[tauri::command]
fn delete_server(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.delete_server(&id).map_err(|e| e.to_string())?;
    let _ = keyring_store::delete_secret(&id);
    // Best-effort: Server ohne je erzeugten Key haben hier keinen Eintrag - das Löschen des
    // Servers darf daran nicht scheitern.
    let _ = keyring_store::delete_secret(&ssh_key_keyring_name(&id));
    if let Ok(mut pool) = state.ssh_pool.lock() {
        pool.remove(&id);
    }
    Ok(())
}

/// Module A: live CPU/RAM polling via a lightweight remote one-liner.
#[tauri::command]
async fn get_hardware_stats(state: State<'_, AppState>, server_id: String) -> Result<HardwareStats, String> {
    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();

    let result = async {
        let cpu_raw = session.exec("top -bn1 | grep 'Cpu(s)' | awk '{print $2}'").await?;
        let mem_raw = session.exec("free -m | awk '/Mem:/ {print $3\" \"$2}'").await?;
        let disk_raw = session.exec("df -BG / | awk 'NR==2 {gsub(\"G\",\"\"); print $3\" \"$2}'").await?;
        anyhow::Ok((cpu_raw, mem_raw, disk_raw))
    }
    .await;

    let (cpu_raw, mem_raw, disk_raw) = match result {
        Ok(v) => v,
        Err(e) => {
            *guard = None;
            return Err(e.to_string());
        }
    };

    let cpu_percent = cpu_raw.trim().parse::<f32>().unwrap_or(0.0);
    let mut mem_parts = mem_raw.trim().split_whitespace();
    let ram_used_mb = mem_parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let ram_total_mb = mem_parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut disk_parts = disk_raw.trim().split_whitespace();
    let disk_used_gb = disk_parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let disk_total_gb = disk_parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);

    Ok(HardwareStats { cpu_percent, ram_used_mb, ram_total_mb, disk_used_gb, disk_total_gb })
}

/// Module B: lists the games available in the bundled template database.
#[tauri::command]
fn list_games() -> Vec<GameTemplate> {
    games::load_templates()
}

/// Module B: lists installed gameserver instances for a server.
#[tauri::command]
fn list_instances(state: State<'_, AppState>, server_id: String) -> Result<Vec<InstanceRecord>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.list_instances(&server_id).map_err(|e| e.to_string())
}

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

    // Directory starts root-owned (default for `mkdir` run as root) and STAYS root-owned until
    // the script itself hands it to `gameserver` (see render_install_script) - handing it over
    // here instead would leave a window where a process already running as `gameserver` (every
    // game instance shares that one unprivileged account) could race-replace install.sh before
    // it's ever executed as root. Directory ownership, not file permissions, is what actually
    // controls who can unlink/replace a file, so this has to be a directory-level guarantee.
    session.exec(&format!("sudo mkdir -p {install_path}")).await.map_err(|e| e.to_string())?;

    // Recorded so list_active_installs can report which game an in-progress/orphaned install
    // belongs to - nothing else persists game_id anywhere before GRIMMNETZ_DONE writes the
    // actual game record, and a reattaching app/second PC needs it to find the right template.
    session
        .exec(&format!(
            "echo {} | sudo tee {install_path}/.grimmnetz-game-id > /dev/null",
            games::shell_single_quote(&game_id)
        ))
        .await
        .map_err(|e| e.to_string())?;

    // discover_instances reads this manifest to recover full instance metadata for anything it
    // finds that the local DB doesn't know about yet - without it, discovery falls back to a
    // binary-signature guess that only works for templates it happens to have a check for, and
    // never recovers the real display name (falls back to the instance UUID) or resource limits.
    let manifest = format!(
        "{{\"game_id\":\"{}\",\"display_name\":\"{}\",\"cpu_limit_percent\":{},\"ram_limit_mb\":{}}}",
        game_id.replace('"', ""),
        template.name.replace('"', ""),
        template.default_cpu_limit_percent,
        template.default_ram_limit_mb
    );
    session
        .exec(&format!(
            "echo {} | sudo tee {install_path}/.grimmnetz-instance.json > /dev/null",
            games::shell_single_quote(&manifest)
        ))
        .await
        .map_err(|e| e.to_string())?;

    let script = if template.install.install_type == "docker" {
        let uid_out = session.exec("id -u gameserver").await.map_err(|e| e.to_string())?;
        let gid_out = session.exec("id -g gameserver").await.map_err(|e| e.to_string())?;
        let gameserver_uid: u32 = uid_out.trim().parse().map_err(|_| "Konnte gameserver-UID nicht ermitteln".to_string())?;
        let gameserver_gid: u32 = gid_out.trim().parse().map_err(|_| "Konnte gameserver-GID nicht ermitteln".to_string())?;
        provisioning::render_docker_install_script(&instance_id, &install_path, &unit_name, &template, gameserver_uid, gameserver_gid)
    } else {
        provisioning::render_install_script(&instance_id, &install_path, &unit_name, &template)
    };
    let escaped_script = script.replace('\'', "'\\''");
    session
        .exec(&format!(
            "echo '{escaped_script}' | sudo tee {install_path}/install.sh > /dev/null && sudo chmod 700 {install_path}/install.sh"
        ))
        .await
        .map_err(|e| e.to_string())?;

    // The install unit's own stdout/stderr (captured by systemd/journald AND redirected into
    // install.log) is what attach_install_stream tails - RemainAfterExit keeps `systemctl
    // is-active` truthful about "still running" for list_active_installs after the script exits.
    // Runs as root (no User= override): the script needs `sudo`-free root access to write the
    // game's systemd unit and manage the firewall, and drops to `gameserver` itself per-step via
    // `sudo -u gameserver` for the actual install commands (render_install_script, Task 1) -
    // gameserver has no sudo rights of its own, so running the whole unit as that user instead
    // makes every privileged step in the script fail.
    let install_unit_contents = format!(
        "[Unit]\nDescription=GrimmNetz Install {instance_id}\n\n\
         [Service]\nType=oneshot\nRemainAfterExit=yes\nWorkingDirectory={install_path}\n\
         ExecStart=/bin/bash -c '/bin/bash {install_path}/install.sh > {install_path}/install.log 2>&1'\n\n\
         [Install]\nWantedBy=multi-user.target\n"
    );
    provisioning::install_systemd_unit(session, &install_unit_name, &install_unit_contents)
        .await
        .map_err(|e| e.to_string())?;
    // `--no-block`: without it, `systemctl start` on a Type=oneshot unit waits for ExecStart to
    // finish before returning - i.e. for the ENTIRE install to complete - which blows straight
    // through the SSH exec timeout (20s) on any real install. `--no-block` just queues the start
    // job and returns immediately, which is exactly what "kick off and observe separately" needs.
    session
        .exec(&format!("sudo systemctl start --no-block {install_unit_name}"))
        .await
        .map_err(|e| e.to_string())?;

    Ok(instance_id)
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ActiveInstall {
    pub instance_id: String,
    pub game_id: String,
    pub running: bool,
}

/// Server-side discovery of installs that are STILL running, independent of the local DB - this
/// is what lets a second PC (or the same PC after being closed) watch an in-progress install it
/// never started itself. An install that has already finished doesn't need this: its game unit
/// (`grimmnetz-<id>`, written by the script itself before GRIMMNETZ_DONE) is already a normal,
/// fully-formed instance on the server - the existing "Vorhandene Server suchen"
/// (`discover_instances`) already finds and imports those, same as any other server-side
/// instance the local DB doesn't know about yet.
#[tauri::command]
async fn list_active_installs(state: State<'_, AppState>, server_id: String) -> Result<Vec<ActiveInstall>, String> {
    let known_ids: std::collections::HashSet<String> = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.list_instances(&server_id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|i| i.id)
            .collect()
    };

    // `/home/gameserver` is 0750 (gameserver:gameserver) - the SSH login user can't even list
    // it, let alone `install.sh`'s 0700, so this whole scan (including the glob expansion
    // itself, which happens in THIS shell, not a nested one) has to run as root via `sudo bash -c`.
    const SCAN_CMD: &str = "sudo bash -c 'for d in /home/gameserver/instances/*/; do \
               id=$(basename \"$d\"); \
               log=\"$d/install.log\"; \
               [ -f \"$log\" ] || continue; \
               tail -c 4096 \"$log\" | grep -qE \"GRIMMNETZ_DONE|GRIMMNETZ_FAILED\" && continue; \
               unit=\"grimmnetz-install-$id\"; \
               active=$(systemctl is-active \"$unit\" 2>/dev/null); \
               { [ \"$active\" = \"active\" ] || [ \"$active\" = \"activating\" ]; } || continue; \
               gid=$(cat \"$d/.grimmnetz-game-id\" 2>/dev/null); \
               echo \"$id|$gid\"; \
             done'";

    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();
    // A pooled session left dead by an earlier dropped connection (e.g. a timed-out
    // start_install) would otherwise fail this call silently forever - drop the stale slot and
    // retry once on a brand-new connection, same self-healing pattern used elsewhere for exactly
    // this scenario.
    let output = match session.exec(SCAN_CMD).await {
        Ok(out) => out,
        Err(_) => {
            *guard = None;
            let mut fresh = connect_fresh(&state, &server_id).await?;
            let out = fresh.exec(SCAN_CMD).await.map_err(|e| e.to_string())?;
            *guard = Some(fresh);
            out
        }
    };

    Ok(output
        .lines()
        .filter_map(|line| {
            let (id, gid) = line.trim().split_once('|')?;
            if id.is_empty() || known_ids.contains(id) {
                return None;
            }
            Some(ActiveInstall { instance_id: id.to_string(), game_id: gid.to_string(), running: true })
        })
        .collect())
}

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
    // Both `sudo` (same reason as list_active_installs: /home/gameserver is 0750, the SSH login
    // user can't traverse into it at all) and `--retry` (install.log doesn't exist yet for the
    // brief moment between `systemctl start --no-block` returning and the unit's ExecStart
    // actually opening its redirect - without --retry, tail fails instantly on the missing file,
    // which this function used to misreport as "connection lost" on every single fresh install).
    let tail_cmd = format!(
        "sudo tail -f --retry -n +1 {install_path}/install.log 2>/dev/null | \
         awk '{{ print; fflush(); if ($0 ~ /^GRIMMNETZ_DONE/ || $0 ~ /^GRIMMNETZ_FAILED/) exit }}'"
    );

    let mut failure: Option<String> = None;
    let mut done = false;
    let mut total_steps = template.install.steps.len();
    // Idle timeout, NOT a cap on the total call duration - a flat total-duration timeout would
    // fire on every install that takes longer than the timeout regardless of whether data is
    // still flowing fine, forcing a reconnect on a fixed cycle the whole way through (that was a
    // real bug here: a 30s total timeout around a multi-minute tail forced a fresh reconnect
    // roughly every 30s for the entire install, misreported to the user as repeated drops).
    // Dropping the pooled slot on any failure forces the next retry onto a guaranteed-fresh
    // connection instead of potentially reusing a genuinely stuck one.
    let stream_result = session
        .exec_stream_lines_idle(&tail_cmd, Some(std::time::Duration::from_secs(60)), |line| {
            if let Some(rest) = line.strip_prefix("GRIMMNETZ_STEP ") {
                if rest == "unit" {
                    let _ = on_event.send(InstallEvent::Step { label: "Dienst wird eingerichtet".to_string() });
                } else if let Some((n, total)) = rest.split_once('/') {
                    total_steps = total.parse().unwrap_or(total_steps);
                    let _ = on_event.send(InstallEvent::Step { label: format!("Schritt {n}/{total}") });
                }
            } else if line.starts_with("GRIMMNETZ_DONE") {
                done = true;
            } else if let Some(msg) = line.strip_prefix("GRIMMNETZ_FAILED:") {
                failure = Some(msg.to_string());
            } else if let Some((percent, phase)) = parse_steamcmd_progress(&line) {
                let _ = on_event.send(InstallEvent::Progress { percent, phase });
            }
        })
        .await;

    if let Err(e) = stream_result {
        *guard = None;
        return Err(e.to_string());
    }

    if let Some(msg) = failure {
        return Err(msg);
    }
    // The tail pipeline can end without either marker - either a genuine transient drop (retry
    // is right), or the GRIMMNETZ_FAILED line got written right as fail() also deleted the
    // instance directory out from under the tail, losing the race to report it. Telling those
    // apart matters: the ambiguous message below contains "Verbindung", which the frontend's
    // retry filter treats as retryable - a real failure worded that way would retry forever and
    // never show the user anything. Checking whether the directory still exists distinguishes
    // them cheaply: gone means fail() really did run, even though its message was missed.
    if !done {
        let dir_gone = session
            .exec(&format!("test -d {install_path} || echo GRIMMNETZ_DIR_GONE"))
            .await
            .map(|out| out.contains("GRIMMNETZ_DIR_GONE"))
            .unwrap_or(false);
        if dir_gone {
            return Err("Installation fehlgeschlagen (Fehlermeldung nicht mehr verfügbar, Instanzordner wurde bereits aufgeräumt).".to_string());
        }
        return Err("Verbindung zum Install-Log verloren, bevor die Installation abgeschlossen war - erneut versuchen.".to_string());
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
    // Reconnect-safe: attach_install_stream may be called again for an already-completed
    // install (log gets re-tailed from byte 0, so DONE matches again). Only insert once -
    // a second unconditional insert_instance would hit the instances.id PRIMARY KEY and
    // fail with a UNIQUE constraint error on every attach after the first successful one.
    if let Some(existing) = db.get_instance(&record.id).map_err(|e| e.to_string())? {
        return Ok(existing);
    }
    db.insert_instance(&record).map_err(|e| e.to_string())?;
    Ok(record)
}

/// Extracts the file a template's start command would run, relative to the install directory,
/// so `discover_instances` can test for its existence to guess a game when there's no manifest.
/// Handles both `./binary args...` and `java ... -jar name.jar ...` style commands - covers
/// every current template.
fn signature_path_for_template(t: &GameTemplate) -> Option<String> {
    if let Some(rest) = t.start_command.strip_prefix("./") {
        return rest.split_whitespace().next().map(|s| s.to_string());
    }
    if let Some(idx) = t.start_command.find("-jar ") {
        let after = &t.start_command[idx + 5..];
        return after.split_whitespace().next().map(|s| s.to_string());
    }
    None
}

/// Module A/B: finds GrimmNetz-managed systemd units on the server that aren't in the local
/// database yet - e.g. after reinstalling the app, or after an identifier change (like
/// v0.1.23's rename) wipes everyone's local app data - and re-imports them so they show up in
/// the UI again. The game itself, its systemd unit, and its firewall rules already live
/// entirely on the server; the only thing that previously existed nowhere but our local
/// SQLite DB was which game/display name/limits an instance had - which `install_game` now
/// also writes into a small `.grimmnetz-instance.json` manifest alongside the game files.
/// For instances installed before that manifest existed, falls back to guessing the game from
/// a characteristic file each template's start command references.
#[tauri::command]
async fn discover_instances(state: State<'_, AppState>, server_id: String) -> Result<Vec<InstanceRecord>, String> {
    let known: std::collections::HashSet<String> = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.list_instances(&server_id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|i| i.id)
            .collect()
    };

    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();

    let unit_list = session
        .exec("find /etc/systemd/system -maxdepth 1 -name 'grimmnetz-*.service' -printf '%f\\n' 2>/dev/null")
        .await
        .map_err(|e| e.to_string())?;

    let templates = games::load_templates();
    let mut discovered = Vec::new();

    for line in unit_list.lines() {
        let unit_name = line.trim().trim_end_matches(".service").to_string();
        // Scheduled-restart trigger services (e.g. "grimmnetz-restart-<unit>-<id>") and the
        // install-tracking units (e.g. "grimmnetz-install-<id>", see start_install) must never
        // be mistaken for a game instance, or discovery would insert a bogus non-existent one.
        let instance_id = match unit_name.strip_prefix("grimmnetz-") {
            Some(id) if !id.is_empty() && !id.starts_with("restart-") && !id.starts_with("install-") => id.to_string(),
            _ => continue,
        };
        if known.contains(&instance_id) {
            continue;
        }
        let install_path = format!("/home/gameserver/instances/{instance_id}");

        let manifest_raw = session
            .exec(&format!("sudo cat {install_path}/.grimmnetz-instance.json 2>/dev/null"))
            .await
            .unwrap_or_default();
        let manifest: Option<serde_json::Value> = serde_json::from_str(&manifest_raw).ok();

        let (game_id, display_name, cpu_limit_percent, ram_limit_mb) = if let Some(m) = manifest {
            (
                m.get("game_id").and_then(|v| v.as_str()).unwrap_or("unknown").to_string(),
                m.get("display_name").and_then(|v| v.as_str()).unwrap_or(&instance_id).to_string(),
                m.get("cpu_limit_percent").and_then(|v| v.as_u64()).unwrap_or(100) as u32,
                m.get("ram_limit_mb").and_then(|v| v.as_u64()).unwrap_or(2048) as u32,
            )
        } else {
            let mut found: Option<&GameTemplate> = None;
            for t in &templates {
                if let Some(sig) = signature_path_for_template(t) {
                    let check = session
                        .exec(&format!(
                            "sudo test -f {}/{} && echo yes",
                            games::shell_single_quote(&install_path),
                            games::shell_single_quote(&sig)
                        ))
                        .await
                        .unwrap_or_default();
                    if check.trim() == "yes" {
                        found = Some(t);
                        break;
                    }
                }
            }
            (
                found.map(|t| t.id.clone()).unwrap_or_else(|| "unknown".to_string()),
                found.map(|t| t.name.clone()).unwrap_or_else(|| instance_id.clone()),
                found.map(|t| t.default_cpu_limit_percent).unwrap_or(100),
                found.map(|t| t.default_ram_limit_mb).unwrap_or(2048),
            )
        };

        discovered.push(InstanceRecord {
            id: instance_id,
            server_id: server_id.clone(),
            game_id,
            display_name,
            install_path,
            systemd_unit: unit_name,
            cpu_limit_percent,
            ram_limit_mb,
        });
    }

    let db = state.db.lock().map_err(|e| e.to_string())?;
    for record in &discovered {
        db.insert_instance(record).map_err(|e| e.to_string())?;
    }

    Ok(discovered)
}

#[derive(Serialize, Deserialize, Clone)]
pub struct VersionInfo {
    pub installed: Option<String>,
    pub latest: Option<String>,
    pub up_to_date: bool,
}

/// Module B: reports the installed vs. latest available version for a game instance, so the
/// UI can show a real version number instead of a generic label and offer an update when
/// they diverge. Only implemented for Paper (Minecraft) right now - other games' installed
/// version isn't easily readable from a single file the way Paper's version_history.json is.
#[tauri::command]
async fn get_instance_version(
    state: State<'_, AppState>,
    server_id: String,
    game_id: String,
    install_path: String,
) -> Result<VersionInfo, String> {
    if game_id != "minecraft-paper" {
        let template = games::find_template(&game_id).ok_or_else(|| format!("Unbekanntes Spiel: {game_id}"))?;
        let Some(app_id) = template.install.app_id else {
            return Err("Für dieses Spiel gibt es noch keine Versionsanzeige".to_string());
        };

        let mut guard = acquire_session(&state, &server_id).await?;
        let session = guard.as_mut().unwrap();
        let manifest_path = format!("{install_path}/steamapps/appmanifest_{app_id}.acf");
        let raw = session
            .exec(&format!("sudo cat {} 2>/dev/null", games::shell_single_quote(&manifest_path)))
            .await
            .map_err(|e| e.to_string())?;

        // steamcmd doesn't reliably expose a "latest build for this branch" number without a
        // much heavier app_info_print + VDF parse, so this only ever reports the locally
        // installed build - never a false "update available" that isn't backed by a real check.
        let installed = raw
            .lines()
            .find_map(|line| line.trim().strip_prefix("\"buildid\"").map(|rest| rest.trim().trim_matches('"').to_string()));

        return Ok(VersionInfo { installed, latest: None, up_to_date: true });
    }

    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();

    let history_path = format!("{install_path}/.paper/version_history.json");
    let raw = session
        .exec(&format!("sudo cat {} 2>/dev/null", games::shell_single_quote(&history_path)))
        .await
        .map_err(|e| e.to_string())?;

    let installed = serde_json::from_str::<serde_json::Value>(&raw)
        .ok()
        .and_then(|v| v.get("currentVersion").and_then(|s| s.as_str().map(String::from)))
        .and_then(|s| {
            // "26.2-92-0a99345 (MC: 26.2)" -> "26.2"
            let start = s.find("MC: ")? + 4;
            let end = s[start..].find(')')? + start;
            Some(s[start..end].to_string())
        });

    let latest = session
        .exec("curl -s https://fill.papermc.io/v3/projects/paper | jq -r '.versions | to_entries[0].value[0]'")
        .await
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let up_to_date = match (&installed, &latest) {
        (Some(i), Some(l)) => i == l,
        _ => false,
    };

    Ok(VersionInfo { installed, latest, up_to_date })
}

#[derive(Serialize, Deserialize, Clone)]
pub struct MinecraftLiveStatus {
    pub world: Option<String>,
    pub players_online: Option<u32>,
    pub players_max: Option<u32>,
}

/// Module C: reads the world name from server.properties and live player counts via the
/// vanilla Server List Ping protocol - the exact same query the in-game multiplayer server
/// list performs - so "Spieler Online"/"Welt" in the UI show real data instead of a
/// permanent placeholder. Player counts are best-effort: if the server can't be reached
/// directly (firewalled off from the machine running the app, still starting up, etc.) those
/// fields just come back None rather than failing the whole call.
#[tauri::command]
async fn get_minecraft_live_status(
    state: State<'_, AppState>,
    server_id: String,
    install_path: String,
) -> Result<MinecraftLiveStatus, String> {
    let host = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_server(&server_id)
            .map_err(|e| e.to_string())?
            .host
            .ok_or_else(|| "Server hat noch keine IP-Adresse".to_string())?
    };

    let raw = {
        let mut guard = acquire_session(&state, &server_id).await?;
        let session = guard.as_mut().unwrap();
        let props_path = format!("{install_path}/server.properties");
        session
            .exec(&format!("sudo cat {} 2>/dev/null", games::shell_single_quote(&props_path)))
            .await
            .map_err(|e| e.to_string())?
    };

    let mut world = None;
    let mut port: u16 = 25565;
    for line in raw.lines() {
        if let Some((k, v)) = line.trim().split_once('=') {
            match k.trim() {
                "level-name" => world = Some(v.trim().to_string()),
                "server-port" => port = v.trim().parse().unwrap_or(25565),
                _ => {}
            }
        }
    }

    let (players_online, players_max) = mc_ping::ping(&host, port).await.unwrap_or((None, None));

    Ok(MinecraftLiveStatus { world, players_online, players_max })
}

/// Module B: re-runs a game instance's install steps (re-downloading the latest build) and
/// restarts it - used by the "Update" action when get_instance_version reports a mismatch.
#[tauri::command]
async fn update_instance(
    state: State<'_, AppState>,
    server_id: String,
    game_id: String,
    install_path: String,
    unit_name: String,
    ram_limit_mb: u32,
) -> Result<(), String> {
    let template = games::find_template(&game_id).ok_or_else(|| format!("Unbekanntes Spiel: {game_id}"))?;

    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();

    provisioning::control_instance(session, &unit_name, "stop")
        .await
        .map_err(|e| e.to_string())?;

    // install_path already contains the instance id as its last path segment.
    let instance_id = install_path.rsplit('/').next().unwrap_or_default();
    for step in &template.install.steps {
        let rendered = games::render_step(step, instance_id, ram_limit_mb);
        let quoted = games::shell_single_quote(&rendered);
        session
            .exec_long(&format!("sudo -u gameserver bash -c {quoted}"))
            .await
            .map_err(|e| e.to_string())?;
    }

    provisioning::control_instance(session, &unit_name, "start")
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}

/// Module B: reads a game instance's config file over SSH and extracts the values for the
/// fields declared in its template's config schema (falling back to defaults when a field
/// or the file itself doesn't exist yet, e.g. right after install).
#[tauri::command]
async fn get_instance_config(
    state: State<'_, AppState>,
    server_id: String,
    game_id: String,
    install_path: String,
) -> Result<HashMap<String, String>, String> {
    let template = games::find_template(&game_id).ok_or_else(|| format!("Unbekanntes Spiel: {game_id}"))?;
    let schema = template
        .config
        .ok_or_else(|| "Für dieses Spiel gibt es noch keine Konfigurationsoberfläche".to_string())?;

    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();
    let path = format!("{install_path}/{}", schema.file);
    // The config file is owned by `gameserver`, not the SSH login user - a plain `cat` gets
    // "Permission denied" (silently swallowed by the redirect) and we'd parse an empty file,
    // showing every field's default instead of its real value.
    let raw = session
        .exec(&format!("sudo cat {} 2>/dev/null", games::shell_single_quote(&path)))
        .await
        .map_err(|e| e.to_string())?;

    Ok(games::parse_config(&schema, &raw))
}

/// Module B: writes updated field values back into a game instance's config file, preserving
/// unrelated content, so the game's own config format doesn't get clobbered.
#[tauri::command]
async fn save_instance_config(
    state: State<'_, AppState>,
    server_id: String,
    game_id: String,
    install_path: String,
    values: HashMap<String, String>,
) -> Result<(), String> {
    let template = games::find_template(&game_id).ok_or_else(|| format!("Unbekanntes Spiel: {game_id}"))?;
    let schema = template
        .config
        .ok_or_else(|| "Für dieses Spiel gibt es noch keine Konfigurationsoberfläche".to_string())?;

    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();
    let path = format!("{install_path}/{}", schema.file);
    let quoted_path = games::shell_single_quote(&path);
    let raw = session
        .exec(&format!("sudo cat {quoted_path} 2>/dev/null"))
        .await
        .map_err(|e| e.to_string())?;

    let rendered = games::render_config(&schema, &raw, &values);
    // Pipe the rendered content in via stdin instead of interpolating it into the command
    // string, so config values containing quotes/`$`/backticks can never break out of the
    // shell command (same pattern as ensure_passwordless_sudo's stdin-piped password).
    session
        .exec_with_stdin(
            &format!("sudo -u gameserver tee {quoted_path} > /dev/null"),
            rendered.as_bytes(),
        )
        .await
        .map_err(|e| e.to_string())?;

    // If a field controls the game's listen port and it changed, open the new port too -
    // best-effort, doesn't fail the save if the firewall step has trouble.
    for field in &schema.fields {
        if let Some(protocol) = &field.opens_port_protocol {
            if let Some(new_value) = values.get(&field.key) {
                if let Ok(port) = new_value.parse::<u16>() {
                    if let Ok(family) = provisioning::detect_distro_family(session).await {
                        let _ = provisioning::open_port(session, family, port, protocol).await;
                    }
                }
            }
        }
    }

    Ok(())
}

/// Module C: start/stop/restart a game instance via systemd.
#[tauri::command]
async fn control_instance(state: State<'_, AppState>, server_id: String, unit_name: String, action: String) -> Result<String, String> {
    let mut guard = acquire_session(&state, &server_id).await?;
    let result = provisioning::control_instance(guard.as_mut().unwrap(), &unit_name, &action).await;
    match result {
        Ok(v) => Ok(v),
        Err(e) => {
            *guard = None;
            Err(e.to_string())
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct InstanceStatus {
    pub state: String,
    pub uptime_seconds: i64,
    pub pid: Option<i64>,
    pub started_at: Option<String>,
}

/// Module C: reports whether a game instance's systemd unit is currently running
/// and how long it's been up, so the UI can show a real status badge + uptime
/// instead of guessing.
#[tauri::command]
async fn get_instance_status(state: State<'_, AppState>, server_id: String, unit_name: String) -> Result<InstanceStatus, String> {
    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();
    let result = session
        .exec(&format!(
            "STATE=$(systemctl is-active {unit_name}); \
             TS=$(systemctl show -p ActiveEnterTimestamp --value {unit_name}); \
             PID=$(systemctl show -p MainPID --value {unit_name}); \
             if [ -n \"$TS\" ] && [ \"$TS\" != \"n/a\" ]; then \
               NOW=$(date +%s); THEN=$(date -d \"$TS\" +%s 2>/dev/null || echo $NOW); UPTIME=$((NOW-THEN)); \
             else UPTIME=0; fi; \
             echo \"$STATE|$UPTIME|$PID|$TS\""
        ))
        .await;

    let output = match result {
        Ok(v) => v,
        Err(e) => {
            *guard = None;
            return Err(e.to_string());
        }
    };

    let mut parts = output.trim().splitn(4, '|');
    let state = parts.next().unwrap_or("unknown").to_string();
    let uptime_seconds = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let pid = parts.next().and_then(|v| v.parse::<i64>().ok()).filter(|&p| p != 0);
    let started_at = parts.next().map(|v| v.trim().to_string()).filter(|v| !v.is_empty() && v != "n/a");
    Ok(InstanceStatus { state, uptime_seconds, pid, started_at })
}

#[derive(Serialize, Deserialize, Clone)]
pub struct HostCapacity {
    pub cpu_cores: u32,
    pub ram_total_mb: u32,
}

/// So the resource sliders in the UI can be capped at what the server actually has, instead of
/// letting the user assign more RAM/CPU to one instance than the whole machine owns.
#[tauri::command]
async fn get_host_capacity(state: State<'_, AppState>, server_id: String) -> Result<HostCapacity, String> {
    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();
    let output = session
        .exec("echo \"$(nproc)|$(awk '/MemTotal/ {print int($2/1024)}' /proc/meminfo)\"")
        .await
        .map_err(|e| e.to_string())?;

    let mut parts = output.trim().splitn(2, '|');
    let cpu_cores = parts.next().and_then(|v| v.parse().ok()).unwrap_or(1);
    let ram_total_mb = parts.next().and_then(|v| v.parse().ok()).unwrap_or(1024);
    Ok(HostCapacity { cpu_cores, ram_total_mb })
}

/// Rewrites an instance's resource limits both in the systemd unit (the part that's actually
/// enforced by the kernel) and the local DB record (so the UI reflects it immediately), then
/// restarts the service so the new MemoryMax/CPUQuota take effect right away.
#[tauri::command]
async fn update_instance_resources(
    state: State<'_, AppState>,
    server_id: String,
    instance_id: String,
    unit_name: String,
    install_path: String,
    cpu_limit_percent: u32,
    ram_limit_mb: u32,
) -> Result<(), String> {
    validate_instance_path(&install_path, &instance_id)?;

    let start_command = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let instances = db.list_instances(&server_id).map_err(|e| e.to_string())?;
        let record = instances
            .iter()
            .find(|i| i.id == instance_id)
            .ok_or_else(|| "Instanz nicht gefunden".to_string())?;
        let template = games::find_template(&record.game_id).ok_or_else(|| "Unbekanntes Spiel".to_string())?;
        games::render_step(&template.start_command, &instance_id, ram_limit_mb)
    };

    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();
    let unit_contents =
        provisioning::render_systemd_unit(&instance_id, &install_path, &start_command, ram_limit_mb, cpu_limit_percent);
    provisioning::install_systemd_unit(session, &unit_name, &unit_contents)
        .await
        .map_err(|e| e.to_string())?;
    provisioning::control_instance(session, &unit_name, "restart")
        .await
        .map_err(|e| e.to_string())?;

    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.update_instance_resources(&instance_id, cpu_limit_percent, ram_limit_mb)
        .map_err(|e| e.to_string())
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ScheduledRestart {
    pub id: String,
    pub time: String, // "HH:MM", 24h
}

/// Only ever alphanumeric/dash/colon, and always exactly `unit_name` as a prefix - so a
/// malformed id/unit_name can never widen a `rm`/`systemctl` call to another instance's timer.
/// Shifts a "HH:MM" time by `delta_minutes` (may be negative), wrapping around midnight.
fn shift_time(hh: &str, mm: &str, delta_minutes: i32) -> (String, String) {
    let total = hh.parse::<i32>().unwrap_or(0) * 60 + mm.parse::<i32>().unwrap_or(0);
    let shifted = (total + delta_minutes).rem_euclid(24 * 60);
    (format!("{:02}", shifted / 60), format!("{:02}", shifted % 60))
}

/// Path of the FIFO systemd feeds into the game process's stdin (wired up in
/// `render_systemd_unit`) - the only way to get a `say ...` console command into a running
/// instance, since these are plain systemd services with no other IPC channel.
fn console_fifo_path(unit_name: &str) -> Option<String> {
    let instance_id = unit_name.strip_prefix("grimmnetz-")?;
    Some(format!("/run/grimmnetz-{instance_id}.console"))
}

/// A single best-effort console write: `timeout` guards against ever hanging forever if the
/// FIFO has no reader (game not actually running) - `echo > fifo` blocks indefinitely otherwise.
fn console_say_command(fifo: &str, say_format: &str, message: &str) -> String {
    let line = say_format.replace("{message}", message);
    let echo_cmd = format!("echo {} > {fifo}", games::shell_single_quote(&line));
    format!("timeout 2 sudo -u gameserver bash -c {} 2>/dev/null", games::shell_single_quote(&echo_cmd))
}

/// Minimal Source-RCON client, run server-side via `python3 -c` over the existing SSH session -
/// avoids needing a Rust TCP client from the desktop app (which would require opening the RCON
/// port to the internet) and avoids shipping a compiled RCON binary. Connects to 127.0.0.1 only,
/// matching `--rcon-bind 127.0.0.1:<port>` in the game's start command.
const RCON_PY: &str = r#"
import socket,struct,sys
def pkt(i,t,b):
    body=b.encode()+b'\x00\x00'
    data=struct.pack('<ii',i,t)+body
    return struct.pack('<i',len(data))+data
port=int(sys.argv[1])
pw=open(sys.argv[2]).read().strip()
cmd=sys.argv[3]
s=socket.create_connection(('127.0.0.1',port),5)
s.sendall(pkt(1,3,pw))
auth=s.recv(4096)
if len(auth)<8 or struct.unpack('<i',auth[4:8])[0]==-1:
    sys.exit(1)
s.sendall(pkt(2,2,cmd))
s.recv(4096)
"#;

fn rcon_command(port: u16, password_file: &str, command: &str) -> String {
    // The password file is owned by `gameserver` with mode 600 - the SSH login user is neither
    // root nor `gameserver`, so reading it needs the same `sudo -u gameserver` wrapper the FIFO
    // path already uses, otherwise `open()` fails with Permission denied (swallowed by 2>/dev/null).
    let py_cmd = format!(
        "python3 -c {} {port} {} {}",
        games::shell_single_quote(RCON_PY),
        games::shell_single_quote(password_file),
        games::shell_single_quote(command)
    );
    format!("timeout 5 sudo -u gameserver bash -c {} 2>/dev/null", games::shell_single_quote(&py_cmd))
}

/// Builds the ready-to-exec shell command that announces `message` inside the running game
/// identified by `unit_name` - RCON for games that declare an `RconSpec` (Factorio: stdin isn't
/// reliably read), the stdin FIFO otherwise. Falls back to the generic FIFO `say` form if the
/// instance/template can't be found (e.g. was deleted between listing and use).
fn build_broadcast_command(db: &db::Db, server_id: &str, unit_name: &str, message: &str) -> String {
    let instance = db.find_instance_by_unit(server_id, unit_name).ok().flatten();
    let template = instance.as_ref().and_then(|inst| games::find_template(&inst.game_id));

    if let (Some(inst), Some(tpl)) = (&instance, &template) {
        if let Some(rcon) = &tpl.rcon {
            let password_path = format!("{}/{}", inst.install_path, rcon.password_file);
            let command = rcon.broadcast_command.replace("{message}", message);
            return rcon_command(rcon.port, &password_path, &command);
        }
    }
    let fifo = console_fifo_path(unit_name).unwrap_or_default();
    let say_format = template.map(|tpl| tpl.console_say_format).unwrap_or_else(|| "say {message}".to_string());
    console_say_command(&fifo, &say_format, message)
}

/// If the game declares a `save_command`, builds the RCON command that forces an immediate save -
/// run right before restart/stop so the countdown doesn't depend on the regular autosave timing.
fn build_save_command(db: &db::Db, server_id: &str, unit_name: &str) -> Option<String> {
    let inst = db.find_instance_by_unit(server_id, unit_name).ok().flatten()?;
    let tpl = games::find_template(&inst.game_id)?;
    let rcon = tpl.rcon?;
    let save = rcon.save_command?;
    let password_path = format!("{}/{}", inst.install_path, rcon.password_file);
    Some(rcon_command(rcon.port, &password_path, &save))
}

fn format_duration(seconds: u32) -> String {
    if seconds < 60 {
        format!("{seconds} Sek")
    } else if seconds % 60 == 0 {
        format!("{} Min", seconds / 60)
    } else {
        format!("{} Min {} Sek", seconds / 60, seconds % 60)
    }
}

/// The fixed countdown announcement marks, in seconds-before-the-action and their display label -
/// same schedule for every game so behaviour doesn't quietly differ per template. Coarse marks
/// down to 1 minute, then a 30s mark, then every single second for the final 10.
fn countdown_marks() -> Vec<(u32, String)> {
    let mut marks: Vec<(u32, String)> = vec![
        (900, "15 Min".to_string()),
        (600, "10 Min".to_string()),
        (300, "5 Min".to_string()),
        (240, "4 Min".to_string()),
        (180, "3 Min".to_string()),
        (120, "2 Min".to_string()),
        (60, "1 Min".to_string()),
        (30, "30 Sek".to_string()),
    ];
    for s in (1..=10).rev() {
        marks.push((s, format!("{s} Sek")));
    }
    marks
}

/// Builds the full shell script for an announced restart/stop: optional immediate custom message,
/// then every countdown mark that fits inside `total_seconds`, then an optional forced save, then
/// the actual `systemctl` action. Shared by the "Neustart mit Ansage" button and the daily
/// scheduled-restart timer so both get the same countdown behaviour.
fn build_countdown_script(
    db: &db::Db,
    server_id: &str,
    unit_name: &str,
    action: &str,
    custom_message: Option<&str>,
    total_seconds: u32,
) -> String {
    let mut parts = Vec::new();
    let action_word = if action == "restart" { "Neustart" } else { "Stop" };
    if let Some(msg) = custom_message {
        parts.push(build_broadcast_command(db, server_id, unit_name, msg));
        // Same line-count as a paragraph break in chat, without embedding a raw newline into the
        // FIFO/RCON payload - each broadcast call is already its own line in the game's chat.
        parts.push(build_broadcast_command(db, server_id, unit_name, &format!("Server {action_word} in {}", format_duration(total_seconds))));
    }
    let mut prev = total_seconds;
    for (mark_secs, label) in countdown_marks() {
        let within = if custom_message.is_some() { mark_secs < total_seconds } else { mark_secs <= total_seconds };
        if !within {
            continue;
        }
        let wait = prev.saturating_sub(mark_secs);
        if wait > 0 {
            parts.push(format!("sleep {wait}"));
        }
        parts.push(build_broadcast_command(db, server_id, unit_name, &format!("Server {action_word} in {label}")));
        prev = mark_secs;
    }
    if prev > 0 {
        parts.push(format!("sleep {prev}"));
    }
    if let Some(save_cmd) = build_save_command(db, server_id, unit_name) {
        parts.push(save_cmd);
        parts.push("sleep 2".to_string());
    }
    parts.push(format!("sudo systemctl {action} {unit_name}"));
    parts.join(" ; ")
}

fn timer_name(unit_name: &str, id: &str) -> Result<String, String> {
    let valid = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !valid(unit_name) || !valid(id) {
        return Err("Ungültiger Bezeichner".to_string());
    }
    Ok(format!("restart-{unit_name}-{id}"))
}

/// Scheduled restarts live entirely as systemd timers on the game server itself (not in our
/// local DB) - they keep firing even if GrimmNetz is closed or the local install is ever lost,
/// same reasoning as the rest of the server-side-source-of-truth design in this app.
#[tauri::command]
async fn list_scheduled_restarts(
    state: State<'_, AppState>,
    server_id: String,
    unit_name: String,
) -> Result<Vec<ScheduledRestart>, String> {
    if unit_name.is_empty() || !unit_name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("Ungültiger Bezeichner".to_string());
    }
    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();
    let prefix = format!("restart-{unit_name}-");
    let output = session
        .exec(&format!(
            "for f in /etc/systemd/system/grimmnetz-{prefix}*.timer; do \
               [ -e \"$f\" ] || continue; \
               NAME=$(basename \"$f\" .timer); \
               CAL=$(grep -oP 'OnCalendar=\\*-\\*-\\* \\K[0-9:]+' \"$f\"); \
               echo \"$NAME|$CAL\"; \
             done"
        ))
        .await
        .map_err(|e| e.to_string())?;

    let restart_prefix = format!("grimmnetz-{prefix}");
    Ok(output
        .lines()
        .filter_map(|line| {
            let (name, cal) = line.split_once('|')?;
            let id = name.strip_prefix(&restart_prefix)?;
            // The timer itself fires 15 min before the displayed restart time (that's when the
            // countdown announcements start) - shift back so the UI shows the actual restart time.
            let (h, m) = cal.get(0..2).zip(cal.get(3..5))?;
            let (h, m) = shift_time(h, m, 15);
            Some(ScheduledRestart { id: id.to_string(), time: format!("{h}:{m}") })
        })
        .collect())
}

/// Writes a `.timer` + `.service` pair that runs `systemctl restart <unit_name>` daily at the
/// given time - a plain, minimal alternative to cron that's consistent with how every other
/// game instance is already managed (systemd unit files, not a separate scheduling system).
#[tauri::command]
async fn add_scheduled_restart(
    state: State<'_, AppState>,
    server_id: String,
    unit_name: String,
    time: String,
) -> Result<(), String> {
    let (hh, mm) = time
        .split_once(':')
        .filter(|(h, m)| h.len() == 2 && m.len() == 2 && h.chars().all(|c| c.is_ascii_digit()) && m.chars().all(|c| c.is_ascii_digit()))
        .ok_or_else(|| "Ungültige Uhrzeit".to_string())?;

    let id = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
    let full_name = format!("grimmnetz-{}", timer_name(&unit_name, &id)?);

    // Fires 15 min before the time the user picked, counts down with in-game announcements,
    // and only restarts once it lands exactly on the requested time - not the other way around
    // (restart at HH:MM, warnings trailing off beforehand would mean the countdown text lies
    // about how much time is actually left).
    let countdown = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        build_countdown_script(&db, &server_id, &unit_name, "restart", None, 15 * 60)
    };
    let (warn_hh, warn_mm) = shift_time(hh, mm, -15);

    let service_contents = format!(
        "[Unit]\nDescription=Geplanter Neustart fuer {unit_name}\n\n\
         [Service]\nType=oneshot\nExecStart=/bin/bash -c '{}'\n",
        countdown.replace('\'', "'\\''")
    );
    let timer_contents = format!(
        "[Unit]\nDescription=Timer fuer geplanten Neustart von {unit_name}\n\n\
         [Timer]\nOnCalendar=*-*-* {warn_hh}:{warn_mm}:00\nPersistent=false\n\n\
         [Install]\nWantedBy=timers.target\n"
    );

    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();
    provisioning::install_systemd_unit(session, &full_name, &service_contents)
        .await
        .map_err(|e| e.to_string())?;
    session
        .exec(&format!(
            "sudo tee /etc/systemd/system/{full_name}.timer > /dev/null <<'GRIMMNETZ_EOF'\n{timer_contents}GRIMMNETZ_EOF\n\
             sudo systemctl daemon-reload && sudo systemctl enable --now {full_name}.timer"
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Broadcasts an operator-chosen message into the running game (e.g. "Wartungsarbeiten, Server
/// startet in 5 Minuten neu"), waits the chosen delay, then restarts or stops the instance.
/// Runs on its own dedicated connection detached from the shared session pool - the shared
/// pool slot would otherwise sit locked out for the whole delay (up to 15 minutes).
#[tauri::command]
async fn broadcast_and_execute(
    state: State<'_, AppState>,
    server_id: String,
    unit_name: String,
    message: String,
    delay_minutes: u32,
    action: String,
) -> Result<(), String> {
    if action != "restart" && action != "stop" {
        return Err("Ungültige Aktion".to_string());
    }
    let delay_seconds = delay_minutes.min(180) * 60;
    let script = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        build_countdown_script(&db, &server_id, &unit_name, &action, Some(&message), delay_seconds)
    };

    let mut session = connect_fresh(&state, &server_id).await?;
    tauri::async_runtime::spawn(async move {
        let _ = session.exec_long(&script).await;
    });
    Ok(())
}

#[tauri::command]
async fn remove_scheduled_restart(
    state: State<'_, AppState>,
    server_id: String,
    unit_name: String,
    id: String,
) -> Result<(), String> {
    let full_name = format!("grimmnetz-{}", timer_name(&unit_name, &id)?);
    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();
    session
        .exec(&format!(
            "sudo systemctl disable --now {full_name}.timer 2>/dev/null; \
             sudo rm -f /etc/systemd/system/{full_name}.timer /etc/systemd/system/{full_name}.service; \
             sudo systemctl daemon-reload"
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Guards against ever running `rm -rf` on something wider than a single instance's own
/// directory - e.g. an empty/truncated path that resolves to the shared instances root.
/// The path must be exactly `/home/gameserver/instances/<instance_id>` with a non-empty,
/// slash-free, dot-free instance_id, so a malformed or missing id can never widen the blast
/// radius to "delete everything installed on the server".
fn validate_instance_path(install_path: &str, instance_id: &str) -> Result<(), String> {
    const BASE: &str = "/home/gameserver/instances/";
    if instance_id.is_empty() || instance_id.contains('/') || instance_id.contains("..") {
        return Err(format!("Ungültige instance_id: {instance_id:?}"));
    }
    let expected = format!("{BASE}{instance_id}");
    if install_path != expected {
        return Err(format!(
            "Install-Pfad passt nicht zur Instanz-ID, breche Löschung ab (erwartet {expected:?}, erhalten {install_path:?})"
        ));
    }
    Ok(())
}

/// "Schlank" option: forgets the instance in GrimmNetz only, leaving the service and its
/// files untouched on the server (e.g. to keep a world/save for later, or re-add it as a
/// server-side unit manually). Does not require an SSH connection.
#[tauri::command]
fn forget_instance(state: State<'_, AppState>, instance_id: String) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.delete_instance(&instance_id).map_err(|e| e.to_string())
}

/// "Radikal" option: fully uninstalls a game instance: stops + disables the service, removes its systemd
/// unit file, reloads the daemon, deletes the installed files, and removes it from the
/// local list - the server ends up exactly as if the instance had never existed.
#[tauri::command]
async fn delete_instance(
    state: State<'_, AppState>,
    server_id: String,
    instance_id: String,
    unit_name: String,
    install_path: String,
) -> Result<(), String> {
    validate_instance_path(&install_path, &instance_id)?;

    // Deleting only from the local DB while the server-side cleanup silently failed would
    // leave the service running and the files in place forever, with the app having no way
    // left to find or retry it - so a dead/failing SSH session must fail the whole command,
    // not just skip straight to removing the local record.
    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();
    let stop_result = provisioning::control_instance(session, &unit_name, "stop").await;
    if let Err(e) = &stop_result {
        *guard = None;
        return Err(format!("Konnte Dienst nicht stoppen: {e}"));
    }
    session
        .exec(&format!("sudo systemctl disable {unit_name}"))
        .await
        .map_err(|e| e.to_string())?;
    session
        .exec(&format!("sudo rm -f /etc/systemd/system/{unit_name}.service"))
        .await
        .map_err(|e| e.to_string())?;
    // Any scheduled restarts for this instance would otherwise keep firing forever afterwards,
    // trying to restart a systemd unit that no longer exists.
    session
        .exec(&format!(
            "for f in /etc/systemd/system/grimmnetz-restart-{unit_name}-*; do \
               [ -e \"$f\" ] || continue; \
               NAME=$(basename \"$f\" | sed -E 's/\\.(timer|service)$//'); \
               sudo systemctl disable --now \"$NAME.timer\" 2>/dev/null; \
               sudo rm -f \"$f\"; \
             done"
        ))
        .await
        .map_err(|e| e.to_string())?;
    // The install-tracking unit (grimmnetz-install-<id>, RemainAfterExit=yes) otherwise stays
    // loaded forever as "active exited" - a real gap found live during Task 8: the game unit and
    // instance directory were cleaned up correctly, but this one was silently left behind on
    // every single deinstall. Best-effort: a missing install unit (e.g. instance was manually
    // imported via "Vorhandene Server suchen") is not an error.
    let install_unit_name = format!("grimmnetz-install-{instance_id}");
    let _ = session
        .exec(&format!(
            // RemainAfterExit=yes keeps the unit "active" indefinitely even after its ExecStart
            // process exits - `disable` alone only removes the enablement symlink, it does NOT
            // stop an active unit, so `stop` has to run first or the unit stays loaded and
            // "active" in systemd's memory even once its file is gone (confirmed live: exactly
            // this happened when `stop` was missing from the first version of this fix).
            "sudo systemctl stop {install_unit_name} 2>/dev/null; \
             sudo systemctl disable {install_unit_name} 2>/dev/null; \
             sudo rm -f /etc/systemd/system/{install_unit_name}.service"
        ))
        .await;
    // Best-effort: a docker-based install that crashed before its ExecStartPre ever ran (or
    // between crash-loop restarts) can leave a stopped container behind that --rm never got to
    // clean up - harmless either way if this game wasn't docker-based (no matching container).
    let _ = session.exec(&format!("sudo docker rm -f {unit_name} 2>/dev/null")).await;
    session.exec("sudo systemctl daemon-reload").await.map_err(|e| e.to_string())?;
    session
        .exec(&format!("sudo rm -rf {install_path}"))
        .await
        .map_err(|e| e.to_string())?;

    // Close the ports this instance's install opened, so repeatedly installing/uninstalling
    // different games doesn't leave the firewall full of stale allow-rules forever (found live:
    // ports stayed open indefinitely after deinstalling). Best-effort - a missing/unknown game
    // template (e.g. an instance whose template was since removed) must not block the delete.
    let instance_game_id = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_instance(&instance_id).ok().flatten().map(|i| i.game_id)
    };
    if let Some(game_id) = instance_game_id {
        if let Some(template) = games::find_template(&game_id) {
            if let Ok(family) = provisioning::detect_distro_family(session).await {
                for p in &template.ports {
                    let _ = provisioning::close_port(session, family, p.port, &p.protocol).await;
                }
            }
        }
    }

    drop(guard);

    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.delete_instance(&instance_id).map_err(|e| e.to_string())
}

#[derive(Serialize, Deserialize, Clone)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub size_bytes: u64,
}

/// Only ever lets the two directories we actually manage for an instance be browsed - never
/// an arbitrary path - since `path` comes straight from the frontend.
fn resolve_browsable_path(instance_id: &str, target: &str) -> Result<String, String> {
    if instance_id.is_empty() || instance_id.contains('/') || instance_id.contains("..") {
        return Err(format!("Ungültige instance_id: {instance_id:?}"));
    }
    match target {
        "install" => Ok(format!("/home/gameserver/instances/{instance_id}")),
        "backups" => Ok(format!("/home/gameserver/backups/{instance_id}")),
        other => Err(format!("Unbekanntes Verzeichnis: {other:?}")),
    }
}

/// Module D: lists the top-level contents (not recursive - just enough to see what's there)
/// of a game instance's install or backups directory.
#[tauri::command]
async fn list_directory(
    state: State<'_, AppState>,
    server_id: String,
    instance_id: String,
    target: String,
) -> Result<Vec<DirEntry>, String> {
    let path = resolve_browsable_path(&instance_id, &target)?;

    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();
    let raw = session
        .exec(&format!(
            "sudo find {path} -mindepth 1 -maxdepth 1 -printf '%y|%f|%s\\n' 2>/dev/null"
        ))
        .await
        .map_err(|e| e.to_string())?;

    let mut entries: Vec<DirEntry> = raw
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '|');
            let type_char = parts.next()?;
            let name = parts.next()?.to_string();
            let size_bytes = parts.next()?.parse().ok()?;
            Some(DirEntry { name, is_dir: type_char == "d", size_bytes })
        })
        .collect();
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(entries)
}

#[derive(Serialize, Deserialize, Clone)]
pub struct BackupEntry {
    pub name: String,
    pub size_bytes: u64,
    /// Unix timestamp (seconds) of the backup file's mtime.
    pub created_at: i64,
}

fn validate_backup_filename(name: &str) -> Result<(), String> {
    if name.is_empty() || name.contains('/') || name.contains("..") || !name.ends_with(".tar.gz") {
        return Err(format!("Ungültiger Backup-Dateiname: {name:?}"));
    }
    Ok(())
}

/// The local folder downloaded backups for an instance get saved into (and where the upload
/// file picker should default to, so downloaded-then-reuploaded backups round-trip through
/// the same place without the user having to hunt for it).
fn local_backup_dir(app: &tauri::AppHandle, instance_id: &str) -> Result<std::path::PathBuf, String> {
    let mut dir = app.path().app_local_data_dir().map_err(|e| e.to_string())?;
    dir.push("backups");
    dir.push(instance_id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Module D: returns (creating if needed) the local folder for an instance's downloaded
/// backups, so the frontend can point the upload file picker there by default.
#[tauri::command]
fn get_local_backup_dir(app: tauri::AppHandle, instance_id: String) -> Result<String, String> {
    Ok(local_backup_dir(&app, &instance_id)?.to_string_lossy().to_string())
}

/// Module D: uploads a local `.tar.gz` file (e.g. a backup the user saved from before a
/// reinstall/update) into an instance's backup directory on the server, so it shows up
/// alongside server-created backups and can be restored the same way.
#[tauri::command]
async fn upload_backup(
    state: State<'_, AppState>,
    server_id: String,
    instance_id: String,
    local_path: String,
    on_progress: Channel<UploadEvent>,
) -> Result<BackupEntry, String> {
    let filename = std::path::Path::new(&local_path)
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "Ungültiger Dateiname".to_string())?
        .to_string();
    validate_backup_filename(&filename)?;

    let data = std::fs::read(&local_path).map_err(|e| e.to_string())?;

    let backup_dir = format!("/home/gameserver/backups/{instance_id}");
    let remote_path = format!("{backup_dir}/{filename}");

    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();
    session
        .exec(&format!(
            "sudo mkdir -p {} && sudo chown gameserver:gameserver {}",
            games::shell_single_quote(&backup_dir),
            games::shell_single_quote(&backup_dir)
        ))
        .await
        .map_err(|e| e.to_string())?;
    session
        .exec_with_stdin_progress(
            &format!("sudo -u gameserver tee {} > /dev/null", games::shell_single_quote(&remote_path)),
            &data,
            |bytes_sent, total_bytes| {
                let _ = on_progress.send(UploadEvent::Progress { bytes_sent, total_bytes });
            },
        )
        .await
        .map_err(|e| e.to_string())?;

    let stat = session
        .exec(&format!("sudo stat -c '%s|%Y' {}", games::shell_single_quote(&remote_path)))
        .await
        .map_err(|e| e.to_string())?;
    let (size_bytes, created_at) = stat
        .trim()
        .split_once('|')
        .and_then(|(s, t)| Some((s.parse().ok()?, t.parse().ok()?)))
        .ok_or_else(|| "Hochgeladen, aber Größe/Zeitstempel konnten nicht gelesen werden".to_string())?;

    Ok(BackupEntry { name: filename, size_bytes, created_at })
}

/// Module D: creates a `.tar.gz` snapshot of an instance's entire install directory under
/// `/home/gameserver/backups/<instance_id>/`, timestamped so multiple backups can coexist.
/// Backs up everything rather than trying to guess each game's "world folder" convention -
/// simpler and safer, at the cost of a bigger archive for games with large install sizes.
#[tauri::command]
async fn create_backup(
    state: State<'_, AppState>,
    server_id: String,
    instance_id: String,
    install_path: String,
) -> Result<BackupEntry, String> {
    validate_instance_path(&install_path, &instance_id)?;

    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();

    let backup_dir = format!("/home/gameserver/backups/{instance_id}");
    let filename = format!("backup-{}.tar.gz", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
    let archive_path = format!("{backup_dir}/{filename}");

    session
        .exec(&format!(
            "sudo mkdir -p {backup_dir} && sudo chown gameserver:gameserver {backup_dir} && \
             sudo -u gameserver tar -czf {archive_path} -C {install_path} ."
        ))
        .await
        .map_err(|e| e.to_string())?;

    let stat = session
        .exec(&format!("sudo stat -c '%s|%Y' {}", games::shell_single_quote(&archive_path)))
        .await
        .map_err(|e| e.to_string())?;
    let (size_bytes, created_at) = stat
        .trim()
        .split_once('|')
        .and_then(|(s, t)| Some((s.parse().ok()?, t.parse().ok()?)))
        .ok_or_else(|| "Backup erstellt, aber Größe/Zeitstempel konnten nicht gelesen werden".to_string())?;

    Ok(BackupEntry { name: filename, size_bytes, created_at })
}

/// Module D: lists backups for an instance, newest first.
#[tauri::command]
async fn list_backups(state: State<'_, AppState>, server_id: String, instance_id: String) -> Result<Vec<BackupEntry>, String> {
    if instance_id.is_empty() || instance_id.contains('/') || instance_id.contains("..") {
        return Err(format!("Ungültige instance_id: {instance_id:?}"));
    }
    let backup_dir = format!("/home/gameserver/backups/{instance_id}");

    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();
    let raw = session
        .exec(&format!(
            "sudo find {backup_dir} -maxdepth 1 -name '*.tar.gz' -printf '%f|%s|%T@\\n' 2>/dev/null"
        ))
        .await
        .map_err(|e| e.to_string())?;

    let mut entries: Vec<BackupEntry> = raw
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '|');
            let name = parts.next()?.to_string();
            let size_bytes = parts.next()?.parse().ok()?;
            let created_at = parts.next()?.split('.').next()?.parse().ok()?;
            Some(BackupEntry { name, size_bytes, created_at })
        })
        .collect();
    entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(entries)
}

/// Module D: pulls a backup archive down over the SSH connection (no SFTP subsystem needed -
/// just `cat` read as raw bytes) and saves it under the app's local data folder, so the user
/// has an off-server copy without needing to know what SSH/SCP even is.
#[tauri::command]
async fn download_backup(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    server_id: String,
    instance_id: String,
    filename: String,
) -> Result<String, String> {
    validate_backup_filename(&filename)?;
    let remote_path = format!("/home/gameserver/backups/{instance_id}/{filename}");

    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();
    let bytes = session
        .exec_bytes(&format!("sudo cat {}", games::shell_single_quote(&remote_path)))
        .await
        .map_err(|e| e.to_string())?;

    let local_dir = local_backup_dir(&app, &instance_id)?;
    let local_path = local_dir.join(&filename);
    std::fs::write(&local_path, bytes).map_err(|e| e.to_string())?;

    Ok(local_path.to_string_lossy().to_string())
}

/// Module D: deletes a backup archive from the server.
#[tauri::command]
async fn delete_backup(state: State<'_, AppState>, server_id: String, instance_id: String, filename: String) -> Result<(), String> {
    validate_backup_filename(&filename)?;
    let remote_path = format!("/home/gameserver/backups/{instance_id}/{filename}");

    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();
    session
        .exec(&format!("sudo rm -f {}", games::shell_single_quote(&remote_path)))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Module D: restores a backup, completely replacing the instance's current install directory
/// contents with the archive's. Destructive - the frontend must confirm with the user before
/// calling this, same as delete_instance. Stops the service first (extracting into a running
/// game's files would corrupt it), wipes the directory, extracts, restarts.
#[tauri::command]
async fn restore_backup(
    state: State<'_, AppState>,
    server_id: String,
    instance_id: String,
    install_path: String,
    unit_name: String,
    filename: String,
) -> Result<(), String> {
    validate_instance_path(&install_path, &instance_id)?;
    validate_backup_filename(&filename)?;
    let backup_path = format!("/home/gameserver/backups/{instance_id}/{filename}");

    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();

    let _ = provisioning::control_instance(session, &unit_name, "stop").await;

    session
        .exec(&format!(
            "sudo -u gameserver bash -c \"find {} -mindepth 1 -delete && tar -xzf {} -C {}\"",
            games::shell_single_quote(&install_path),
            games::shell_single_quote(&backup_path),
            games::shell_single_quote(&install_path)
        ))
        .await
        .map_err(|e| e.to_string())?;

    provisioning::control_instance(session, &unit_name, "start")
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}

/// Every SFTP path must resolve inside `/home/gameserver` (the isolated account every game
/// runs as) - "Vollzugriff" means the whole gameserver tree across all instances/backups, not
/// literal root filesystem access, which stays off-limits from the app no matter what.
fn validate_sftp_path(path: &str) -> Result<(), String> {
    const ROOT: &str = "/home/gameserver";
    if path != ROOT && !path.starts_with(&format!("{ROOT}/")) {
        return Err("Pfad außerhalb des zulässigen Bereichs".to_string());
    }
    if path.contains("..") {
        return Err("Ungültiger Pfad".to_string());
    }
    Ok(())
}

/// Module E ("Vollzugriff"/SFTP): lists any directory under `/home/gameserver`, not just an
/// instance's own install/backups folder - for admins who want to poke around the whole tree
/// (shared mod caches, manual edits, orphaned files from a deleted instance, etc.).
#[tauri::command]
async fn sftp_list(state: State<'_, AppState>, server_id: String, path: String) -> Result<Vec<DirEntry>, String> {
    validate_sftp_path(&path)?;
    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();
    let raw = session
        .exec(&format!(
            "sudo find {} -mindepth 1 -maxdepth 1 -printf '%y|%f|%s\\n' 2>/dev/null",
            games::shell_single_quote(&path)
        ))
        .await
        .map_err(|e| e.to_string())?;

    let mut entries: Vec<DirEntry> = raw
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '|');
            let type_char = parts.next()?;
            let name = parts.next()?.to_string();
            let size_bytes = parts.next()?.parse().ok()?;
            Some(DirEntry { name, is_dir: type_char == "d", size_bytes })
        })
        .collect();
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(entries)
}

#[tauri::command]
async fn sftp_mkdir(state: State<'_, AppState>, server_id: String, path: String) -> Result<(), String> {
    validate_sftp_path(&path)?;
    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();
    session
        .exec(&format!(
            "sudo -u gameserver mkdir -p {}",
            games::shell_single_quote(&path)
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Refuses to ever delete `/home/gameserver` itself - every other path inside it is fair game,
/// same "everything below the isolated account, never the account root or above" rule as list.
#[tauri::command]
async fn sftp_delete(state: State<'_, AppState>, server_id: String, path: String) -> Result<(), String> {
    validate_sftp_path(&path)?;
    if path == "/home/gameserver" {
        return Err("Der Basisordner selbst kann nicht gelöscht werden".to_string());
    }
    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();
    // If a compromised game process planted a symlink pointing outside its own instance folder
    // (e.g. at a system file), `rm -rf` would happily follow it and delete the *target*, not
    // just the link itself - refuse to touch anything that isn't a real file/directory.
    let quoted = games::shell_single_quote(&path);
    session
        .exec(&format!(
            "if [ -L {quoted} ]; then echo GRIMMNETZ_SYMLINK_REFUSED; else sudo rm -rf {quoted}; fi"
        ))
        .await
        .map_err(|e| e.to_string())
        .and_then(|out| {
            if out.contains("GRIMMNETZ_SYMLINK_REFUSED") {
                Err("Verweigert: Ziel ist ein symbolischer Link, kein echter Ordner/Datei".to_string())
            } else {
                Ok(())
            }
        })
}

#[tauri::command]
async fn sftp_rename(state: State<'_, AppState>, server_id: String, from: String, to: String) -> Result<(), String> {
    validate_sftp_path(&from)?;
    validate_sftp_path(&to)?;
    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();
    session
        .exec(&format!(
            "sudo -u gameserver mv {} {}",
            games::shell_single_quote(&from),
            games::shell_single_quote(&to)
        ))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Downloads any file under the SFTP root to a location the user picks via a native save dialog
/// - unlike backups, there's no single sensible default folder for an arbitrary file.
#[tauri::command]
async fn sftp_download(state: State<'_, AppState>, server_id: String, path: String, local_path: String) -> Result<(), String> {
    validate_sftp_path(&path)?;
    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();
    let quoted = games::shell_single_quote(&path);
    // Same symlink guard as sftp_delete - a symlink here could otherwise read out any file the
    // `gameserver` account has access to (or, via a dangling/absolute link, attempt to read
    // outside the instance's own directory entirely), disguised as an innocuous save file.
    let is_symlink = session
        .exec(&format!("[ -L {quoted} ] && echo yes || echo no"))
        .await
        .map_err(|e| e.to_string())?;
    if is_symlink.trim() == "yes" {
        return Err("Verweigert: Ziel ist ein symbolischer Link, kein echter Ordner/Datei".to_string());
    }
    let bytes = session
        .exec_bytes(&format!("sudo cat {quoted}"))
        .await
        .map_err(|e| e.to_string())?;
    std::fs::write(&local_path, bytes).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn sftp_upload(
    state: State<'_, AppState>,
    server_id: String,
    remote_dir: String,
    local_path: String,
    on_progress: Channel<UploadEvent>,
) -> Result<(), String> {
    validate_sftp_path(&remote_dir)?;
    let filename = std::path::Path::new(&local_path)
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "Ungültiger Dateiname".to_string())?
        .to_string();
    let data = std::fs::read(&local_path).map_err(|e| e.to_string())?;
    let remote_path = format!("{remote_dir}/{filename}");

    let mut guard = acquire_session(&state, &server_id).await?;
    let session = guard.as_mut().unwrap();
    session
        .exec_with_stdin_progress(
            &format!("sudo -u gameserver tee {} > /dev/null", games::shell_single_quote(&remote_path)),
            &data,
            |bytes_sent, total_bytes| {
                let _ = on_progress.send(UploadEvent::Progress { bytes_sent, total_bytes });
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Module C: streams `journalctl -fu <unit>` live, line-by-line, to the frontend via a Tauri
/// Channel. Runs as a detached background task so the command returns immediately and the
/// UI thread is never blocked, even while the remote log keeps following indefinitely.
#[tauri::command]
async fn stream_instance_logs(
    state: State<'_, AppState>,
    server_id: String,
    unit_name: String,
    on_event: Channel<LogEvent>,
) -> Result<(), String> {
    // Held open indefinitely by the spawned task below (journalctl -f never returns), so it
    // gets its own dedicated connection instead of tying up the shared per-server pool slot.
    let mut session = connect_fresh(&state, &server_id).await?;

    tauri::async_runtime::spawn(async move {
        // -o cat drops journalctl's own date/host/pid prefix - most game servers (Paper
        // included) already print their own [HH:MM:SS LEVEL] timestamp in the message
        // itself, so this avoids showing the date/host/pid twice per line.
        let command = format!("journalctl -fu {unit_name} -n 200 --no-pager -o cat");
        let _ = session
            .exec_stream_lines(&command, |line| {
                let _ = on_event.send(LogEvent::Line { text: line });
            })
            .await;
        let _ = on_event.send(LogEvent::Closed);
    });

    Ok(())
}

#[derive(Serialize, Deserialize, Clone)]
pub struct AppSettings {
    #[serde(default)]
    pub close_to_tray: bool,
    #[serde(default = "default_refresh_interval_ms")]
    pub refresh_interval_ms: u32,
}

fn default_refresh_interval_ms() -> u32 {
    3000
}

impl Default for AppSettings {
    fn default() -> Self {
        Self { close_to_tray: false, refresh_interval_ms: default_refresh_interval_ms() }
    }
}

fn load_settings(path: &std::path::Path) -> AppSettings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> AppSettings {
    let close_to_tray = *state.close_to_tray.lock().unwrap();
    let mut settings = load_settings(&state.settings_path);
    settings.close_to_tray = close_to_tray;
    settings
}

#[tauri::command]
fn save_settings(state: State<'_, AppState>, settings: AppSettings) -> Result<(), String> {
    *state.close_to_tray.lock().map_err(|e| e.to_string())? = settings.close_to_tray;
    let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(&state.settings_path, json).map_err(|e| e.to_string())
}

#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data_dir)?;
            let db_path = app_data_dir.join("grimmnetz.db");
            let db_key = keyring_store::get_or_create_db_key()?;
            let db = Db::open(db_path, &db_key)?;
            let settings_path = app_data_dir.join("settings.json");
            let settings = load_settings(&settings_path);
            app.manage(AppState {
                db: Mutex::new(db),
                ssh_pool: Mutex::new(HashMap::new()),
                local_sys: Mutex::new(LocalSystemMonitor::new()),
                close_to_tray: Mutex::new(settings.close_to_tray),
                settings_path,
            });

            // Tray icon: left-click shows/focuses the window, menu offers an explicit quit -
            // the only way this app should ever exit unprompted once "In Tray minimieren" is on.
            use tauri::menu::{Menu, MenuItem};
            use tauri::tray::TrayIconBuilder;
            let show_item = MenuItem::with_id(app, "show", "Öffnen", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Beenden", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&show_item, &quit_item])?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&tray_menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => app.exit(0),
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Left,
                        button_state: tauri::tray::MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            if let Some(window) = app.get_webview_window("main") {
                let handle = app.handle().clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        let state = handle.state::<AppState>();
                        let close_to_tray = *state.close_to_tray.lock().unwrap();
                        if close_to_tray {
                            api.prevent_close();
                            if let Some(w) = handle.get_webview_window("main") {
                                let _ = w.hide();
                            }
                        }
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            quit_app,
            get_local_system_stats,
            add_server,
            prepare_key_only_server,
            finalize_pending_server,
            check_firewall_active,
            enable_firewall,
            update_server,
            list_servers,
            delete_server,
            reboot_server,
            get_hardware_stats,
            list_games,
            list_instances,
            start_install,
            list_active_installs,
            attach_install_stream,
            discover_instances,
            get_instance_version,
            get_minecraft_live_status,
            update_instance,
            get_instance_config,
            save_instance_config,
            control_instance,
            get_instance_status,
            get_host_capacity,
            update_instance_resources,
            list_scheduled_restarts,
            add_scheduled_restart,
            remove_scheduled_restart,
            broadcast_and_execute,
            delete_instance,
            forget_instance,
            list_directory,
            sftp_list,
            sftp_mkdir,
            sftp_delete,
            sftp_rename,
            sftp_download,
            sftp_upload,
            create_backup,
            get_local_backup_dir,
            upload_backup,
            list_backups,
            download_backup,
            delete_backup,
            restore_backup,
            stream_instance_logs,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
