import { useEffect, useState } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import type { GameTemplate, InstallEvent, InstanceRecord } from "./types";
import GameIcon from "./GameIcon";

type Props = {
  serverId: string;
  onClose: () => void;
  onInstalled: (instance: InstanceRecord) => void;
  onInstallStart: (game: GameTemplate) => void;
  onInstallProgress: (label: string) => void;
  onInstallDone: () => void;
};

export default function GameStoreDialog({ serverId, onClose, onInstalled, onInstallStart, onInstallProgress, onInstallDone }: Props) {
  const [games, setGames] = useState<GameTemplate[]>([]);
  const [installingId, setInstallingId] = useState<string | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    invoke<GameTemplate[]>("list_games")
      .then(setGames)
      .catch((err) => setError(String(err)));
  }, []);

  async function install(game: GameTemplate) {
    setInstallingId(game.id);
    setError("");
    onInstallStart(game);
    try {
      const onEvent = new Channel<InstallEvent>();
      onEvent.onmessage = (event) => {
        if (event.event === "step") onInstallProgress(event.label);
        else onInstallProgress(`${event.phase}: ${event.percent.toFixed(0)}%`);
      };
      const instance = await invoke<InstanceRecord>("install_game", {
        serverId,
        gameId: game.id,
        displayName: game.name,
        onEvent,
      });
      onInstalled(instance);
    } catch (err) {
      setError(String(err));
    } finally {
      setInstallingId(null);
      onInstallDone();
    }
  }

  return (
    <div className="nx-modal-overlay" onClick={onClose}>
      <div className="nx-modal nx-store-modal" onClick={(e) => e.stopPropagation()}>
        <h2>App-Store</h2>
        {error && <div className="nx-update-error">{error}</div>}

        <div className="nx-game-grid">
          {games.map((game) => (
            <div key={game.id} className="nx-game-card">
              <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 4 }}>
                <GameIcon gameId={game.id} size={32} />
                <span style={{ fontWeight: 600 }}>{game.name}</span>
                {game.tested_on.length > 0 && (
                  <span
                    title={`Getestet auf: ${game.tested_on.join(", ")}`}
                    style={{ color: "var(--nx-success)", fontSize: 12 }}
                  >
                    ✅
                  </span>
                )}
              </div>
              <div style={{ color: "var(--nx-text-muted)", fontSize: 12, marginBottom: 2 }}>{game.subtitle}</div>
              <div style={{ color: "var(--nx-text-muted)", fontSize: 11, marginBottom: 10 }}>
                {game.tested_on.length > 0 ? `Getestet: ${game.tested_on.join(", ")}` : "Noch nicht getestet"}
              </div>
              <button
                className="nx-update-btn"
                disabled={installingId !== null}
                onClick={() => install(game)}
              >
                {installingId === game.id && <span className="nx-spinner" />}
                {installingId === game.id ? "Installiere…" : "Installieren"}
              </button>
              {installingId === game.id && (
                <div style={{ color: "var(--nx-text-muted)", fontSize: 11, marginTop: 6 }}>
                  Kann 1–2 Minuten dauern (Download & Einrichtung auf dem Server)
                </div>
              )}
            </div>
          ))}
        </div>

        <div className="nx-modal-actions">
          <button type="button" onClick={onClose}>
            Schließen
          </button>
        </div>
      </div>
    </div>
  );
}
