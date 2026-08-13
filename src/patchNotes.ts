export type PatchNote = {
  version: string;
  date: string;
  items: string[];
};

// Maintained by hand alongside each release - keep entries short and user-facing,
// newest first. Not every internal commit needs its own entry.
export const PATCH_NOTES: PatchNote[] = [
  {
    version: "0.1.46",
    date: "2026-08-13",
    items: [
      "Docker-Unterstützung intern erweitert: Ein Container kann jetzt mehrere getrennte Verzeichnisse mounten statt nur eines - Grundlage für künftige Docker-Spiele mit komplexeren Datenstrukturen",
    ],
  },
  {
    version: "0.1.45",
    date: "2026-08-13",
    items: [
      "Neu: SSH-Key-Authentifizierung - GrimmNetz generiert automatisch einen eigenen Schlüssel pro Server und wechselt nach dem ersten Login von Passwort auf Key-Auth, ohne dass du etwas tun musst",
      "'Neuer Cloud-Server'-Modus beim Hinzufügen: Key vorab erzeugen und beim Server-Erstellen direkt beim Provider (z.B. Hetzner) hinterlegen - umgeht damit Anbieter, die beim ersten Passwort-Login eine Passwortänderung erzwingen, komplett",
      "Bugfix: Server mit erzwungenem Passwort-Ablauf (z.B. frisch gemietete Hetzner-Server) konnten bisher nur über die Provider-Webkonsole eingerichtet werden - läuft jetzt automatisch und vollständig über GrimmNetz",
      "Bugfix: Server-Einrichtung konnte bei einem frischen Server mit knappem Zeitlimit abbrechen (z.B. bei apt-get-Updates) - Zeitlimit für diese Schritte deutlich erhöht",
    ],
  },
  {
    version: "0.1.44",
    date: "2026-08-12",
    items: [
      "Minecraft (Paper) und Factorio laufen jetzt über Docker statt SteamCMD/direktem Prozess - läuft dadurch zuverlässig über verschiedene Linux-Distributionen hinweg (getestet Ubuntu 24.04 & 26.04)",
      "Factorio ist neu als Stable- und Experimental-Variante wählbar (2.0 vs. Space-Age-Zweig)",
      "Bugfix: Server-Nachricht bei geplantem/manuellem Neustart kam bei Minecraft nicht an - läuft jetzt wie bei Factorio über RCON statt stdin",
    ],
  },
  {
    version: "0.1.43",
    date: "2026-08-11",
    items: [
      "Spiele-Installationen laufen jetzt server-seitig als eigenständiger Dienst statt über die offene App-Verbindung - App-Absturz oder Verbindungsabbruch während eines Downloads zerstört den Install nicht mehr",
      "Laufende oder gerade fertig gewordene Installationen werden beim Öffnen eines Servers automatisch erkannt und angezeigt, kein manuelles Suchen mehr nötig",
      "Diverse Verbindungsstabilitäts-Fixes rund um Reattach/Fortschrittsanzeige nach längerem Live-Testing",
    ],
  },
  {
    version: "0.1.42",
    date: "2026-08-11",
    items: [
      "Factorio als getestet markiert: Install, Config (Name/Slots/Passwort), Neustart und LAN-Verbindung end-to-end verifiziert",
      "Bugfix: Server-Nachricht bei geplantem/manuellem Neustart kam bei Factorio nicht an - Factorio liest kein stdin zuverlässig, Ansagen laufen dort jetzt über RCON",
      "Neustart-Countdown erweitert: automatische Zwischenansagen (15/10/5/4/3/2/1 Min, 30 Sek, letzte 10 Sekunden einzeln) bei jedem Neustart/Stop mit Ansage, unabhängig vom Spiel; erzwungenes Speichern direkt vor dem Neustart bei Spielen mit RCON-Unterstützung",
    ],
  },
  {
    version: "0.1.41",
    date: "2026-08-11",
    items: [
      "Neu: Einstellungen-Seite mit Autostart (App startet mit Windows), 'In Tray minimieren' (Schließen legt die App nur ins Tray statt sie zu beenden) und einstellbarem Aktualisierungs-Intervall (1/2/5 Sek.)",
      "System-Tray-Icon mit Öffnen/Beenden-Menü",
      "Design-Feintuning: neutralere Grautöne statt Blaustich, Rahmen an mehreren Karten entfernt",
    ],
  },
  {
    version: "0.1.40",
    date: "2026-08-09",
    items: [
      "App-Store komplett überarbeitet: läuft jetzt direkt unter 'Installierte Gameserver' statt als eigenes Fenster, 3-Spalten-Kachelansicht, Suche, eigener Scrollbereich",
      "Neu: Live-Versionsanzeige pro Spiel (echte Steam-Build-Nummer bzw. aktuelle Version von Minecraft/Factorio) statt statischem Text",
      "Getestet-Status jetzt als Text + Farbpunkt (Grün/Gelb) mit Erklärung beim Hovern, statt nur einem Symbol",
    ],
  },
  {
    version: "0.1.39",
    date: "2026-08-09",
    items: [
      "Fix: Bricht die Verbindung genau während des Aufräumens nach einer fehlgeschlagenen Installation ab, blieb der halbfertige Ordner liegen statt gelöscht zu werden - App verbindet jetzt notfalls neu, um trotzdem aufzuräumen",
    ],
  },
  {
    version: "0.1.38",
    date: "2026-08-09",
    items: [
      "Sicherheit: Gameserver-Instanzen laufen jetzt in einer systemd-Sandbox (ProtectSystem=strict) - das gesamte Dateisystem außer dem eigenen Instanz-Ordner ist für den Prozess schreibgeschützt, verhindert dass eine kompromittierte Instanz Nachbar-Server manipuliert",
      "Sicherheit: SFTP-Vollzugriff verweigert jetzt Lösch-/Lese-Operationen auf symbolische Links (verhindert, dass ein manipulierter Symlink auf Systemdateien zeigt)",
    ],
  },
  {
    version: "0.1.37",
    date: "2026-08-09",
    items: [
      "Sicherheit: SSH-Host-Key wird jetzt beim ersten Verbinden gespeichert und bei jeder weiteren Verbindung geprüft (verhindert Man-in-the-Middle-Angriffe) - vorher wurde jeder Server-Key ungeprüft akzeptiert",
      "Neu: fail2ban wird automatisch auf neuen Servern eingerichtet (SSH-Login wird nach 3 falschen Passwort-Versuchen für 1 Stunde gesperrt)",
      "Fix: Derselbe Server konnte doppelt hinzugefügt werden, was zu Datenbankfehlern beim Server-Scan führte - wird jetzt vorher abgefangen",
    ],
  },
  {
    version: "0.1.36",
    date: "2026-08-09",
    items: [
      "Neu: App-Store zeigt jetzt an, welche Spiele wirklich getestet wurden (✅-Haken) statt nur 'sollte funktionieren'",
      "README: Tabelle mit getesteten Spielen inkl. Distro ergänzt",
    ],
  },
  {
    version: "0.1.35",
    date: "2026-08-09",
    items: [
      "Fix: Firewall-Erfolgsmeldung zeigte weiterhin '⚠️ Server ungeschützt' als Überschrift statt '🛡️ Server jetzt geschützt'",
    ],
  },
  {
    version: "0.1.34",
    date: "2026-08-09",
    items: [
      "Fix: 'Firewall aktivieren' schlug immer fehl ('SSH-Port nicht gefunden') - die Prüfung nutzte 'ufw status', das im inaktiven Zustand nie Regeln anzeigt, egal ob sie existieren",
    ],
  },
  {
    version: "0.1.33",
    date: "2026-08-09",
    items: [
      "Fix: 'Komplett deinstallieren' ließ geplante Neustarts der gelöschten Instanz aktiv - liefen ewig weiter und versuchten einen nicht mehr existierenden Dienst neu zu starten",
    ],
  },
  {
    version: "0.1.32",
    date: "2026-08-09",
    items: [
      "Vollständiger Rebrand-Abschluss: App-ID, Zugangsdaten-Speicher und lokale Datenbank laufen jetzt komplett unter GrimmNetz statt GlimaNexus",
      "Wichtig: Neuinstallation nötig, bestehende Server müssen einmalig neu eingerichtet werden (Passwörter neu eingeben)",
      "Systemd-Dienste neuer Installationen heißen jetzt grimmnetz-* statt novanexus-*",
      "Diverse Restnamen aus dem alten Branding entfernt (Platzhaltertexte, Dateinamen auf dem Server)",
    ],
  },
  {
    version: "0.1.31",
    date: "2026-08-09",
    items: [
      "Neu: Vollzugriff (SFTP) - Dateibrowser für den kompletten Server-Ordner (hoch-/runterladen, löschen, Ordner anlegen), erreichbar über das ⋯-Menü eines Servers",
      "Fix: Rahmen/Trennlinien im Türkis-Design angepasst, Konsole jetzt tiefschwarz",
      "Instanz-Metadatei heißt jetzt .grimmnetz-instance.json statt .glimanexus-instance.json (Rebrand-Rest)",
    ],
  },
  {
    version: "0.1.30",
    date: "2026-08-09",
    items: [
      "Neu: Reiter 'Verwaltung' pro Gameserver - CPU/RAM-Limits mit Empfehlung nach Spieleranzahl, Overcommit-Warnung",
      "Neu: Geplante tägliche Neustarts mit automatischer In-Game-Countdown-Ansage (15/10/5/4/3/2/1 Min)",
      "Neu: Neustart/Stop mit eigener Ansage an die Spieler und wählbarer Vorlaufzeit",
      "Neu: Ressourcen-Limits (CPU/RAM) werden jetzt vom Betriebssystem selbst durchgesetzt, nicht mehr nur angezeigt",
      "Status-Seite: Logs jetzt in voller Breite unten, mehr Platz beim Betrachten eines Servers durch ausgeblendete Serverliste",
    ],
  },
  {
    version: "0.1.29",
    date: "2026-08-08",
    items: [
      "Neu: Konfiguration für 7 Days to Die und Palworld (vorher nur Minecraft/Factorio) - Server Name, Port, Spielmodus, Zombies, Landanspruch, Sandbox-Code u.a.",
      "Neu: Dropdown-Auswahl für Einstellungen mit festen Werten statt Freitext (z.B. Spielmodus)",
      "Neu: Versionsanzeige (Steam-Build-Nummer) auch für SteamCMD-Spiele, nicht mehr nur Minecraft",
      "Fix: Einige 7-Days-to-Die-Einstellungen (Schwierigkeit, XP, Loot) existieren seit Version 1.0 nicht mehr einzeln, sondern nur noch im Sandbox-Code - entsprechend angepasst",
    ],
  },
  {
    version: "0.1.28",
    date: "2026-08-08",
    items: [
      "Neu: Live-Fortschrittsanzeige bei der Installation (Schritt, Download/Überprüfung, Prozent)",
      "Fix: Fehlgeschlagene Installationen ließen halbfertige Downloads auf dem Server liegen - werden jetzt automatisch aufgeräumt",
      "Fix: 'Komplett deinstallieren' konnte bei Verbindungsfehlern die Instanz nur lokal entfernen, Dienst/Dateien blieben unbemerkt auf dem Server erhalten",
      "Fix: 7 Days to Die startete nicht (falsches Start-Flag, ungültiger Log-Pfad)",
    ],
  },
  {
    version: "0.1.27",
    date: "2026-08-08",
    items: [
      "Fix: Spiele-Installation aus dem App-Store brach bei größeren Downloads mit Zeitüberschreitung ab (Timeout war fix auf 20 Sekunden)",
      "Ausführbarer Dateiname jetzt grimmnetz.exe statt glimanexus.exe",
    ],
  },
  {
    version: "0.1.26",
    date: "2026-08-07",
    items: [
      "Umbenannt zu GrimmNetz (vorher GlimaNexus) - neues Logo, App-Icon und Titelleiste",
      "Fix: Update installierte sich nicht dauerhaft (alte Verknüpfung zeigte auf veraltete Installation)",
    ],
  },
  {
    version: "0.1.25",
    date: "2026-08-05",
    items: [
      "Neues Logo & App-Icon (GlimaLabs-Rebrand)",
      "Status-Seite: Live-Log direkt neben CPU/RAM-Übersicht, CPU/RAM-Diagramme mit Achsenbeschriftung",
      "Server bearbeiten: Name/IP/Port/Zugangsdaten nachträglich ändern, ohne den Server neu einzurichten",
      "Neu: Firewall-Aktivieren-Vorschlag bei ungeschützten Servern (mit automatischem Sicherheitsnetz)",
      "Diverse Layout-Fixes (Log-Fenster, Ressourcen-Anzeige, Sidebar-Breite)",
    ],
  },
  {
    version: "0.1.24",
    date: "2026-08-05",
    items: [
      "Neu: Server suchen findet Gameserver, die schon auf dem Server installiert sind, aber der App nicht mehr bekannt sind (z.B. nach App-Neuinstallation)",
      "Neu: Backups wiederherstellen",
      "Neu: Automatische Sicherheitsupdates, begrenzte Log-Größe und passende Zeitzone bei neuen Servern",
      "Minecraft zeigt jetzt live Spieleranzahl und Weltname an",
      "Diverse Anzeige-Fehler behoben (Update-Button, Versionsanzeige, Backup-Button-Farben)",
    ],
  },
  {
    version: "0.1.23",
    date: "2026-08-05",
    items: [
      "Umbenannt zu GlimaNexus (vorher NovaNexus) - bestehende Installationen brauchen einmalig den neuen Installer und müssen Server-Passwörter neu eingeben",
      "Unterstützung für Fedora/RHEL/CentOS/Rocky/AlmaLinux-Server zusätzlich zu Ubuntu/Debian",
      "Firewall-Ports werden beim Installieren und bei Port-Änderungen automatisch freigegeben (ufw/firewalld)",
      "Server ohne Swap bekommen automatisch eine 2 GB Swap-Datei, damit RAM-Spitzen nicht den Gameserver killen",
      "Neuer Backup-Manager: Server-Backups erstellen, herunterladen und löschen",
      "Neue einfache Verzeichnis-Ansicht für Hauptverzeichnis und Backup-Ordner übers ⋯-Menü",
    ],
  },
  {
    version: "0.1.22",
    date: "2026-08-04",
    items: [
      "Steamcmd-Installationen (Palworld, 7 Days to Die, DayZ, Satisfactory, SCUM, Valheim, V Rising) funktionieren jetzt zuverlässig",
      "7 Days to Die zeigt jetzt das richtige Icon",
      "Server- und Gameserver-Kacheln überarbeitet (größere Icons, Layout näher am Design)",
      "Minecraft zeigt jetzt die echte installierte Version an, inkl. Update-Button bei neuen Versionen",
      "Während der Installation eines Gameservers erscheint jetzt eine Fortschritts-Kachel",
    ],
  },
  {
    version: "0.1.21",
    date: "2026-08-04",
    items: [
      "Konfiguration-Tab (Server-Einstellungen) liest jetzt tatsächlich die gespeicherten Werte statt immer die Standardwerte zu zeigen",
    ],
  },
  {
    version: "0.1.20",
    date: "2026-08-04",
    items: [
      "Minecraft installiert jetzt automatisch die neueste Paper-Version statt einer festen alten Version",
    ],
  },
  {
    version: "0.1.19",
    date: "2026-08-04",
    items: [
      "Gameserver mit relativem Startbefehl (Palworld, 7DTD, DayZ, Satisfactory, SCUM, Valheim, V Rising, Factorio) starteten nicht - behoben",
    ],
  },
  {
    version: "0.1.18",
    date: "2026-08-04",
    items: [
      "Server-Einstellungen: neuer Konfiguration-Tab zum Bearbeiten von Server-Name, Spieler-Slots, Passwort etc. (Minecraft & Factorio)",
      "7 weitere Spiele installierbar: 7 Days to Die, DayZ, Factorio, Satisfactory, SCUM, Valheim, V Rising",
    ],
  },
  {
    version: "0.1.17",
    date: "2026-08-04",
    items: [
      "Neuer System-Status-Widget in der Sidebar (CPU/RAM/Netzwerk des eigenen PCs)",
      "Echte Distro-Icons, überarbeiteter Status & Ressourcen Tab",
    ],
  },
];
