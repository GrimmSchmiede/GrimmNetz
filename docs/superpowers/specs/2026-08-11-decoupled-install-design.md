# Installationen von der App-Laufzeit entkoppeln

Datum: 2026-08-11
Status: Approved (Design), noch nicht implementiert

## Problem

Eine Spiele-Installation läuft aktuell über die offene SSH-Session direkt aus der
App (`install_game` in `src-tauri/src/lib.rs`, `exec_stream_lines`/`exec_long`).
Bricht die SSH-Verbindung ab (Internet-Aussetzer, PC-Absturz) oder wird die App
geschlossen, ist der Install-Fortschritt weg - der Instanzordner bleibt
halbfertig auf dem Server liegen, es gibt keinen Weg, den Install von der App
aus fortzusetzen oder sauber zu erkennen, was passiert ist. Nutzer der App
kennen sich per Definition nicht mit Linux/Konsole aus - "geh auf den Server
und schau nach" ist keine akzeptable Fehlerbehandlung.

Zusätzliche Anforderung aus dem Gespräch: die App speichert Server/Instanzen
aktuell nur lokal in SQLite. Zwei Nutzer (oder ein Nutzer an zwei PCs), die
denselben Server verwalten, sollen trotzdem beide denselben Installationsstatus
sehen können, auch wenn der Install auf dem jeweils anderen PC gestartet wurde.

## Ziel

Der eigentliche Installationsvorgang läuft komplett unabhängig von
App/SSH-Verbindung auf dem Server weiter (systemd-oneshot-Dienst). Die App
hängt sich nur noch als Beobachter an einen laufenden oder bereits
abgeschlossenen Install an - von jedem PC aus, der Zugriff auf denselben
Server hat, unabhängig von der lokalen DB.

## Architektur

Statt Install-Schritte direkt über die offene SSH-Session auszuführen,
generiert die App ein Bash-Skript aus den Template-Steps und startet es als
eigenständigen systemd-oneshot-Dienst (`grimmnetz-install-<instance_id>.service`)
auf dem Server. Das Skript schreibt fortlaufend strukturierte Statuszeilen in
eine Log-Datei im Instanzverzeichnis (`install.log`). Die App tailt diese Datei
per SSH (`tail -f`, derselbe Mechanismus wie das bestehende Live-Terminal /
`exec_stream_lines`) und parst sie zu denselben `InstallEvent`s wie heute
(Step/Progress).

Reales SteamCMD-Rohoutput landet unverändert mit im Log - `parse_steamcmd_progress`
bleibt unverändert, nur die Quelle wechselt von Live-Stdout zu einer Datei.

## Komponenten

### 1. Skript-Generator (`provisioning.rs`, neu)

Baut aus den Template-Steps (wiederverwendet: `games::render_step`) ein
Bash-Skript, das:
- pro Step eine Marker-Zeile `GRIMMNETZ_STEP n/total` ins Log schreibt
- den gerenderten Befehl ausführt, dessen Rohausgabe mit ins Log geht
  (SteamCMD-`progress:`-Zeilen bleiben client-seitig genauso parsbar wie heute)
- bei Fehler `GRIMMNETZ_FAILED:<meldung>` schreibt, den Instanzordner selbst
  aufräumt (`rm -rf`, gleiche Logik wie im heutigen Cleanup-Pfad) und abbricht
- am Ende `GRIMMNETZ_DONE` schreibt

### 2. `start_install` (neuer Tauri-Command)

Erzeugt `instance_id`, schreibt und startet den oneshot-Dienst, kehrt sofort
zurück - kein Warten auf Fertigstellung. Kein DB-Eintrag zu diesem Zeitpunkt
(Instanz existiert erst nach erfolgreichem Abschluss lokal).

### 3. `attach_install_stream` (neuer Tauri-Command)

Tailt `install.log` per `exec_stream_lines` ab Byte 0 (kurze Textdatei, erneutes
komplettes Lesen ist unproblematisch und bringt die UI beim Reconnect einfach
wieder auf den aktuellen Stand). Parst Zeilen wie heute zu `InstallEvent::Step`/
`InstallEvent::Progress`, bis:
- `GRIMMNETZ_DONE` kommt → baut `InstanceRecord` (Systemd-Unit existiert schon,
  gleiche Logik wie im heutigen Post-Install-Teil von `install_game`), trägt es
  in die lokale DB ein, gibt es zurück
- `GRIMMNETZ_FAILED:<meldung>` kommt → gibt die Meldung als Fehler zurück

### 4. `list_active_installs` (neuer Tauri-Command, discovery-basiert)

Scannt serverseitig `/home/gameserver/instances/*/install.log` (analog zum
bestehenden `discover_instances`-Muster) nach Logs ohne `DONE`/`FAILED`-Endzeile,
deren zugehöriger systemd-Dienst noch aktiv ist. Löst das Multi-PC-Problem, weil
serverseitig statt in der lokalen DB gesucht wird. Wird vom App-Store-Tab beim
Öffnen aufgerufen; laufende Installs erscheinen dort als Fortschrittskachel,
unabhängig davon, welcher PC sie gestartet hat.

## Frontend-Änderung

`install_game`-Aufruf wird ersetzt durch `start_install` gefolgt von
`attach_install_stream`. Bricht der Tail-Stream ab (SSH-Fehler), ruft das
Frontend automatisch erneut `attach_install_stream` auf ("Verbindung
unterbrochen, verbinde erneut...") - der Install selbst läuft davon unberührt
weiter.

## Fehlerbehandlung

- **SSH/App-Verbindung bricht während `tail -f` ab**: Fehler im Channel,
  Frontend zeigt Reconnect-Hinweis und ruft `attach_install_stream` erneut auf
  (bestehende Reconnect-Logik in `ssh.rs` greift). Install läuft unbeeinflusst
  weiter.
- **Skript-interner Fehler** (SteamCMD-Login fehlgeschlagen, Festplatte voll,
  ...): `GRIMMNETZ_FAILED:<meldung>` im Log, Instanzordner wird vom Skript
  selbst aufgeräumt, App zeigt die Fehlermeldung.
- **App bleibt tagelang zu, Server läuft/lief weiter**: beim nächsten Öffnen
  erkennt `list_active_installs` den noch laufenden oder bereits
  fertigen/fehlgeschlagenen Install und zeigt den passenden Zustand - kein
  stiller verwaister Zustand ohne Erklärung.

## Testing / Verifikation

Live gegen den WSL-Testserver:
1. Install starten, App während des SteamCMD-Downloads hart beenden -
   verifizieren, dass der systemd-Dienst weiterläuft und der Download
   fertig wird.
2. App neu starten, verifizieren dass Wiederandocken den korrekten Fortschritt
   zeigt und der Install sauber mit `DONE` abschließt (Instanz erscheint als
   Kachel).
3. Fehlerfall erzwingen (z. B. ungültige `app_id`), verifizieren dass
   `GRIMMNETZ_FAILED` korrekt ankommt, die Fehlermeldung im UI erscheint und
   der Instanzordner aufgeräumt wurde.

## Out of Scope

- Kein Sync der lokalen SQLite-DB zwischen mehreren PCs - `list_active_installs`
  löst nur den Install-in-Progress-Fall server-seitig; bereits abgeschlossene
  Instanzen weiterhin nur über die bestehende "Vorhandene Server suchen"-Funktion
  auf einem zweiten PC sichtbar.
- Keine Änderung an bereits laufenden Spiele-Instanzen (nur der Install-Vorgang
  selbst wird entkoppelt, nicht der laufende Gameserver-Betrieb).
