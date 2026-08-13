# 7 Days to Die: Docker-Migration

## Problem

GrimmNetz betreibt Spiele-Server aktuell hybrid: Minecraft und Factorio laufen
über Docker (siehe `docs/superpowers/specs/2026-08-11-docker-migration-pilot-design.md`),
alle anderen Spiele - inklusive 7 Days to Die - noch über den ursprünglichen
SteamCMD-/nativen-Prozess-Pfad. Ziel ist die schrittweise Ablösung des alten
Pfads, bis er komplett entfernt werden kann. 7 Days to Die ist der erste
Kandidat: bereits end-to-end getestet (geringes Risiko einer Regression) und
mit `vinanrra/Docker-7DaysToDie` existiert ein aktiv gepflegtes, verbreitetes
Docker-Image (1M+ Pulls).

## Lösung

7DTD bekommt einen neuen `install.type: "docker"`-Eintrag in `games.json`,
analog zu Minecraft/Factorio, mit einer wichtigen strukturellen Erweiterung:
Das gewählte Image (`vinanrra/7dtd-server`) verlangt mehrere getrennte
Volume-Mounts (Weltdaten, Server-Config, Logs, Backups) statt eines einzigen
`/data`-Mounts wie bei den bisherigen Docker-Spielen. Das bestehende
Docker-Schema unterstützt aktuell nur einen Bind-Mount pro Container - wird
dafür auf mehrere Mounts verallgemeinert.

Broadcast-Ansagen (Neustart-Countdown etc.) sind **explizit außerhalb des
Scopes** dieser Migration - 7DTDs Docker-Image nutzt Telnet statt des
bestehenden Source-RCON-Mechanismus (`RCON_PY` in `lib.rs`), das ist ein
eigenständiges Folge-Feature, sobald der reine Docker-Umstieg steht und
läuft.

## Architektur

### Multi-Mount-Unterstützung (Schema-Erweiterung)

`GameInstall.container_mount: String` (aktuell ein einzelner Pfad, Default
`/data`) wird durch `GameInstall.mounts: Vec<DockerMount>` ersetzt:

```rust
#[derive(Serialize, Deserialize, Clone)]
pub struct DockerMount {
    /// Unterordner relativ zum Instanz-Install-Pfad auf dem Host. Leerer
    /// String heißt "der Install-Pfad selbst" (deckt das bisherige
    /// Ein-Mount-Verhalten von Minecraft/Factorio ab).
    #[serde(default)]
    pub subdir: String,
    pub container_path: String,
}
```

Rückwärtskompatibilität: Bestehende `games.json`-Einträge mit dem alten
`container_mount`-Feld werden beim Laden (oder per einmaliger manueller
Migration der bestehenden Minecraft/Factorio-Einträge auf das neue
`mounts`-Feld mit einem einzigen Eintrag `{subdir: "", container_path: "/data"}`
bzw. `{subdir: "", container_path: "/factorio"}`) auf das neue Format
überführt - da `games.json` eine mitgelieferte Ressourcendatei ist (kein
User-Content), ist eine einmalige Migration der bestehenden Einträge im
selben Commit der sauberere Weg als ein Kompatibilitäts-Codepfad für ein
Format, das nirgendwo mehr existiert, sobald der Commit gemerged ist.

`render_docker_systemd_unit` baut aus `mounts` mehrere `-v`-Flags:

```
-v {install_path}/{subdir1}:{container_path1} -v {install_path}/{subdir2}:{container_path2} ...
```

(bzw. nur `-v {install_path}:{container_path}` wenn `subdir` leer ist).

### `games.json`-Eintrag für 7DTD (Docker-Variante)

```json
{
  "id": "7dtd",
  "name": "7 Days to Die",
  "subtitle": "Dedicated Server",
  "icon": "7daystodie.png",
  "requires": [],
  "tested_on": [],
  "install": {
    "type": "docker",
    "image": "vinanrra/7dtd-server:latest",
    "mounts": [
      { "subdir": "world",       "container_path": "/home/sdtdserver/.local/share/7DaysToDie/" },
      { "subdir": "serverfiles", "container_path": "/home/sdtdserver/serverfiles/" },
      { "subdir": "logs",        "container_path": "/home/sdtdserver/log/" },
      { "subdir": "backups",     "container_path": "/home/sdtdserver/lgsm/backup/" }
    ],
    "docker_env": {
      "TimeZone": "Europe/Berlin"
    },
    "pre_start_steps": []
  },
  "start_command": "",
  "default_cpu_limit_percent": 200,
  "default_ram_limit_mb": 6144,
  "config": {
    "file": "serverfiles/sdtdserver.xml",
    "format": "xml-properties",
    "fields": [ /* wie bisher - ServerName, ServerPort, GameMode, MaxSpawnedZombies, LandClaimSize, ServerMaxPlayerCount, SandboxCode */ ]
  },
  "ports": [
    { "port": 26900, "protocol": "tcp" },
    { "port": 26900, "protocol": "udp" }
  ]
}
```

Der `TimeZone`-Wert wird - wie bei den bestehenden Docker-Spielen die
Zeitzonen-Behandlung schon läuft - aus der App heraus mit dem tatsächlichen
IANA-Bezeichner befüllt, nicht hartkodiert; das JSON oben zeigt nur die
Feldstruktur.

`docker_env` bleibt bewusst minimal - `START_MODE` wird NICHT gesetzt
(Default des Images verwendet, da `START_MODE=0` einen bekannten
Endlos-Neustart-Bug unter Orchestrierung hat; der tatsächlich richtige Wert
für unseren systemd-Betrieb wird beim ersten Live-Test anhand der Logs
verifiziert, nicht blind vorab festgelegt).

Das alte `install.type: "steamcmd"`-Feld/`start_command`/`requires:
["steamcmd"]` entfällt komplett für 7DTD - kein Parallelbetrieb beider
Varianten, gleiches Muster wie beim Minecraft/Factorio-Umstieg (dort wurde
der alte Pfad auch direkt ersetzt, nicht doppelt gepflegt).

### Config-Feldschema

Bleibt inhaltlich wie beim bisherigen nativen Eintrag (`ServerName`,
`ServerPort`, `GameMode`, `MaxSpawnedZombies`, `LandClaimSize`,
`ServerMaxPlayerCount`, `SandboxCode`) - nur der `file`-Pfad ändert sich auf
`serverfiles/sdtdserver.xml`. Das exakte XML-Schema (Attributnamen,
eventuell leicht abweichende Struktur ggü. der bisherigen `serverconfig.xml`)
wird gegen die tatsächlich vom Image generierte Datei verifiziert, sobald ein
Testinstall läuft - `xml-properties` als Parse-Format bleibt vermutlich
gültig (LinuxGSM-Konfigs nutzen dasselbe `<property name="X" value="Y"/>`-
Format wie die bisherige `serverconfig.xml`), wird aber nicht blind
angenommen.

## Fehlerfälle

- Folgt dem etablierten Muster aus der Docker-Pilot-Migration: Crashes durch
  Schema-Mismatches im generierten Default-Config werden über echte
  `journalctl`-Logs auf einem Testserver diagnostiziert, nicht spekulativ im
  Voraus gefixt.
- `mounts`-Unterordner werden vom `chown gameserver:gameserver`-Schritt der
  Install-Skript-Generierung (bereits vorhanden für den Root-Install-Pfad)
  weiterhin als Teil des rekursiven `install_path`-Chowns erfasst, da sie
  Unterordner davon sind - kein zusätzlicher Schritt nötig.

## Out of Scope

- Telnet-basierte Broadcast-Ansagen (Neustart-Countdown) - eigenständiges
  Folge-Feature.
- Entfernen des alten SteamCMD-Pfads aus dem Code - passiert erst, wenn alle
  Spiele migriert sind.
- Migration weiterer Spiele (Palworld, DayZ, ...) - eigene Iterationen.
