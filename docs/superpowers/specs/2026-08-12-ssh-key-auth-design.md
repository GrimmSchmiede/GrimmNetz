# SSH-Key-Authentifizierung

## Problem

GrimmNetz verbindet sich aktuell ausschließlich per Passwort. Zwei Probleme:

1. Manche Cloud-Provider (u.a. Hetzner) erzwingen beim allerersten Passwort-Login
   eine Passwortänderung (`SSH_MSG_USERAUTH_PASSWD_CHANGEREQ`). Die verwendete
   SSH-Bibliothek `russh` 0.45 unterstützt dieses Protokoll-Feature clientseitig
   nicht - die Verbindung schlägt fehl bzw. hängt, ohne verständlichen Grund.
   Der User musste bisher manuell über die Provider-Web-Konsole das Passwort
   ändern, bevor GrimmNetz sich verbinden konnte.
2. Passwort-Auth ist grundsätzlich weniger sicher als Key-Auth.

## Lösung

Pro Server ein eigenes ED25519-Schlüsselpaar, Key-Auth wird gegenüber
Passwort-Auth bevorzugt. Zwei Wege, wie ein Server angelegt wird:

- **Weg A - Bestehender Server:** Unverändert wie heute per Passwort. Nach dem
  ersten erfolgreichen Passwort-Login generiert GrimmNetz automatisch einen Key
  für diesen Server und trägt ihn in `~/.ssh/authorized_keys` ein. Ab dem
  nächsten Connect wird Key-Auth verwendet, das Passwort bleibt nur als
  Fallback gespeichert. Für den User unsichtbar, kein Zutun nötig.
- **Weg B - Neuer Cloud-Server (empfohlen):** Key wird schon *vor* der
  VM-Erstellung generiert. User kopiert den Public Key ins Provider-Panel
  (z.B. Hetzners "SSH-Key hinzufügen" beim Server-Erstellen), trägt danach nur
  noch die IP nach. Erster Connect ist direkt Key-Auth - das Passwort-Zwang-
  Problem tritt gar nicht erst auf, da Hetzner die Passwortänderung nur bei
  reinem Passwort-Login erzwingt, nie bei Key-Login.

Weg B ist der empfohlene Standardpfad im UI, Weg A bleibt für Server, die
schon laufen oder deren Provider kein Key-Feld bei der Erstellung anbietet.

## Architektur

### Key-Erzeugung & Format

Neues Modul `ssh_keys.rs`:

- `generate_keypair(server_id: &str) -> Result<(KeyPair, String)>` - erzeugt
  ein ED25519-Paar über `russh_keys::key::KeyPair::generate_ed25519()` (bereits
  vorhandene Abhängigkeit, kein neuer Crate nötig). Der Public-Key-String wird
  über `PublicKeyBase64::public_key_base64()` extrahiert und als
  `ssh-ed25519 <base64> grimmnetz-{server_id}` formatiert (Kommentar-Suffix
  macht den Key auf dem Server identifizierbar).
- Private Key wird nach dem gleichen Muster wie das Passwort in
  `keyring_store.rs` gespeichert, eigener Eintrag pro Server
  (`{server_id}::ssh_key`, getrennt vom Passwort-Eintrag).

Pro-Server-Keys statt eines globalen App-Keys: Kompromittierung eines Servers
weitet sich nie auf andere aus (Blast-Radius-Prinzip), Mehraufwand ist minimal
da das `server_id`-Keyring-Muster schon existiert.

### Verbindungsablauf (`SshSession::connect`)

1. Ist ein Key für diesen Server im Keyring hinterlegt: `authenticate_publickey`
   versuchen.
2. Erfolgreich → fertig, Passwort wird nicht angerührt.
3. Kein Key vorhanden, oder Key-Auth schlägt fehl (z.B. Key wurde serverseitig
   aus `authorized_keys` entfernt): Fallback auf `authenticate_password`.
4. Nach erfolgreichem Passwort-Login: Wenn kein Key für diesen Server existiert
   ODER der vorhandene Key gerade fehlgeschlagen ist, neuen Key generieren
   und ausrollen (deckt sowohl Erstinstallation als auch "Key wurde auf dem
   Server gelöscht, automatisch neu ausrollen" ab).

### Ausrollen des Keys (Weg A)

Nach erfolgreichem Passwort-Login, eine kombinierte, idempotente Shell-Pipeline
über die bereits offene SSH-Session:

```bash
mkdir -p ~/.ssh && chmod 700 ~/.ssh && \
grep -qxF "<pubkey>" ~/.ssh/authorized_keys 2>/dev/null || echo "<pubkey>" >> ~/.ssh/authorized_keys && \
chmod 600 ~/.ssh/authorized_keys
```

`grep -qxF` verhindert Duplikate bei wiederholten Verbindungsversuchen (z.B.
nach einem App-Absturz zwischen Ausrollen und Speichern des Erfolgsstatus).
Die `chmod`-Aufrufe sind nötig, da SSH Keys in zu offen berechtigten
`.ssh`-Verzeichnissen/Dateien ignoriert.

### UI-Flow (Weg B)

"Server hinzufügen"-Dialog bekommt zwei Tabs:

- **"Bestehender Server"**: heutiger Passwort-Dialog, unverändert.
- **"Neuer Cloud-Server" (empfohlen, Standard-Tab)**: Generiert sofort einen
  Key im Backend (noch ohne Server-DB-Eintrag), zeigt den Public-Key-String
  mit Kopieren-Button und einer kurzen 3-Schritte-Anleitung ("Hetzner Cloud
  öffnen → Key beim Server-Erstellen einfügen → IP hier eintragen"). Das
  IP-Feld ist zunächst leer/optional; der Server-Eintrag bleibt im Zustand
  "Warte auf IP", bis der User sie nachträgt und die Verbindung erstmalig
  getestet wird.

### DB-Schema

`ip_address` in der `servers`-Tabelle wird nullable (aktuell vermutlich
`NOT NULL`) - Weg B legt einen Server-Eintrag an, bevor die IP bekannt ist.

## Fehlerfälle

- `.ssh`-Verzeichnis fehlt → wird durch `mkdir -p` in der Ausroll-Pipeline
  automatisch angelegt.
- Key-Ausrollen schlägt fehl (z.B. Festplatte voll, keine Schreibrechte) →
  Passwort-Auth bleibt weiterhin nutzbar, Key-Ausrollen wird beim nächsten
  Connect erneut versucht, kein harter Fehler für den User.
- Weg B, VM noch nicht erreichbar wenn IP eingetragen wird → normales
  Verbindungsfehler-Handling wie heute, kein Sonderfall nötig.

## Out of Scope

- Kein manuelles Importieren eigener/vorhandener SSH-Keys durch den User in
  dieser Iteration - nur App-generierte, App-verwaltete Keys.
- Kein UI zum Anzeigen/Rotieren/Löschen von Keys durch den User - passiert
  vollautomatisch im Hintergrund.
