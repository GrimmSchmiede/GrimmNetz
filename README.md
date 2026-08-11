![GrimmNetz](.github/grimmnetz_banner.png)

# 🛡️ GrimmNetz

<p align="center">
  <strong>Der intuitive Gameserver-Manager für Gamer – Volle Linux-Performance ohne CLI-Angst.</strong>
</p>

<p align="center">
  <img src="https://img.shields.io/github/v/release/GrimmSchmiede/GrimmNetz?label=Version&color=1abc9c" alt="Version">
  <img src="https://img.shields.io/github/license/GrimmSchmiede/GrimmNetz?color=1abc9c" alt="License">
  <img src="https://img.shields.io/badge/Platform-Windows-1abc9c" alt="Platform">
</p>

---

GrimmNetz ist eine ressourcenschonende, plattformübergreifende Desktop-App, die es Gamern ohne CLI-Angst ermöglicht, dedizierte Gameserver auf einem Linux-VPS oder Root-Server via SSH zu verwalten. 

> [!IMPORTANT]
> **Open Source & Community-driven:** GrimmNetz ist unter der **GPL-3.0 Lizenz** veröffentlicht.

---

## 🗺️ Inhaltsverzeichnis
- [✨ Key-Features](#-key-features)
- [🔒 Sicherheitsprinzipien](#-security-by-design)
- [💻 Tech-Stack](#-tech-stack)
- [🎮 Unterstützte Spiele](#-unterstützte-spiele)
- [🛠️ Entwicklung & Build-Setup](#%EF%B8%8F-entwicklung--build-setup)
- [🚀 Releases & Auto-Update](#-releases--auto-update)
- [🗺️ Roadmap](#%EF%B8%8F-roadmap)

---

## ✨ Key-Features

### 🖥️ Server-Verwaltung & Automatisierung
- **Multi-Server-Support:** Verbinde beliebig viele Linux-Root-/VPS-Server sicher über SSH.
- **Intelligentes OS-Onboarding:** Erkennt automatisch die Linux-Distribution.
- **Automatisches Hardening:** Legt einen isolierten `gameserver`-Systemnutzer an.
- **Ressourcen-Schutz:** Automatische Anlage einer Linux-Swap-Datei.
- **Dynamische Firewall:** UFW / Firewalld wird automatisch konfiguriert.

### 🎮 Betrieb & 1-Klick-Installation
- **App-Store-Feeling:** Installiere Spiele mit nur einem Klick.
- **Systemd-Integration:** Gameserver laufen als native Hintergrunddienste.
- **Echtzeit-Überwachung:** CPU, RAM und Festplattenauslastung im Blick.
- **Live-Terminal:** Integrierter Log-Stream (Echtzeit-Konsole).
- **Einstellungs-Editor:** Visuelle Bearbeitung von Server-Configs.

### 💾 Backup- & Dateiverwaltung
- **1-Klick-Backups:** Erstelle komprimierte `.tar.gz`-Archive.
- **SFTP-Dateibrowser:** Intuitive Datei- und Backup-Verwaltung.

---

## 🔒 Security by Design

Wir glauben an absolute Datensouveränität.

1. **Zero-Cloud-Storage:** Alle Verbindungsdaten verbleiben zu 100 % lokal.
2. **OS-Keyring-Integration:** Passwörter werden sicher im OS-eigenen Tresor gespeichert.
3. **Keine Root-Rechte für Spiele:** Alle Gameserver laufen strikt unter einem isolierten Nutzer.

---

## 💻 Tech-Stack

GrimmNetz kombiniert Rust im Backend mit React im Frontend:

| Schicht | Technologie | Beschreibung |
| :--- | :--- | :--- |
| **Framework** | [Tauri 2](https://tauri.app) | Ressourcenschonende Brücke. |
| **Frontend** | React + TypeScript (Vite) | Modernes, reaktives UI. |
| **Backend** | Rust (`tokio`, `russh`) | Hochperformante SSH-Verarbeitung. |
| **Datenbank**| SQLite + SQLCipher | Verschlüsselte lokale Speicherung. |

---

## 🎮 Unterstützte Spiele

Der GrimmNetz App-Store unterstützt die 1-Klick-Installation für:

| Spiel | Getestet | Distro |
| :--- | :---: | :--- |
| 🟩 **Minecraft (Paper)** *(mit Auto-Updates)* | ✅ | Ubuntu 24.04 |
| 🧟 **7 Days to Die** | ✅ | Ubuntu 24.04 |
| 🌴 **Palworld** | — | — |
| 🐺 **DayZ** | — | — |
| ⚙️ **Factorio** | — | — |
| 🏭 **Satisfactory** | — | — |
| 🪓 **Valheim** | — | — |
| 🧛 **V Rising** | — | — |
| 🏝️ **SCUM** | — | — |

✅ heißt: einmal komplett durchinstalliert und laufend verifiziert. Ohne Haken ist die Installation genauso implementiert, aber noch nicht end-to-end bestätigt.

---

## 🛠️ Entwicklung & Build-Setup

Für das Kompilieren unter Windows wird OpenSSL benötigt, um SQLCipher zu linken:

```bash
# Umgebungsvariablen setzen (Beispiel)
export OPENSSL_DIR="C:/Program Files/OpenSSL-Win64"
export OPENSSL_LIB_DIR="C:/Program Files/OpenSSL-Win64/lib/VC/x64/MD"

# Abhängigkeiten installieren & Dev-Server starten
npm install
npm run tauri dev
```

---

## 🚀 Releases & Auto-Update

Der Prozess ist vollständig automatisiert über GitHub Actions. Der Updater prüft bei Start auf neue Versionen.

```bash
git tag vX.Y.Z
git push origin vX.Y.Z
```

---

## 🗺️ Roadmap

Bewusst zurückgestellte, aber sinnvolle Verbesserungen ohne laufende Kosten - keine feste Zusage, wann/ob sie kommen, aber die Richtung, in die es gehen kann:

- **Vollzugriff (SFTP):** Umbenennen/Rename von Dateien und Ordnern, Inline-Texteditor für Konfigs
- **Instanz-Isolation verschärfen:** Instanzen von `/home/gameserver` nach `/srv/grimmnetz` verschieben, damit `ProtectHome=yes` im systemd-Sandbox nutzbar wird (verhindert dann auch Lesezugriff zwischen Nachbar-Instanzen, nicht nur Schreibzugriff)
- **SteamCMD-Wrapper:** Strukturierte Fehlerbehandlung (Festplatte voll, Login-Fehler, ...) statt reinem Fortschritts-Parsing
- **vcpkg-Integration:** OpenSSL/SQLCipher-Abhängigkeiten unter Windows automatisch auflösen statt manueller Umgebungsvariablen
- **Weitere Spiele durchtesten:** Palworld, DayZ, Factorio, Satisfactory, Valheim, V Rising, SCUM einmal komplett end-to-end verifizieren (siehe [Unterstützte Spiele](#-unterstützte-spiele))
- **Interaktive Konsole:** Aktuell reines Log-Streaming - eine echte Zwei-Wege-Konsole (Befehle direkt eintippen) wäre der nächste Schritt
- **Installationen von der App-Laufzeit entkoppeln:** Läuft eine Spiele-Installation aktuell über den offenen SSH-Stream - bricht die Verbindung während eines langen SteamCMD-Downloads ab oder wird die App geschlossen, ist der Install weg statt pausiert/fortsetzbar. Lösung wäre, den Install als eigenständigen systemd-oneshot-Dienst auf dem Server laufen zu lassen und den Fortschritt nur noch abzufragen, statt ihn live zu streamen
- **CI-Validierung der generierten systemd-Units:** `systemd-analyze verify` gegen die von `render_systemd_unit` erzeugten Dateien in GitHub Actions laufen lassen, um Syntaxfehler bei neuen Spiele-Templates automatisch abzufangen - ohne echte SteamCMD-Downloads in CI zu brauchen
- **Desktop-Benachrichtigungen:** Meldung bei Server-Absturz oder erreichtem RAM-Limit, statt es nur beim aktiven Draufschauen zu bemerken
- **Theme-Wechsler:** Echter Light-Mode bzw. Deep-Black-Variante für OLED-Monitore, zusätzlich zum aktuellen Dark-Theme
- **SSH-Schlüsselverwaltung:** Aktuell nur Passwort-Login - Key-basierte Authentifizierung wäre ein eigenes, größeres Feature
