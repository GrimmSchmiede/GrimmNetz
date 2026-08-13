# Startup-Fortschrittsanzeige statt sofortigem "Online"

## Problem

`get_instance_status` (in `lib.rs`) meldet eine Instanz als "running"/"Online",
sobald `systemctl is-active` für die systemd-Unit `active` zurückgibt - also
sobald der Prozess bzw. Docker-Container gestartet ist. Bei Docker-Spielen,
die nach dem Containerstart selbst noch etwas herunterladen, verifizieren oder
eine Welt generieren (z.B. `vinanrra/7dtd-server` für 7 Days to Die - im
Live-Test dieser Session beobachtet: Weltgenerierung dauert nach Containerstart
noch 1-2 Minuten), ist das irreführend. Der User sieht "Online" und versucht
zu verbinden, obwohl der eigentliche Spiel-Server-Prozess noch nicht bereit
ist - das wirkt wie ein Fehler in der App, ist aber nur fehlende Information.

## Lösung

Pro Spiel eine optionale, geordnete Liste von "Startup-Meilensteinen" -
Textmuster, die in den letzten Docker-Log-Zeilen erscheinen, sobald eine
bestimmte Phase des Starts erreicht ist, jeweils mit einer zugeordneten
Prozentzahl. `get_instance_status` durchsucht bei aktiver Unit die Logs nach
dem am weitesten fortgeschrittenen erkannten Meilenstein und meldet dessen
Prozentwert zurück, solange kein 100%-Meilenstein gefunden wurde. Das
Frontend zeigt in diesem Fall einen Ladebalken mit Prozentangabe statt des
"Online"-Badges.

Nur 7 Days to Die bekommt in dieser Iteration konkrete Meilensteine (echte,
in dieser Session live beobachtete Logzeilen als Kalibrierungsgrundlage).
Andere Spiele bleiben unverändert beim bisherigen sofortigen "Online"-Verhalten
- das Schema ist aber bewusst generisch gehalten, sodass jedes weitere Spiel
später eigenständig um seine eigene Meilenstein-Liste ergänzt werden kann,
ohne Code-Änderungen an `get_instance_status` oder dem Frontend.

## Architektur

### `games.rs`

```rust
#[derive(Serialize, Deserialize, Clone)]
pub struct StartupMilestone {
    /// Text, der als Teilstring in einer Docker-Log-Zeile gesucht wird (keine Regex - einfache
    /// Substring-Suche reicht für die bisher beobachteten Logformate und ist robuster gegen
    /// harmlose Formatierungsunterschiede zwischen Spiel-/Image-Versionen).
    pub pattern: String,
    /// Fortschritt in Prozent, sobald `pattern` in den Logs auftaucht. Der höchste erreichte
    /// Meilenstein gewinnt, auch wenn ältere mit niedrigerem Prozentwert noch in den zuletzt
    /// gescannten Log-Zeilen sichtbar sind.
    pub percent: u8,
}
```

`GameTemplate` bekommt ein neues Feld `startup_milestones: Vec<StartupMilestone>`
mit `#[serde(default)]` - leer für jedes Spiel außer 7DTD, kein
Verhaltensunterschied für sie.

### `lib.rs` - `get_instance_status`

Nach dem bestehenden `systemctl is-active`-Aufruf: wenn der Zustand `active`
ist UND das Spiel-Template (nachgeschlagen über die Instanz's `game_id`)
nicht-leere `startup_milestones` hat UND noch kein Meilenstein mit
`percent == 100` bereits als erreicht gilt (siehe unten), zusätzlich
`docker logs --tail 200 <unit_name>` ausführen, für jeden Meilenstein prüfen
ob sein `pattern` als Substring vorkommt, und den höchsten `percent`-Wert
unter den gefundenen Treffern zurückgeben. Kein Treffer → `startup_percent:
None` (Frontend zeigt normales "Online", genau wie bisher - deckt sowohl
"Spiel ohne Meilensteine" als auch "letzte 200 Zeilen enthalten aus welchem
Grund auch immer keinen Treffer" ab, kein Sonderfall nötig).

`InstanceStatus` bekommt ein neues Feld `startup_percent: Option<u8>`.

Best-effort: schlägt der zusätzliche `docker logs`-Aufruf fehl (Timeout,
Container weg, etc.), wird das best-effort ignoriert und `startup_percent:
None` zurückgegeben - der bestehende `state`/`uptime_seconds`/`pid`-Teil der
Antwort bleibt davon unberührt, kein Fehler für den ganzen Command.

### Frontend

Wo auch immer `InstanceStatus` aktuell zur "Online"/"Offline"-Badge-Anzeige
führt: wenn `startup_percent` gesetzt UND `< 100`, statt des Online-Badges
einen Ladebalken mit "Startet... {percent}%" anzeigen. `startup_percent ==
Some(100)` oder `None` bei aktivem Zustand → normales "Online"-Badge wie
bisher.

### 7DTD-Meilensteine (konkrete Werte)

Aus den in dieser Session live beobachteten Logzeilen (Docker-Container-Boot
bis zum ersten erfolgreichen Spieler-Connect) abgeleitet, chronologisch:

| Prozent | Log-Textmuster (Substring) |
|---|---|
| 10 | `Starting periodic command scheduler cron` |
| 25 | `createWorld:` |
| 40 | `Started thread ChunkRegeneration` |
| 55 | `Started thread GenerateChunks` |
| 70 | `Calculating world hashes` |
| 85 | `GameServer.LogOn successful` |
| 100 | `StartGame done` |

## Fehlerfälle

- `docker logs`-Aufruf schlägt fehl: best-effort, `startup_percent: None`,
  Rest der Statusabfrage bleibt unbeeinflusst (siehe oben).
- Instanz ist nicht Docker-basiert (`install.type != "docker"`): hat ohnehin
  nie `startup_milestones` gesetzt (nur bei 7DTD in dieser Iteration), also
  kein Sonderfall in `get_instance_status` nötig - die leere Liste sorgt
  automatisch für unverändertes Verhalten.
- Spiel-Prozess crasht während der Startphase, bevor ein 100%-Meilenstein
  erreicht wurde, und die systemd-Unit zeigt weiterhin `active` (Docker
  `--rm`+`Restart=on-failure` startet neu): der Ladebalken bleibt einfach auf
  dem zuletzt erreichten Prozentwert stehen bzw. springt beim Neustart wieder
  auf einen niedrigeren Wert zurück, sobald neue Logzeilen den alten
  Fortschritt aus den letzten 200 Zeilen verdrängen - kein explizites
  Crash-Handling nötig, das Verhalten ergibt sich korrekt aus dem
  Log-Scan-Mechanismus selbst.

## Out of Scope

- Meilensteine für andere Spiele (Minecraft, Factorio, 7DTD-Vorgänger-Weg,
  ...) - eigene Folge-Iterationen pro Spiel, sobald reale Boot-Logs dafür
  vorliegen.
- Ein generischerer/automatischer Weg, Meilensteine ohne manuell beobachtete
  Logzeilen zu ermitteln - nicht in Scope, jedes Spiel wird einzeln anhand
  echter Logs kalibriert.
