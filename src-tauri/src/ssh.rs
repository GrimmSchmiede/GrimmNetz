use anyhow::{anyhow, Result};
use russh::client::{self, Handle, Msg};
use russh::{Channel, ChannelMsg};
use russh_keys::key;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

/// Pause im Remote-Output, ab der ein Prompt als "fertig ausgegeben" gilt.
const QUIET_GAP: Duration = Duration::from_millis(700);

/// Obergrenze für den kompletten automatisierten Passwortwechsel - `wait_for_quiet` allein ist
/// unbegrenzt (ein dauerhaft nachplätschernder Banner würde die Pause nie erreichen), und das
/// Ganze läuft unter dem Per-Server-Verbindungslock.
const PASSWORD_CHANGE_TIMEOUT: Duration = Duration::from_secs(60);

/// Drains a channel's output until there's a pause of `QUIET_GAP` with no new data - used to
/// detect "the remote side just finished printing a prompt and is waiting for input" without
/// having to match the prompt's exact (locale-dependent) text. Returns once the pause is
/// observed, or immediately if the channel closes.
async fn wait_for_quiet(channel: &mut Channel<Msg>, transcript: &mut Vec<u8>) -> Result<()> {
    loop {
        match tokio::time::timeout(QUIET_GAP, channel.wait()).await {
            Ok(Some(ChannelMsg::Data { data })) => transcript.extend_from_slice(&data),
            Ok(Some(ChannelMsg::ExtendedData { data, .. })) => transcript.extend_from_slice(&data),
            Ok(Some(ChannelMsg::Eof)) | Ok(Some(ChannelMsg::Close)) | Ok(None) => return Ok(()),
            Ok(Some(_)) => {}
            Err(_) => return Ok(()), // timeout elapsed with no new data - prompt is ready
        }
    }
}

/// A hung TCP connect/handshake (e.g. remote not reachable yet, firewall dropping packets)
/// would otherwise block forever - and since connections are held behind a per-server lock,
/// one stuck attempt could block every future attempt too. Give up and let the caller retry.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Same idea for individual commands: if a pooled connection has gone silently dead (e.g.
/// the remote host disappeared without a clean TCP close), a channel.wait() can hang forever.
const EXEC_TIMEOUT: Duration = Duration::from_secs(20);

/// Trust-on-first-use host key pinning: `expected` is `None` on a server's very first-ever
/// connect (anything is accepted, then pinned by the caller) or `Some(fingerprint)` on every
/// connect after that. A mismatch is refused at the protocol level (`Ok(false)`) - russh then
/// fails the handshake - and recorded in `mismatch` so the caller can turn the otherwise-generic
/// handshake error into a clear "host key changed" warning instead of a confusing timeout/refusal.
struct ClientHandler {
    expected: Option<String>,
    observed: Arc<StdMutex<Option<String>>>,
    mismatch: Arc<StdMutex<bool>>,
}

#[async_trait::async_trait]
impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(&mut self, server_public_key: &key::PublicKey) -> Result<bool, Self::Error> {
        let fingerprint = server_public_key.fingerprint();
        *self.observed.lock().unwrap() = Some(fingerprint.clone());
        match &self.expected {
            None => Ok(true),
            Some(expected) if *expected == fingerprint => Ok(true),
            Some(_) => {
                *self.mismatch.lock().unwrap() = true;
                Ok(false)
            }
        }
    }
}

pub struct SshSession {
    handle: Handle<ClientHandler>,
    /// SHA-256 fingerprint of the host key this session actually connected with - the caller
    /// pins this on a server's first-ever connect (`expected_fingerprint: None`).
    pub host_fingerprint: String,
}

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

    /// Runs a single command to completion, returning combined stdout.
    pub async fn exec(&mut self, command: &str) -> Result<String> {
        let bytes = tokio::time::timeout(EXEC_TIMEOUT, self.exec_bytes_inner(command))
            .await
            .map_err(|_| anyhow!("Zeitüberschreitung beim Ausführen des Befehls (Verbindung tot?)"))??;
        Ok(String::from_utf8_lossy(&bytes).to_string())
    }

    /// Same as `exec`, but with a much longer timeout - for commands that legitimately take
    /// minutes (steamcmd downloads during game installation), where the 20s default would
    /// abort a perfectly healthy, still-running install.
    pub async fn exec_long(&mut self, command: &str) -> Result<String> {
        let bytes = tokio::time::timeout(Duration::from_secs(1800), self.exec_bytes_inner(command))
            .await
            .map_err(|_| anyhow!("Zeitüberschreitung beim Ausführen des Befehls (Verbindung tot?)"))??;
        Ok(String::from_utf8_lossy(&bytes).to_string())
    }

    /// Same as `exec`, but returns the raw output bytes instead of lossy-decoding them as
    /// UTF-8 - required for pulling down binary file content (e.g. `cat` on a backup archive)
    /// without corrupting it. Uses a longer timeout since files can take a while to transfer.
    pub async fn exec_bytes(&mut self, command: &str) -> Result<Vec<u8>> {
        tokio::time::timeout(Duration::from_secs(300), self.exec_bytes_inner(command))
            .await
            .map_err(|_| anyhow!("Zeitüberschreitung beim Ausführen des Befehls (Verbindung tot?)"))?
    }

    async fn exec_bytes_inner(&mut self, command: &str) -> Result<Vec<u8>> {
        let mut channel = self.handle.channel_open_session().await?;
        channel.exec(true, command).await?;

        let mut output = Vec::new();
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { data } => output.extend_from_slice(&data),
                ChannelMsg::ExtendedData { data, .. } => output.extend_from_slice(&data),
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }
        }
        Ok(output)
    }

    /// Same as `exec_with_stdin`, but writes the data in chunks and calls `on_progress` with
    /// (bytes_sent, total_bytes) after each one - used for uploads where the caller wants to
    /// show a real percentage instead of an indefinite spinner while a large file transfers.
    pub async fn exec_with_stdin_progress<F>(&mut self, command: &str, data: &[u8], mut on_progress: F) -> Result<String>
    where
        F: FnMut(u64, u64),
    {
        const CHUNK_SIZE: usize = 1024 * 1024;
        let mut channel = self.handle.channel_open_session().await?;
        channel.exec(true, command).await?;

        let total = data.len() as u64;
        let mut sent = 0usize;
        while sent < data.len() {
            let end = (sent + CHUNK_SIZE).min(data.len());
            channel.data(&data[sent..end]).await?;
            sent = end;
            on_progress(sent as u64, total);
        }
        channel.eof().await?;

        let mut output = Vec::new();
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { data } => output.extend_from_slice(&data),
                ChannelMsg::ExtendedData { data, .. } => output.extend_from_slice(&data),
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }
        }
        Ok(String::from_utf8_lossy(&output).to_string())
    }

    /// Runs a command, writing `stdin_data` to it right after starting (e.g. to answer
    /// a `sudo -S` password prompt) before reading the combined output to completion.
    pub async fn exec_with_stdin(&mut self, command: &str, stdin_data: &[u8]) -> Result<String> {
        let mut channel = self.handle.channel_open_session().await?;
        channel.exec(true, command).await?;
        channel.data(stdin_data).await?;
        channel.eof().await?;

        let mut output = Vec::new();
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { data } => output.extend_from_slice(&data),
                ChannelMsg::ExtendedData { data, .. } => output.extend_from_slice(&data),
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }
        }
        Ok(String::from_utf8_lossy(&output).to_string())
    }

    /// Cloud-Provider (u.a. Hetzner) markieren ein frisch vergebenes/zurückgesetztes
    /// root-Passwort als sofort abgelaufen - jeder nicht-interaktive Befehl (auch ein simpler
    /// `exec`) wird dann von sshd/PAM mit "Password change required but no TTY available"
    /// verweigert, nicht nur `sudo`. Es gibt keinen Weg, das ohne eine echte interaktive
    /// Terminal-Sitzung zu umgehen, also spielen wir hier den erzwungenen `passwd`-Dialog
    /// automatisiert durch: PTY anfordern, Shell starten, und die Prompts zeitbasiert (Pause im
    /// Output = nächster Prompt ist fertig) statt rein textbasiert bedienen.
    /// Ein administrator-erzwungener Wechsel (der hier vorliegende Fall) fragt nur "Neues
    /// Passwort" + "Wiederholung", KEIN "aktuelles Passwort" - anders als ein freiwilliger
    /// `passwd`-Aufruf. Da sich das je nach PAM-Konfiguration/Distro unterscheiden kann, wird die
    /// erste Ausgabe kurz auf "current"/"aktuell" geprüft, um zu entscheiden, ob das aktuelle
    /// Passwort zusätzlich gesendet werden muss - einziger Textbezug hier, alles andere bleibt
    /// zeitbasiert.
    pub async fn change_expired_password(&mut self, current_password: &str, new_password: &str) -> Result<()> {
        // Entfernt beide Klartext-Passwörter aus einem Transcript, bevor es in einer Fehlermeldung
        // (und damit bis in die UI) landet - bei einem Prompt-Versatz kann eine interaktive Shell
        // eine gesendete Zeile echoen.
        let redact = |text: &str| {
            let mut out = text.to_string();
            for secret in [new_password, current_password] {
                if !secret.is_empty() {
                    out = out.replace(secret, "[REDACTED]");
                }
            }
            out
        };
        let attempt = async {
        let mut channel = self.handle.channel_open_session().await?;
        channel
            .request_pty(false, "xterm", 80, 24, 0, 0, &[])
            .await
            .map_err(|e| anyhow!("PTY-Anforderung fehlgeschlagen: {e}"))?;
        channel
            .request_shell(false)
            .await
            .map_err(|e| anyhow!("Shell-Start fehlgeschlagen: {e}"))?;

        let mut transcript = Vec::new();
        wait_for_quiet(&mut channel, &mut transcript).await?;
        // Momentaufnahme AUSSCHLIESSLICH der Bytes vor der ersten gesendeten Zeile: die
        // Entscheidung fällt genau einmal und wird später nicht auf dem weiterwachsenden
        // Transcript neu ausgewertet.
        let pre = String::from_utf8_lossy(&transcript).to_lowercase();
        let asks_current_first = pre.contains("current") || pre.contains("aktuell");
        drop(pre);

        let lines: &[&str] = if asks_current_first {
            &[current_password, new_password, new_password]
        } else {
            &[new_password, new_password]
        };
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                // Wartet auf eine Sendepause im Output (der nächste Prompt ist fertig ausgegeben).
                wait_for_quiet(&mut channel, &mut transcript).await?;
            }
            channel
                .data(format!("{line}\n").as_bytes())
                .await
                .map_err(|e| anyhow!("Eingabe konnte nicht gesendet werden: {e}"))?;
        }
        // Letzte Ausgabe (Erfolg/Fehler-Meldung von `passwd`) noch abwarten, dann die Sitzung
        // schließen - der Server trennt die Verbindung nach einem erzwungenen Passwortwechsel
        // ohnehin selbst, ein neuer Connect mit dem neuen Passwort folgt direkt danach.
        let _ = wait_for_quiet(&mut channel, &mut transcript).await;
        // Auch die Erfolgsprüfung läuft auf dem redigierten Text: enthielte ein Passwort zufällig
        // einen dieser Teilstrings, würde ein Echo sonst fälschlich als Erfolg gelten.
        let output = redact(&String::from_utf8_lossy(&transcript)).to_lowercase();
        if output.contains("passwd: password updated successfully")
            // `passwd`-Präfix wie oben, nur die auf PAM-Systemen übliche Formulierung.
            || output.contains("passwd: all authentication tokens updated successfully")
            || output.contains("passwort erfolgreich")
        {
            return Ok(());
        }
        if output.contains("bad password")
            || output.contains("password unchanged")
            || output.contains("authentication token manipulation error")
        {
            return Err(anyhow!(
                "Erzwungener Passwortwechsel fehlgeschlagen - Server-Antwort: {}",
                redact(String::from_utf8_lossy(&transcript).trim())
            ));
        }
        // Kein eindeutiger Erfolgs-/Fehlertext gefunden (unbekannte PAM-Meldung) - optimistisch
        // weitermachen, der anschließende Connect mit dem neuen Passwort ist der eigentliche Test.
        Ok(())
        };
        match tokio::time::timeout(PASSWORD_CHANGE_TIMEOUT, attempt).await {
            Ok(result) => result,
            Err(_) => Err(anyhow!(
                "Zeitüberschreitung beim automatischen Passwortwechsel (Server antwortet nicht wie erwartet)"
            )),
        }
    }

    /// Runs a (potentially long-lived / follow-mode) command, invoking `on_line` for every
    /// complete line as data arrives, so the caller can forward it live (e.g. to the UI)
    /// instead of waiting for the whole process to finish.
    pub async fn exec_stream_lines<F>(&mut self, command: &str, on_line: F) -> Result<()>
    where
        F: FnMut(String) + Send,
    {
        self.exec_stream_lines_idle(command, None, on_line).await
    }

    /// Same as `exec_stream_lines`, but errors out if no data arrives for longer than
    /// `idle_timeout` - deliberately an IDLE timeout (reset on every message), not a cap on the
    /// call's total duration. A flat total-duration timeout would fire on every long-running
    /// stream regardless of whether it's actually stuck, forcing a reconnect every N seconds even
    /// while data keeps flowing fine - exactly the bug this replaced (a 30s total timeout around
    /// a multi-minute install tail was forcing a fresh reconnect roughly every 30s, the whole
    /// install through, misreported to the user as repeated "connection lost" drops).
    pub async fn exec_stream_lines_idle<F>(&mut self, command: &str, idle_timeout: Option<Duration>, mut on_line: F) -> Result<()>
    where
        F: FnMut(String) + Send,
    {
        let mut channel = self.handle.channel_open_session().await?;
        channel.exec(true, command).await?;

        let mut buffer = Vec::new();
        loop {
            let msg = match idle_timeout {
                Some(d) => match tokio::time::timeout(d, channel.wait()).await {
                    Ok(msg) => msg,
                    Err(_) => return Err(anyhow!("Zeitüberschreitung: keine Daten mehr empfangen (Verbindung tot?)")),
                },
                None => channel.wait().await,
            };
            let Some(msg) = msg else { break };
            match msg {
                ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                    buffer.extend_from_slice(&data);
                    // steamcmd redraws its download-progress line in place using '\r' instead
                    // of '\n' - treat both as a line boundary so progress updates stream out
                    // instead of piling up in the buffer until an unrelated '\n' shows up.
                    while let Some(pos) = buffer.iter().position(|&b| b == b'\n' || b == b'\r') {
                        let line: Vec<u8> = buffer.drain(..=pos).collect();
                        let text = String::from_utf8_lossy(&line).trim_end().to_string();
                        on_line(text);
                    }
                }
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }
        }
        if !buffer.is_empty() {
            on_line(String::from_utf8_lossy(&buffer).trim_end().to_string());
        }
        Ok(())
    }
}
