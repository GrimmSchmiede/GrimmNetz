import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ServerRecord } from "./types";

type Props = {
  onClose: () => void;
  onCreated: (server: ServerRecord) => void;
};

export default function AddServerDialog({ onClose, onCreated }: Props) {
  const [name, setName] = useState("");
  const [host, setHost] = useState("");
  const [port, setPort] = useState("22");
  const [username, setUsername] = useState("root");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  async function submit(e: React.FormEvent) {
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

  return (
    <div className="nx-modal-overlay" onClick={onClose}>
      <form className="nx-modal" onClick={(e) => e.stopPropagation()} onSubmit={submit}>
        <h2>Server hinzufügen</h2>

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
          GrimmNetz richtet automatisch einen Brute-Force-Schutz (fail2ban) ein: Nach 3 falschen Passwort-Versuchen
          wird deine IP für 1 Stunde gesperrt. Gib das Passwort also sorgfältig ein.
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
    </div>
  );
}
