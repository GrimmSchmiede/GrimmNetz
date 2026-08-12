import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ServerRecord } from "./types";

type Props = {
  onClose: () => void;
  onCreated: (server: ServerRecord) => void;
};

type Mode = "existing" | "new-cloud";

export default function AddServerDialog({ onClose, onCreated }: Props) {
  const [mode, setMode] = useState<Mode>("new-cloud");

  // Weg A: bestehender Server, Passwort
  const [name, setName] = useState("");
  const [host, setHost] = useState("");
  const [port, setPort] = useState("22");
  const [username, setUsername] = useState("root");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  // Weg B: neuer Cloud-Server, Key zuerst
  const [pendingName, setPendingName] = useState("");
  const [pendingUsername, setPendingUsername] = useState("root");
  const [pendingServerId, setPendingServerId] = useState("");
  const [publicKey, setPublicKey] = useState("");
  const [pendingHost, setPendingHost] = useState("");
  const [pendingPort, setPendingPort] = useState("22");
  const [preparing, setPreparing] = useState(false);
  const [finalizing, setFinalizing] = useState(false);
  const [pendingError, setPendingError] = useState("");

  async function submitExisting(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError("");
    try {
      const server = await invoke<ServerRecord>("add_server", {
        input: {
          name,
          host,
          port: Number(port),
          username,
          password,
          timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
        },
      });
      onCreated(server);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  async function prepareNewCloud(e: React.FormEvent) {
    e.preventDefault();
    setPreparing(true);
    setPendingError("");
    try {
      const result = await invoke<{ server_id: string; public_key: string }>("prepare_key_only_server", {
        name: pendingName,
        username: pendingUsername,
      });
      setPendingServerId(result.server_id);
      setPublicKey(result.public_key);
    } catch (err) {
      setPendingError(String(err));
    } finally {
      setPreparing(false);
    }
  }

  async function finalizeNewCloud(e: React.FormEvent) {
    e.preventDefault();
    setFinalizing(true);
    setPendingError("");
    try {
      const server = await invoke<ServerRecord>("finalize_pending_server", {
        serverId: pendingServerId,
        host: pendingHost,
        port: Number(pendingPort),
        timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
      });
      onCreated(server);
    } catch (err) {
      setPendingError(String(err));
    } finally {
      setFinalizing(false);
    }
  }

  return (
    <div className="nx-modal-overlay" onClick={onClose}>
      <div className="nx-modal" onClick={(e) => e.stopPropagation()}>
        <h2>Server hinzufügen</h2>

        <div className="nx-tab-row">
          <button type="button" className={mode === "new-cloud" ? "nx-tab-active" : "nx-tab"} onClick={() => setMode("new-cloud")}>
            Neuer Cloud-Server (empfohlen)
          </button>
          <button type="button" className={mode === "existing" ? "nx-tab-active" : "nx-tab"} onClick={() => setMode("existing")}>
            Bestehender Server
          </button>
        </div>

        {mode === "existing" && (
          <form onSubmit={submitExisting}>
            <label>
              Name
              <input value={name} onChange={(e) => setName(e.target.value)} required placeholder="z.B. Hetzner-VPS-01" />
            </label>
            <label>
              IP-Adresse / Host
              <input value={host} onChange={(e) => setHost(e.target.value)} required placeholder="88.198.23.45" />
            </label>
            <div className="nx-modal-row">
              <label style={{ flex: 1 }}>
                SSH-Port
                <input value={port} onChange={(e) => setPort(e.target.value)} required />
              </label>
              <label style={{ flex: 2 }}>
                Benutzername
                <input value={username} onChange={(e) => setUsername(e.target.value)} required />
              </label>
            </div>
            <label>
              Passwort
              <input type="password" value={password} onChange={(e) => setPassword(e.target.value)} required />
            </label>
            <p style={{ color: "var(--nx-text-muted)", fontSize: 11, marginTop: -4 }}>
              GrimmNetz richtet automatisch einen Brute-Force-Schutz (fail2ban) ein und rollt danach automatisch einen
              SSH-Key aus, sodass zukünftige Verbindungen ohne Passwort auskommen.
            </p>
            {error && <div className="nx-update-error">{error}</div>}
            {busy && (
              <div style={{ color: "var(--nx-text-muted)", fontSize: 12 }}>
                Verbinde per SSH und richte Server ein (gameserver-User, Abhängigkeiten)…
              </div>
            )}
            <div className="nx-modal-actions">
              <button type="button" onClick={onClose} disabled={busy}>
                Abbrechen
              </button>
              <button type="submit" className="nx-update-btn" disabled={busy}>
                {busy ? "Verbinde…" : "Server hinzufügen"}
              </button>
            </div>
          </form>
        )}

        {mode === "new-cloud" && !publicKey && (
          <form onSubmit={prepareNewCloud}>
            <label>
              Name
              <input value={pendingName} onChange={(e) => setPendingName(e.target.value)} required placeholder="z.B. Hetzner-VPS-01" />
            </label>
            <label>
              Benutzername (beim Provider meist "root")
              <input value={pendingUsername} onChange={(e) => setPendingUsername(e.target.value)} required />
            </label>
            <p style={{ color: "var(--nx-text-muted)", fontSize: 11, marginTop: -4 }}>
              GrimmNetz erzeugt jetzt einen SSH-Key nur für diesen Server. Kein Passwort nötig - umgeht auch
              Anbieter (z.B. Hetzner), die beim ersten Passwort-Login eine Passwortänderung erzwingen.
            </p>
            {pendingError && <div className="nx-update-error">{pendingError}</div>}
            <div className="nx-modal-actions">
              <button type="button" onClick={onClose} disabled={preparing}>
                Abbrechen
              </button>
              <button type="submit" className="nx-update-btn" disabled={preparing}>
                {preparing ? "Erzeuge Key…" : "Key erzeugen"}
              </button>
            </div>
          </form>
        )}

        {mode === "new-cloud" && publicKey && (
          <form onSubmit={finalizeNewCloud}>
            <ol style={{ fontSize: 13, paddingLeft: 18 }}>
              <li>Cloud-Panel deines Anbieters öffnen (z.B. Hetzner Cloud)</li>
              <li>Beim Server-Erstellen diesen Key einfügen:</li>
            </ol>
            <div className="nx-modal-row">
              <textarea readOnly value={publicKey} rows={3} style={{ flex: 1, fontFamily: "monospace", fontSize: 11 }} />
              <button type="button" onClick={() => navigator.clipboard.writeText(publicKey)}>
                Kopieren
              </button>
            </div>
            <label>
              IP-Adresse, sobald die VM läuft
              <input value={pendingHost} onChange={(e) => setPendingHost(e.target.value)} required placeholder="88.198.23.45" />
            </label>
            <label>
              SSH-Port
              <input value={pendingPort} onChange={(e) => setPendingPort(e.target.value)} required />
            </label>
            {pendingError && <div className="nx-update-error">{pendingError}</div>}
            {finalizing && (
              <div style={{ color: "var(--nx-text-muted)", fontSize: 12 }}>
                Verbinde per Key und richte Server ein…
              </div>
            )}
            <div className="nx-modal-actions">
              <button type="button" onClick={onClose} disabled={finalizing}>
                Später fertigstellen
              </button>
              <button type="submit" className="nx-update-btn" disabled={finalizing}>
                {finalizing ? "Verbinde…" : "Verbinden"}
              </button>
            </div>
          </form>
        )}
      </div>
    </div>
  );
}
