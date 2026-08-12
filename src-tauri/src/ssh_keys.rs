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
