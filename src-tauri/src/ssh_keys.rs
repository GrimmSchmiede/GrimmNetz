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
/// einträgt - läuft nach erfolgreichem Passwort-Login über die offene SSH-Session als genau
/// dieser Nutzer, deshalb ist `$HOME` die verlässliche Quelle für das Home-Verzeichnis (statt
/// `/home/{username}` zu raten) und ein `chown` unnötig (selbst angelegte Dateien gehören dem
/// Nutzer bereits; ein `chown user:user` würde auf Systemen mit abweichendem Primärgruppennamen
/// sogar fehlschlagen). Der `sed -i`-Aufruf entfernt zuerst eine eventuell vorhandene ältere
/// GrimmNetz-Zeile desselben Servers (Kommentar `grimmnetz-{server_id}`), damit bei einer
/// Neu-Generierung des Keys kein alter, nicht mehr widerrufbarer Public Key auf dem Server
/// zurückbleibt; `grep -qxF` verhindert zusätzlich Duplikate exakt gleicher Zeilen. Die
/// `chmod`-Aufrufe sind nötig, da SSH Keys in zu offen berechtigten `.ssh`-Verzeichnissen/
/// Dateien ignoriert. Der abschließende Marker `GRIMMNETZ_KEY_INSTALLED` wird nur ausgegeben,
/// wenn die gesamte Kette erfolgreich war - `exec` liefert keinen Exit-Code, der Aufrufer prüft
/// deshalb auf diesen Marker in der Ausgabe.
pub fn install_command(public_line: &str) -> String {
    let quoted = crate::games::shell_single_quote(public_line);
    // Kommentarfeld der Public-Key-Zeile (`grimmnetz-{server_id}`) - identifiziert die von
    // GrimmNetz verwaltete Zeile in `authorized_keys`.
    let comment = public_line.split_whitespace().last().unwrap_or_default();
    let sed_script = crate::games::shell_single_quote(&format!("\\@ {comment}$@d"));
    format!(
        "mkdir -p \"$HOME/.ssh\" && chmod 700 \"$HOME/.ssh\" && \
         touch \"$HOME/.ssh/authorized_keys\" && \
         sed -i {sed_script} \"$HOME/.ssh/authorized_keys\" && \
         (grep -qxF {quoted} \"$HOME/.ssh/authorized_keys\" 2>/dev/null || echo {quoted} >> \"$HOME/.ssh/authorized_keys\") && \
         chmod 600 \"$HOME/.ssh/authorized_keys\" && echo GRIMMNETZ_KEY_INSTALLED"
    )
}

/// Marker, den `install_command` bei Erfolg ausgibt - siehe dort.
pub const INSTALL_SUCCESS_MARKER: &str = "GRIMMNETZ_KEY_INSTALLED";
