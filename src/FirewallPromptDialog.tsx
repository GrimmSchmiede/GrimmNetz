import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type Props = {
  serverName: string;
  serverId: string;
  onClose: () => void;
};

export default function FirewallPromptDialog({ serverName, serverId, onClose }: Props) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [done, setDone] = useState(false);

  async function activate() {
    setBusy(true);
    setError("");
    try {
      await invoke("enable_firewall", { serverId });
      setDone(true);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="nx-modal-overlay" onClick={done || busy ? undefined : onClose}>
      <div className="nx-modal" onClick={(e) => e.stopPropagation()}>
        <h2>{done ? "🛡️ Server jetzt geschützt" : "⚠️ Server ungeschützt"}</h2>
        {!done && (
          <>
            <p style={{ color: "var(--nx-text-muted)", fontSize: 14, lineHeight: 1.5 }}>
              Auf <strong>{serverName}</strong> läuft aktuell keine Firewall. Wir empfehlen, eine zu aktivieren, damit
              nur die Ports erreichbar sind, die GrimmNetz auch tatsächlich freigibt (SSH bleibt garantiert erlaubt).
            </p>
            {error && <div className="nx-update-error">{error}</div>}
            {busy && (
              <div style={{ color: "var(--nx-text-muted)", fontSize: 12 }}>
                Aktiviere Firewall und prüfe die Verbindung…
              </div>
            )}
            <div className="nx-modal-actions">
              <button type="button" onClick={onClose} disabled={busy}>
                Nein
              </button>
              <button type="button" className="nx-update-btn" onClick={activate} disabled={busy}>
                {busy ? "…" : "Ja (empfohlen)"}
              </button>
            </div>
          </>
        )}
        {done && (
          <>
            <p style={{ color: "var(--nx-success)", fontSize: 14 }}>
              Firewall ist aktiv, SSH-Verbindung erfolgreich geprüft.
            </p>
            <div className="nx-modal-actions">
              <button type="button" className="nx-update-btn" onClick={onClose}>
                OK
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
