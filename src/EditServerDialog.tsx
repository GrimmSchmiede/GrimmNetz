import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ServerRecord } from "./types";

type Props = {
  server: ServerRecord;
  onClose: () => void;
  onUpdated: (server: ServerRecord) => void;
};

export default function EditServerDialog({ server, onClose, onUpdated }: Props) {
  const [name, setName] = useState(server.name);
  const [host, setHost] = useState(server.host ?? "");
  const [port, setPort] = useState(String(server.port));
  const [username, setUsername] = useState(server.username);
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError("");
    try {
      const updated = await invoke<ServerRecord>("update_server", {
        serverId: server.id,
        name,
        host,
        port: Number(port),
        username,
        password: password || null,
      });
      onUpdated(updated);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="nx-modal-overlay" onClick={onClose}>
      <form className="nx-modal" onClick={(e) => e.stopPropagation()} onSubmit={submit}>
        <h2>Server bearbeiten</h2>

        <label>
          Name
          <input value={name} onChange={(e) => setName(e.target.value)} required />
        </label>

        <label>
          IP-Adresse / Host
          <input value={host} onChange={(e) => setHost(e.target.value)} required />
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
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder="Leer lassen, um das bestehende Passwort zu behalten"
          />
        </label>

        {error && <div className="nx-update-error">{error}</div>}
        {busy && (
          <div style={{ color: "var(--nx-text-muted)", fontSize: 12 }}>Prüfe Verbindung mit den neuen Daten…</div>
        )}

        <div className="nx-modal-actions">
          <button type="button" onClick={onClose} disabled={busy}>
            Abbrechen
          </button>
          <button type="submit" className="nx-update-btn" disabled={busy}>
            {busy ? "Prüfe…" : "Speichern"}
          </button>
        </div>
      </form>
    </div>
  );
}
