# Docker als Install-/Betriebs-Grundlage - Pilot (Minecraft + Factorio)

Datum: 2026-08-11
Status: Approved (Design), noch nicht implementiert

## Problem

GrimmNetz installiert Gameserver aktuell direkt per SteamCMD/curl-Downloads
auf dem Host und betreibt sie als rohe systemd-Units. Das bringt zwei
strukturelle Probleme mit sich:

1. **Distro-Kompatibilität**: Jede Spiele-Installation hängt von den
   Bibliotheken/Paketen ab, die auf der jeweiligen Ziel-Distro verfügbar
   sind - SteamCMD-Eigenheiten, fehlende Pakete, Versionsunterschiede
   zwischen Debian/Fedora/etc. sind eine wiederkehrende Fehlerquelle.
2. **Keine Update-Story**: Es gibt aktuell keinen Mechanismus, um einen
   laufenden Gameserver zu aktualisieren - nur komplette Neuinstallation.

## Ziel

Docker ersetzt SteamCMD/direkte Downloads als Installationsgrundlage
**vollständig** (kein Parallelbetrieb, keine Nutzer aktuell betroffen).
Dieser Pilot deckt zwei Spiele mit unterschiedlichen Eigenschaften ab
(Minecraft: kein RCON, javabasiert; Factorio: RCON-basiert, bereits
funktionierendes Ansage/Neustart-Feature), um Designfehler früh
aufzudecken, bevor alle übrigen Spiele-Templates migriert werden.

**Out of Scope für diesen Pilot** (eigene, spätere Teilprojekte):
- Automatische Updates (Watchtower-artiger Mechanismus)
- Migration der übrigen Spiele-Templates (Palworld, DayZ, Satisfactory,
  Valheim, V Rising, SCUM, 7 Days to Die)
- Eigene Docker-Images bauen (nur relevant, falls für ein Spiel kein
  brauchbares Community-Image existiert - für Minecraft und Factorio ist
  das nicht der Fall)
- Interaktive Zwei-Wege-Konsole (bestehender Roadmap-Punkt, unabhängig)

## Architektur

Docker-basierte Templates laufen über dieselbe Decoupled-Install-Pipeline,
die im vorherigen Branch gebaut wurde (server-seitiges Skript als
systemd-oneshot-Dienst, Tail-Streaming fürs Fortschritts-UI,
Crash-Sicherheit, automatische Discovery) - nur der Inhalt des generierten
Skripts ändert sich: statt SteamCMD/curl-Downloads macht es `docker pull
<image>`, statt eines rohen Java-/Binary-Prozesses startet die
Game-systemd-Unit den Container per `docker run --network host -v
<instanz>:/data ...`.

- **Netzwerk**: Host-Networking (`--network host`) statt Bridge/Port-Mapping.
  Grund: kompatibel mit der bestehenden Firewall-Öffnen-Logik (ein Port ist
  ein Port, egal ob Container oder nativer Prozess), keine
  Port-Mapping-Konfiguration pro Spiel nötig, echte Netzwerk-Isolation ist
  ohnehin nicht der Haupttreiber dieses Umstiegs.
- **Persistenz**: Bind-Mount (`-v /home/gameserver/instances/<id>:/data`),
  nicht Docker-Volume - Weltdaten/Configs bleiben direkt auf dem
  Host-Dateisystem sichtbar, damit SFTP-Browser und Config-Editor
  unverändert funktionieren.
- **Dateibesitz**: Community-Images mit `PUID`/`PGID`-Unterstützung (u.a.
  `itzg/minecraft-server`) bekommen die UID/GID des bestehenden
  `gameserver`-Users übergeben - Dateien im Bind-Mount bleiben exakt so
  besitzrechtlich wie heute, kein neuer User/keine neue Rechtelogik nötig.
- **Bootstrap**: `bootstrap_server` bekommt einen neuen Schritt: Docker via
  `get-docker.sh` installieren, falls `command -v docker` fehlschlägt.

## games.json-Schema

Neue `install.type: "docker"`-Variante:

```json
"install": {
  "type": "docker",
  "image": "itzg/minecraft-server:latest",
  "docker_env": { "EULA": "TRUE", "TYPE": "PAPER", "MEMORY": "" },
  "pre_start_steps": []
}
```

- `image`: Docker-Image mit Tag - immer `:latest` in diesem Pilot (echtes
  Versions-Pinning ist Teil des späteren Auto-Update-Teilprojekts)
- `docker_env`: Umgebungsvariablen für `docker run -e KEY=VALUE ...`
- `pre_start_steps`: optionale Shell-Schritte, die vor dem ersten
  Containerstart auf dem Host laufen (z. B. Factorio braucht eine
  `server-settings.json`, bevor der Container sie einliest - identisch zum
  bestehenden Mechanismus, nur ohne den eigentlichen Spiele-Download-Step)

Konkrete Pilot-Templates:
- **Minecraft**: `itzg/minecraft-server:latest`, `EULA=TRUE`, `TYPE=PAPER`,
  RAM-Limit über `MEMORY`-Env-Var statt JVM-Flags im `start_command`.
- **Factorio**: `factoriotools/factorio:latest`, `pre_start_steps` erzeugt
  `server-settings.json` (identischer Inhalt wie heute) im Bind-Mount, RCON
  über `--rcon-port`/`--rcon-password`-Env-Variablen des Images.

## Systemd-Unit für Docker-Container

```
[Service]
Type=simple
ExecStartPre=-/usr/bin/docker rm -f grimmnetz-{instance_id}
ExecStart=/usr/bin/docker run --rm --name grimmnetz-{instance_id} \
  --network host \
  -v {install_path}:/data \
  -e PUID={gameserver_uid} -e PGID={gameserver_gid} \
  {docker_env_flags} \
  {image}
ExecStop=/usr/bin/docker stop -t 30 grimmnetz-{instance_id}
Restart=on-failure
```

- `ExecStartPre` (mit führendem `-`, Fehler stoppt den Start nicht) räumt
  einen verwaisten Container mit demselben Namen weg (z. B. nach hartem
  Absturz vor einem sauberen `docker rm`).
- `--rm`: Container wird beim Stoppen automatisch entfernt.
- `ExecStop` schickt sauberes `docker stop` (SIGTERM + Timeout) statt dass
  systemd nur den `docker run`-Client-Prozess killt und einen laufenden
  Container verwaist zurücklässt.
- `MemoryMax`/`CPUQuota`/etc. bleiben unverändert (wirken über cgroups
  genauso auf den `docker run`-Prozess und den davon gestarteten
  Container-Prozessbaum).
- Firewall-Port-Öffnen unverändert (Host-Networking, gleiche Logik wie
  heute).
- RCON (Factorio) bleibt unverändert nutzbar: `--network host` bedeutet
  `127.0.0.1:27015` ist exakt wie heute erreichbar, das bestehende
  Python-RCON-Skript über SSH braucht keine Anpassung.

## Fehlerbehandlung

- **Docker nicht installiert**: `bootstrap_server` installiert via
  `get-docker.sh`, bricht mit klarer Fehlermeldung ab, falls die
  Installation selbst scheitert (z. B. exotische Distro ohne Support).
- **Image-Pull schlägt fehl** (Netzwerk, falscher Tag, Docker-Hub
  Rate-Limit): landet als `GRIMMNETZ_FAILED:<meldung>` im bestehenden
  Install-Skript-Mechanismus, unverändert.
- **Container startet, crasht aber sofort** (falsche Env-Vars, kaputtes
  Image): `Restart=on-failure` greift wie bisher, Status zeigt sich über
  die bestehende Status-Abfrage genauso wie bei einem crashenden
  Java-Prozess.
- **Deinstallation**: `delete_instance` ergänzt `docker rm -f
  grimmnetz-{instance_id}` vor dem Entfernen der Unit-Datei, falls der
  Container aus irgendeinem Grund noch existiert.

## Testing / Verifikation

Live gegen den WSL-Testserver:
1. Minecraft komplett installieren, verbinden, Config über bestehenden
   Editor ändern, neu starten.
2. Factorio komplett installieren, RCON-Ansage/Neustart-Countdown testen
   (bereits gebautes Feature - prüft, ob es mit Docker-Networking noch
   greift).
3. Absturz-Test: App während `docker pull` schließen, prüfen dass der
   Install server-seitig weiterläuft und beim Wiederöffnen automatisch
   erkannt wird (identischer Flow wie im vorherigen Branch).
4. Deinstallieren, prüfen dass kein Container/keine Unit/keine
   Firewall-Regel übrig bleibt.

## Out of Scope (nochmal explizit)

- Automatische Updates (Watchtower) - eigenes Teilprojekt.
- Migration der übrigen Spiele-Templates - eigenes Teilprojekt, nachdem
  dieser Pilot sich bewährt hat.
- Eigene Docker-Images bauen - nur falls für ein späteres Spiel kein
  brauchbares Community-Image existiert.
- Interaktive Zwei-Wege-Konsole - unabhängiger Roadmap-Punkt.
