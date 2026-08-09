export type PatchNote = {
  version: string;
  date: string;
  items: string[];
};

// Maintained by hand alongside each release - keep entries short and user-facing,
// newest first. Not every internal commit needs its own entry.
export const PATCH_NOTES: PatchNote[] = [
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
