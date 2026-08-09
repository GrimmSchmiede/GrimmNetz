import { useEffect, useMemo, useState } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { fetch as tauriFetch } from "@tauri-apps/plugin-http";
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
  const [search, setSearch] = useState("");
  const [steamBuilds, setSteamBuilds] = useState<Record<number, string>>({});
  const [otherVersions, setOtherVersions] = useState<Record<string, string>>({});

  useEffect(() => {
    invoke<GameTemplate[]>("list_games")
      .then(setGames)
      .catch((err) => setError(String(err)));
  }, []);

  // Live build number straight from Steam's public depot info, instead of a version string we'd
  // have to keep updated by hand (and which would silently go stale the moment Steam ships an
  // update, since installs always pull the latest build anyway via `app_update ... validate`).
  useEffect(() => {
    const appIds = [...new Set(games.map((g) => g.install.app_id).filter((id): id is number => id != null))];
    appIds.forEach((appId) => {
      fetch(`https://api.steamcmd.net/v1/info/${appId}`)
        .then((res) => res.json())
        .then((json) => {
          const buildId = json?.data?.[String(appId)]?.depots?.branches?.public?.buildid;
          if (buildId) setSteamBuilds((prev) => ({ ...prev, [appId]: String(buildId) }));
        })
        .catch(() => {});
    });
  }, [games]);

  // The two direct-download games each have their own public "latest version" API - same idea
  // as the Steam build lookup above, just per-game since there's no shared endpoint for these.
  useEffect(() => {
    if (games.some((g) => g.id === "minecraft-paper")) {
      fetch("https://fill.papermc.io/v3/projects/paper")
        .then((res) => res.json())
        .then((json) => {
          const latest = Object.values(json.versions)[0] as string[] | undefined;
          if (latest?.[0]) setOtherVersions((prev) => ({ ...prev, "minecraft-paper": `Paper ${latest[0]}` }));
        })
        .catch(() => {});
    }
    if (games.some((g) => g.id === "factorio")) {
      // factorio.com sends no Access-Control-Allow-Origin header, so a page-level fetch gets
      // blocked by the webview's CORS enforcement - the Rust-native http plugin isn't subject
      // to that (no browser origin involved), unlike api.steamcmd.net/fill.papermc.io above.
      tauriFetch("https://factorio.com/api/latest-releases")
        .then((res) => res.json())
        .then((json) => {
          const version = json?.stable?.headless;
          if (version) setOtherVersions((prev) => ({ ...prev, factorio: version }));
        })
        .catch(() => {});
    }
  }, [games]);

  const filteredGames = useMemo(() => {
    const q = search.trim().toLowerCase();
    if (!q) return games;
    return games.filter((g) => g.name.toLowerCase().includes(q) || g.subtitle.toLowerCase().includes(q));
  }, [games, search]);

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
    <div>
      <div style={{ display: "flex", alignItems: "center", gap: 12, marginBottom: 16 }}>
        <button className="nx-icon-btn" onClick={onClose} aria-label="Zurück">
          ← Zurück
        </button>
        <h3 style={{ margin: 0 }}>App-Store</h3>
      </div>

      <div style={{ display: "flex", justifyContent: "center", marginBottom: 18 }}>
        <div style={{ position: "relative", width: "100%", maxWidth: 420 }}>
          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Spiel suchen…"
            style={{
              width: "100%",
              background: "var(--nx-panel-alt)",
              border: "1px solid color-mix(in srgb, var(--nx-accent) 40%, transparent)",
              borderRadius: "var(--nx-radius)",
              padding: "10px 36px 10px 14px",
              color: "var(--nx-text)",
              fontSize: 14,
            }}
          />
          {search && (
            <button
              onClick={() => setSearch("")}
              aria-label="Suche zurücksetzen"
              style={{
                position: "absolute",
                right: 8,
                top: "50%",
                transform: "translateY(-50%)",
                background: "transparent",
                border: "none",
                color: "var(--nx-text-muted)",
                fontSize: 16,
                lineHeight: 1,
                cursor: "pointer",
                padding: 4,
              }}
            >
              ×
            </button>
          )}
        </div>
      </div>

      {error && <div className="nx-update-error">{error}</div>}
      {filteredGames.length === 0 && !error && (
        <p style={{ color: "var(--nx-text-muted)", textAlign: "center" }}>Kein Spiel gefunden.</p>
      )}

      <div className="nx-store-scroll">
        <div className="nx-game-grid nx-game-grid-3col">
          {filteredGames.map((game) => {
            const tested = game.tested_on.length > 0;
            const dotColor = tested ? "var(--nx-success)" : "#facc15";
            const dotTitle = tested
              ? `Getestet: GrimmNetz hat dieses Spiel auf ${game.tested_on.join(", ")} geprüft.`
              : "Nicht getestet: sollte funktionieren, wurde aber auf deiner Version noch nicht verifiziert.";
            return (
              <div key={game.id} className="nx-game-card">
                <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 8 }}>
                  <GameIcon gameId={game.id} size={32} />
                  <span style={{ fontWeight: 600, flex: 1 }}>{game.name}</span>
                  <span title={dotTitle} style={{ display: "flex", alignItems: "center", gap: 5, cursor: "help", flexShrink: 0 }}>
                    <span style={{ fontSize: 11, color: "var(--nx-text-muted)" }}>{tested ? "Getestet" : "Nicht getestet"}</span>
                    <span
                      style={{
                        width: 10,
                        height: 10,
                        borderRadius: "50%",
                        background: dotColor,
                        flexShrink: 0,
                      }}
                    />
                  </span>
                </div>
                <div style={{ color: "var(--nx-text-muted)", fontSize: 12, marginBottom: 12 }}>
                  Version:{" "}
                  {game.install.app_id != null
                    ? steamBuilds[game.install.app_id]
                      ? `Build ${steamBuilds[game.install.app_id]}`
                      : "…"
                    : otherVersions[game.id] ?? "…"}
                </div>
                <button
                  className="nx-update-btn"
                  disabled={installingId !== null}
                  onClick={() => install(game)}
                  style={{ width: "100%" }}
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
            );
          })}
        </div>
      </div>
    </div>
  );
}
