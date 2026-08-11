import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { isEnabled, enable, disable } from "@tauri-apps/plugin-autostart";

type Settings = {
  close_to_tray: boolean;
  refresh_interval_ms: number;
};

type Props = {
  onClose: () => void;
  onRefreshIntervalChange: (ms: number) => void;
};

export default function SettingsDialog({ onClose, onRefreshIntervalChange }: Props) {
  const [autostart, setAutostart] = useState(false);
  const [closeToTray, setCloseToTray] = useState(false);
  const [refreshIntervalMs, setRefreshIntervalMs] = useState(3000);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");

  useEffect(() => {
    Promise.all([invoke<Settings>("get_settings"), isEnabled()])
      .then(([settings, autostartEnabled]) => {
        setCloseToTray(settings.close_to_tray);
        setRefreshIntervalMs(settings.refresh_interval_ms);
        setAutostart(autostartEnabled);
      })
      .catch((err) => setError(String(err)))
      .finally(() => setLoading(false));
  }, []);

  async function toggleAutostart(next: boolean) {
    setError("");
    try {
      if (next) await enable();
      else await disable();
      setAutostart(next);
    } catch (err) {
      setError(String(err));
    }
  }

  async function save(next: Partial<Settings>) {
    const merged: Settings = {
      close_to_tray: next.close_to_tray ?? closeToTray,
      refresh_interval_ms: next.refresh_interval_ms ?? refreshIntervalMs,
    };
    setError("");
    try {
      await invoke("save_settings", { settings: merged });
      setCloseToTray(merged.close_to_tray);
      setRefreshIntervalMs(merged.refresh_interval_ms);
      if (next.refresh_interval_ms) onRefreshIntervalChange(merged.refresh_interval_ms);
    } catch (err) {
      setError(String(err));
    }
  }

  return (
    <div className="nx-modal-overlay" onClick={onClose}>
      <div className="nx-modal" onClick={(e) => e.stopPropagation()} style={{ width: 440 }}>
        <h2>Einstellungen</h2>
        {loading && <p style={{ color: "var(--nx-text-muted)" }}>Lade…</p>}
        {error && <div className="nx-update-error">{error}</div>}
        {!loading && (
          <div style={{ display: "flex", flexDirection: "column", gap: 18 }}>
            <label style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12 }}>
              <span>
                Autostart
                <div style={{ color: "var(--nx-text-muted)", fontSize: 11 }}>GrimmNetz beim Windows-Start automatisch öffnen</div>
              </span>
              <input type="checkbox" checked={autostart} onChange={(e) => toggleAutostart(e.target.checked)} />
            </label>

            <label style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12 }}>
              <span>
                In Tray minimieren
                <div style={{ color: "var(--nx-text-muted)", fontSize: 11 }}>
                  Fenster schließen legt die App nur ins Tray, statt sie zu beenden
                </div>
              </span>
              <input
                type="checkbox"
                checked={closeToTray}
                onChange={(e) => save({ close_to_tray: e.target.checked })}
              />
            </label>

            <label style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12 }}>
              <span>
                Aktualisierungs-Intervall
                <div style={{ color: "var(--nx-text-muted)", fontSize: 11 }}>Wie oft CPU/RAM/Netzwerk-Werte abgefragt werden</div>
              </span>
              <select
                value={refreshIntervalMs}
                onChange={(e) => save({ refresh_interval_ms: Number(e.target.value) })}
                style={{
                  background: "var(--nx-panel-alt)",
                  border: "1px solid var(--nx-border)",
                  borderRadius: "var(--nx-radius)",
                  padding: "6px 10px",
                  color: "var(--nx-text)",
                }}
              >
                <option value={1000}>1 Sekunde</option>
                <option value={2000}>2 Sekunden</option>
                <option value={5000}>5 Sekunden</option>
              </select>
            </label>
          </div>
        )}
        <div className="nx-modal-actions">
          <button type="button" onClick={onClose}>
            Schließen
          </button>
        </div>
      </div>
    </div>
  );
}
