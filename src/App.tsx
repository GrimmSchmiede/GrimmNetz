import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import "./App.css";
import UpdateBanner from "./UpdateBanner";
import TitleBar from "./TitleBar";
import AddServerDialog from "./AddServerDialog";
import GameStoreDialog from "./GameStoreDialog";
import PatchNotesDialog from "./PatchNotesDialog";
import DirectoryBrowserDialog from "./DirectoryBrowserDialog";
import EditServerDialog from "./EditServerDialog";
import SftpBrowserDialog from "./SftpBrowserDialog";
import FirewallPromptDialog from "./FirewallPromptDialog";
import InstanceDetail from "./InstanceDetail";
import grimmNetzLogo from "./assets/grimmnetz_logo.png";
import GameIcon from "./GameIcon";
import DistroIcon from "./DistroIcon";
import type { GameTemplate, InstanceRecord, InstanceStatus, LocalSystemStats, ServerRecord, VersionInfo } from "./types";

type HardwareStats = {
  cpu_percent: number;
  ram_used_mb: number;
  ram_total_mb: number;
  disk_used_gb: number;
  disk_total_gb: number;
};

function formatUptime(seconds: number): string {
  if (seconds <= 0) return "–";
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (days > 0) return `${days}T ${hours}h ${minutes}m`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  return `${minutes}m`;
}

function MiniSparkline({ values }: { values: number[] }) {
  const width = 200;
  const height = 24;
  if (values.length < 2) return <svg width="100%" height={height} />;
  const max = Math.max(100, ...values);
  const points = values
    .map((v, i) => {
      const x = (i / (values.length - 1)) * width;
      const y = height - (v / max) * height;
      return `${x},${y}`;
    })
    .join(" ");
  return (
    <svg width="100%" height={height} viewBox={`0 0 ${width} ${height}`} preserveAspectRatio="none">
      <polyline points={points} fill="none" stroke="var(--nx-accent)" strokeWidth="1.5" />
    </svg>
  );
}

function App() {
  const [servers, setServers] = useState<ServerRecord[]>([]);
  const [selectedServerId, setSelectedServerId] = useState<string | null>(null);
  const [showAddDialog, setShowAddDialog] = useState(false);
  const [showStoreDialog, setShowStoreDialog] = useState(false);
  const [loading, setLoading] = useState(true);
  const [stats, setStats] = useState<Record<string, HardwareStats>>({});
  const [instances, setInstances] = useState<InstanceRecord[]>([]);
  const [instanceBusy, setInstanceBusy] = useState<string | null>(null);
  const [openInstanceId, setOpenInstanceId] = useState<string | null>(null);
  const [cpuHistory, setCpuHistory] = useState<Record<string, number[]>>({});
  const [ramHistory, setRamHistory] = useState<Record<string, number[]>>({});
  const [appVersion, setAppVersion] = useState("");
  const [instanceStatus, setInstanceStatus] = useState<Record<string, InstanceStatus>>({});
  const [instanceError, setInstanceError] = useState("");
  const [serverBusy, setServerBusy] = useState(false);
  const [openMenuId, setOpenMenuId] = useState<string | null>(null);
  const [gameSubtitles, setGameSubtitles] = useState<Record<string, string>>({});
  const [gameTemplates, setGameTemplates] = useState<Record<string, GameTemplate>>({});
  const [instanceVersions, setInstanceVersions] = useState<Record<string, VersionInfo>>({});
  const [updatingInstanceId, setUpdatingInstanceId] = useState<string | null>(null);
  const [installingGame, setInstallingGame] = useState<GameTemplate | null>(null);
  const [installProgress, setInstallProgress] = useState<string>("");
  const [showPatchNotes, setShowPatchNotes] = useState(false);
  const [browsingInstance, setBrowsingInstance] = useState<{ instance: InstanceRecord; target: "install" | "backups" } | null>(null);
  const [openServerMenuId, setOpenServerMenuId] = useState<string | null>(null);
  const [editingServer, setEditingServer] = useState<ServerRecord | null>(null);
  const [sftpServer, setSftpServer] = useState<ServerRecord | null>(null);
  const [firewallPromptServer, setFirewallPromptServer] = useState<ServerRecord | null>(null);
  const [discovering, setDiscovering] = useState(false);
  const [discoverError, setDiscoverError] = useState("");
  const [discoverResult, setDiscoverResult] = useState<number | null>(null);

  async function discoverInstances() {
    if (!selectedServerId) return;
    setDiscovering(true);
    setDiscoverError("");
    setDiscoverResult(null);
    try {
      const found = await invoke<InstanceRecord[]>("discover_instances", { serverId: selectedServerId });
      if (found.length > 0) {
        setInstances((prev) => [...prev, ...found]);
      }
      setDiscoverResult(found.length);
    } catch (err) {
      setDiscoverError(String(err));
    } finally {
      setDiscovering(false);
    }
  }
  const [localStats, setLocalStats] = useState<LocalSystemStats | null>(null);
  const [localCpuHistory, setLocalCpuHistory] = useState<number[]>([]);

  useEffect(() => {
    const poll = () => {
      invoke<LocalSystemStats>("get_local_system_stats")
        .then((result) => {
          setLocalStats(result);
          setLocalCpuHistory((prev) => [...prev.slice(-19), result.cpu_percent]);
        })
        .catch(() => {});
    };
    poll();
    const interval = setInterval(poll, 3000);
    return () => clearInterval(interval);
  }, []);

  useEffect(() => {
    invoke<GameTemplate[]>("list_games")
      .then((games) => {
        setGameSubtitles(Object.fromEntries(games.map((g) => [g.id, g.subtitle])));
        setGameTemplates(Object.fromEntries(games.map((g) => [g.id, g])));
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    if (!openMenuId) return;
    const close = () => setOpenMenuId(null);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [openMenuId]);

  useEffect(() => {
    if (!openServerMenuId) return;
    const close = () => setOpenServerMenuId(null);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [openServerMenuId]);

  useEffect(() => {
    loadServers();
    getVersion().then((version) => {
      setAppVersion(version);
      const lastSeen = localStorage.getItem("nx-last-seen-version");
      if (lastSeen && lastSeen !== version) setShowPatchNotes(true);
      localStorage.setItem("nx-last-seen-version", version);
    }).catch(() => {});
  }, []);

  useEffect(() => {
    if (selectedServerId) loadInstances(selectedServerId);
    else setInstances([]);
  }, [selectedServerId]);

  useEffect(() => {
    if (!selectedServerId || instances.length === 0) return;
    instances.forEach((instance) => {
      invoke<VersionInfo>("get_instance_version", {
        serverId: selectedServerId,
        gameId: instance.game_id,
        installPath: instance.install_path,
      })
        .then((info) => setInstanceVersions((prev) => ({ ...prev, [instance.id]: info })))
        .catch(() => {});
    });
  }, [selectedServerId, instances]);

  async function updateInstance(instance: InstanceRecord) {
    if (!selectedServerId) return;
    setUpdatingInstanceId(instance.id);
    setInstanceError("");
    try {
      await invoke("update_instance", {
        serverId: selectedServerId,
        gameId: instance.game_id,
        installPath: instance.install_path,
        unitName: instance.systemd_unit,
        ramLimitMb: instance.ram_limit_mb,
      });
      const info = await invoke<VersionInfo>("get_instance_version", {
        serverId: selectedServerId,
        gameId: instance.game_id,
        installPath: instance.install_path,
      });
      setInstanceVersions((prev) => ({ ...prev, [instance.id]: info }));
      const status = await invoke<InstanceStatus>("get_instance_status", {
        serverId: selectedServerId,
        unitName: instance.systemd_unit,
      });
      setInstanceStatus((prev) => ({ ...prev, [instance.id]: status }));
    } catch (err) {
      setInstanceError(`${instance.display_name}: ${String(err)}`);
    } finally {
      setUpdatingInstanceId(null);
    }
  }

  async function loadServers() {
    setLoading(true);
    try {
      const list = await invoke<ServerRecord[]>("list_servers");
      setServers(list);
      if (list.length > 0 && !selectedServerId) {
        setSelectedServerId(list[0].id);
      }
    } catch {
      // Backend command not reachable (e.g. dev preview outside Tauri) - keep empty list.
    } finally {
      setLoading(false);
    }
  }

  async function loadInstances(serverId: string) {
    try {
      const list = await invoke<InstanceRecord[]>("list_instances", { serverId });
      setInstances(list);
    } catch {
      setInstances([]);
    }
  }

  async function pollHardwareStats() {
    await Promise.all(servers.map((server) => {
      return invoke<HardwareStats>("get_hardware_stats", { serverId: server.id })
        .then((result) => {
          setStats((prev) => ({ ...prev, [server.id]: result }));
          setCpuHistory((prev) => ({ ...prev, [server.id]: [...(prev[server.id] ?? []).slice(-29), result.cpu_percent] }));
          setRamHistory((prev) => ({ ...prev, [server.id]: [...(prev[server.id] ?? []).slice(-29), result.ram_used_mb] }));
          // A successful poll proves the connection is fine now - clear any stale
          // connection error that may have shown up while the server was still booting.
          setInstanceError("");
        })
        .catch(() => {});
    }));
  }

  useEffect(() => {
    if (servers.length === 0) return;
    pollHardwareStats();
    const interval = setInterval(pollHardwareStats, 8000);
    return () => clearInterval(interval);
  }, [servers]);

  useEffect(() => {
    if (!selectedServerId || instances.length === 0) return;
    const poll = () => {
      instances.forEach((instance) => {
        invoke<InstanceStatus>("get_instance_status", { serverId: selectedServerId, unitName: instance.systemd_unit })
          .then((status) => setInstanceStatus((prev) => ({ ...prev, [instance.id]: status })))
          .catch(() => {});
      });
    };
    poll();
    const interval = setInterval(poll, 5000);
    return () => clearInterval(interval);
  }, [selectedServerId, instances]);

  async function runInstanceAction(instance: InstanceRecord, action: "start" | "stop" | "restart") {
    if (!selectedServerId) return;
    setInstanceBusy(instance.id);
    setInstanceError("");
    try {
      await invoke("control_instance", {
        serverId: selectedServerId,
        unitName: instance.systemd_unit,
        action,
      });
      const status = await invoke<InstanceStatus>("get_instance_status", {
        serverId: selectedServerId,
        unitName: instance.systemd_unit,
      });
      setInstanceStatus((prev) => ({ ...prev, [instance.id]: status }));
    } catch (err) {
      setInstanceError(`${instance.display_name}: ${String(err)}`);
    } finally {
      setInstanceBusy(null);
    }
  }

  async function handleForget(instance: InstanceRecord) {
    if (!confirm(`"${instance.display_name}" aus GrimmNetz entfernen? Dienst und Dateien bleiben auf dem Server erhalten.`)) return;
    setInstanceBusy(instance.id);
    try {
      await invoke("forget_instance", { instanceId: instance.id });
      setInstances((prev) => prev.filter((i) => i.id !== instance.id));
    } catch (err) {
      setInstanceError(`${instance.display_name}: ${String(err)}`);
    } finally {
      setInstanceBusy(null);
    }
  }

  async function handleUninstall(instance: InstanceRecord) {
    if (!selectedServerId) return;
    if (!confirm(`"${instance.display_name}" komplett deinstallieren? Dienst UND alle Dateien werden unwiderruflich vom Server gelöscht.`)) return;
    setInstanceBusy(instance.id);
    try {
      await invoke("delete_instance", {
        serverId: selectedServerId,
        instanceId: instance.id,
        unitName: instance.systemd_unit,
        installPath: instance.install_path,
      });
      setInstances((prev) => prev.filter((i) => i.id !== instance.id));
    } catch (err) {
      setInstanceError(`${instance.display_name}: ${String(err)}`);
    } finally {
      setInstanceBusy(null);
    }
  }

  async function handleReload() {
    if (!selectedServerId) return;
    setServerBusy(true);
    try {
      await pollHardwareStats();
      await loadInstances(selectedServerId);
    } finally {
      setServerBusy(false);
    }
  }

  async function handleRebootServer() {
    if (!selectedServer) return;
    if (!confirm(`"${selectedServer.name}" wirklich neu starten? Alle laufenden Gameserver werden dabei kurz unterbrochen.`)) return;
    setServerBusy(true);
    try {
      await invoke("reboot_server", { serverId: selectedServer.id });
    } catch (err) {
      setInstanceError(String(err));
    } finally {
      setServerBusy(false);
    }
  }

  async function handleDisconnectServer() {
    if (!selectedServer) return;
    if (!confirm(`"${selectedServer.name}" wirklich trennen und aus GrimmNetz entfernen?`)) return;
    setServerBusy(true);
    try {
      await invoke("delete_server", { id: selectedServer.id });
      setServers((prev) => prev.filter((s) => s.id !== selectedServer.id));
      setSelectedServerId(null);
    } catch (err) {
      setInstanceError(String(err));
    } finally {
      setServerBusy(false);
    }
  }

  const selectedServer = servers.find((s) => s.id === selectedServerId) ?? null;
  const selectedStats = selectedServerId ? stats[selectedServerId] : undefined;
  const isConnected = !!selectedStats;

  return (
    <div className="nx-shell">
      <TitleBar />
      <UpdateBanner />
      <aside className="nx-sidebar">
        <div className="nx-brand">
          <img src={grimmNetzLogo} alt="GrimmNetz" className="nx-brand-logo" />
        </div>

        <nav className="nx-nav">
          <button className="nx-nav-item active">Server-Liste</button>
          <button
            className="nx-nav-item"
            onClick={() => selectedServerId && setShowStoreDialog(true)}
            disabled={!selectedServerId}
          >
            App-Store
          </button>
          <button className="nx-nav-item">Einstellungen</button>
        </nav>

        <div className="nx-sidebar-footer">
          <div className="nx-user">
            <div className="nx-avatar" />
            <div>
              <div>GrimmUser</div>
              <div style={{ color: "var(--nx-text-muted)", fontSize: 12 }}>
                <span className="nx-status-dot" />
                Online
              </div>
            </div>
          </div>

          {localStats && (
            <div className="nx-system-status">
              <div className="nx-system-status-title">
                <span>🖥️</span> System Status
              </div>
              <div className="nx-system-status-row">
                <div className="nx-system-status-row-head">
                  <span className="nx-system-status-label">CPU</span>
                  <span className="nx-system-status-val">{localStats.cpu_percent.toFixed(0)}%</span>
                </div>
                <MiniSparkline values={localCpuHistory} />
              </div>
              <div className="nx-system-status-row">
                <div className="nx-system-status-row-head">
                  <span className="nx-system-status-label">RAM</span>
                  <span className="nx-system-status-val">
                    {(localStats.ram_used_mb / 1024).toFixed(1)} GB / {(localStats.ram_total_mb / 1024).toFixed(0)} GB
                  </span>
                </div>
                <div className="nx-system-status-bar">
                  <div
                    className="nx-system-status-bar-fill"
                    style={{
                      width: `${Math.min(100, (localStats.ram_used_mb / Math.max(1, localStats.ram_total_mb)) * 100)}%`,
                    }}
                  />
                </div>
              </div>
              <div className="nx-system-status-row-head">
                <span className="nx-system-status-label">Netzwerk</span>
                <div className="nx-system-status-net-col">
                  <span className="nx-system-status-net-line">
                    <span className="nx-system-status-net-arrow">↑</span>
                    <span className="nx-system-status-val">{localStats.net_up_kbps.toFixed(0)} KB/s</span>
                  </span>
                  <span className="nx-system-status-net-line">
                    <span className="nx-system-status-net-arrow">↓</span>
                    <span className="nx-system-status-val">{localStats.net_down_kbps.toFixed(0)} KB/s</span>
                  </span>
                </div>
              </div>
            </div>
          )}

          {appVersion && (
            <button className="nx-version nx-version-btn" onClick={() => setShowPatchNotes(true)}>
              v{appVersion}
            </button>
          )}
        </div>
      </aside>

      <section className="nx-server-list">
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <h2>Deine Server</h2>
          <button className="nx-update-btn" onClick={() => setShowAddDialog(true)}>
            +
          </button>
        </div>
        <input className="nx-search" placeholder="Suchen..." />

        {loading && <div style={{ color: "var(--nx-text-muted)" }}>Lade Server…</div>}
        {!loading && servers.length === 0 && (
          <div style={{ color: "var(--nx-text-muted)", fontSize: 13 }}>
            Noch keine Server verbunden. Füge deinen ersten Server über "+" hinzu.
          </div>
        )}

        {servers.map((server) => {
          const s = stats[server.id];
          return (
            <div
              key={server.id}
              className={`nx-server-card ${server.id === selectedServerId ? "selected" : ""}`}
              onClick={() => setSelectedServerId(server.id)}
            >
              <div className="nx-server-card-head">
                <DistroIcon osInfo={server.os_info} size={34} />
                <div className="nx-server-card-info">
                  <div className="nx-server-card-title">
                    <span>{server.name}</span>
                    <span
                      className={`nx-status-dot ${s ? "" : "nx-pulse"}`}
                      style={{ background: s ? "var(--nx-success)" : "var(--nx-warning)" }}
                    />
                  </div>
                  <div className="nx-server-ip">{server.host}</div>
                </div>
                <div style={{ position: "relative" }}>
                  <button
                    className="nx-icon-btn"
                    onClick={(e) => {
                      e.stopPropagation();
                      setOpenServerMenuId(openServerMenuId === server.id ? null : server.id);
                    }}
                  >
                    ⋯
                  </button>
                  {openServerMenuId === server.id && (
                    <div className="nx-context-menu" onClick={(e) => e.stopPropagation()}>
                      <button onClick={() => { setOpenServerMenuId(null); setEditingServer(server); }}>
                        Server bearbeiten
                      </button>
                      <button onClick={() => { setOpenServerMenuId(null); setSftpServer(server); }}>
                        Vollzugriff (SFTP)
                      </button>
                    </div>
                  )}
                </div>
              </div>
              {s ? (
                <div className="nx-server-meter-row">
                  <div className="nx-server-meter">
                    <span>CPU {s.cpu_percent.toFixed(0)}%</span>
                    <div className="nx-meter-track">
                      <div className="nx-meter-fill" style={{ width: `${Math.min(100, s.cpu_percent)}%` }} />
                    </div>
                  </div>
                  <div className="nx-server-meter">
                    <span>RAM {Math.round((s.ram_used_mb / Math.max(1, s.ram_total_mb)) * 100)}%</span>
                    <div className="nx-meter-track">
                      <div className="nx-meter-fill" style={{ width: `${Math.min(100, (s.ram_used_mb / Math.max(1, s.ram_total_mb)) * 100)}%` }} />
                    </div>
                  </div>
                </div>
              ) : (
                <div style={{ fontSize: 12, color: "var(--nx-text-muted)" }}>Verbinde…</div>
              )}
            </div>
          );
        })}
      </section>

      <main className="nx-main">
        {selectedServer ? (
          <div>
            <div className="nx-server-header">
              <div className="nx-server-header-icon">
                <DistroIcon osInfo={selectedServer.os_info} size={28} />
              </div>
              <div className="nx-server-header-info">
                <h1>{selectedServer.name}</h1>
                <p>
                  {selectedServer.host}:{selectedServer.port}
                  {selectedServer.os_info ? ` · ${selectedServer.os_info}` : ""}
                </p>
              </div>
              <span className={`nx-conn-pill ${isConnected ? "connected" : ""}`}>
                <span className="nx-status-dot" /> {isConnected ? "SSH Verbunden" : "Nicht verbunden"}
              </span>
              <div className="nx-server-header-actions">
                <button disabled={serverBusy} onClick={handleReload}>
                  {serverBusy ? <span className="nx-spinner" /> : "⟳ "}Neu laden
                </button>
                <button disabled={serverBusy} onClick={handleRebootServer}>
                  {serverBusy ? <span className="nx-spinner" /> : "⏻ "}Neustarten
                </button>
                <button className="nx-btn-danger" disabled={serverBusy} onClick={handleDisconnectServer}>
                  ⏏ Trennen
                </button>
              </div>
            </div>

            {instanceError && (
              <div className="nx-update-error" style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 12 }}>
                <span>{instanceError}</span>
                <button
                  onClick={() => setInstanceError("")}
                  style={{ background: "transparent", border: "none", color: "inherit", fontSize: 16, lineHeight: 1, cursor: "pointer" }}
                  aria-label="Fehlermeldung schließen"
                >
                  ×
                </button>
              </div>
            )}

            {!openInstanceId && (
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginTop: 20 }}>
              <h3 style={{ margin: 0 }}>Installierte Gameserver</h3>
              <div style={{ display: "flex", gap: 8 }}>
                <button className="nx-btn-restart" disabled={discovering} onClick={discoverInstances}>
                  {discovering ? "Suche…" : "Vorhandene Server suchen"}
                </button>
                <button className="nx-update-btn" onClick={() => setShowStoreDialog(true)}>
                  + Neues Spiel installieren
                </button>
              </div>
            </div>
            )}
            {!openInstanceId && discoverError && <p style={{ color: "var(--nx-danger)", fontSize: 12 }}>{discoverError}</p>}
            {!openInstanceId && discoverResult !== null && (
              <p style={{ color: "var(--nx-success)", fontSize: 12 }}>
                {discoverResult === 0 ? "Keine neuen Server gefunden." : `${discoverResult} Server(n) gefunden und hinzugefügt.`}
              </p>
            )}
            {!openInstanceId && instances.length === 0 && (
              <p style={{ color: "var(--nx-text-muted)" }}>Noch keine Gameserver installiert.</p>
            )}
            {!openInstanceId && <div className="nx-instance-grid">
              {installingGame && (
                <div className="nx-instance-card nx-instance-card-installing">
                  <div className="nx-instance-card-icon-row">
                    <div className="nx-instance-icon-box">
                      <GameIcon gameId={installingGame.id} size={38} />
                    </div>
                    <div className="nx-instance-card-info">
                      <div className="nx-instance-card-title">{installingGame.name}</div>
                      <div className="nx-instance-card-subtitle">Wird installiert…</div>
                    </div>
                  </div>
                  <div style={{ fontSize: 12, color: "var(--nx-text-muted)" }}>
                    <span className="nx-spinner" />
                    {installProgress || "Download & Einrichtung auf dem Server, kann je nach Spielgröße mehrere Minuten dauern"}
                  </div>
                </div>
              )}
              {instances.map((instance) => {
                const status = instanceStatus[instance.id];
                const isActive = status?.state === "active";
                const isFailed = status?.state === "failed";
                const statusColor = isActive ? "var(--nx-success)" : isFailed ? "var(--nx-danger)" : "var(--nx-text-muted)";
                const isBusy = instanceBusy === instance.id;
                const statusLabel = isBusy
                  ? isActive
                    ? "Wird gestoppt…"
                    : "Wird gestartet…"
                  : isActive
                  ? "Online"
                  : isFailed
                  ? "Fehler"
                  : status
                  ? "Gestoppt"
                  : "Unbekannt";
                return (
                  <div key={instance.id} className="nx-instance-card">
                    <div className="nx-instance-card-icon-row">
                      <div className="nx-instance-icon-box">
                        <GameIcon gameId={instance.game_id} size={38} />
                      </div>
                      <div className="nx-instance-card-info">
                        <div className="nx-instance-card-title">{instance.display_name}</div>
                        {(() => {
                          const version = instanceVersions[instance.id];
                          if (version?.installed) {
                            return (
                              <div className="nx-instance-card-subtitle">
                                v{version.installed}{" "}
                                <span title={version.up_to_date ? "Aktuell" : "Update verfügbar"}>
                                  {version.up_to_date ? "✅" : "⬆️"}
                                </span>
                              </div>
                            );
                          }
                          return (
                            gameSubtitles[instance.game_id] && (
                              <div className="nx-instance-card-subtitle">{gameSubtitles[instance.game_id]}</div>
                            )
                          );
                        })()}
                      </div>
                      <label className={`nx-toggle ${instanceBusy === instance.id ? "busy" : ""}`}>
                        <input
                          type="checkbox"
                          checked={isActive}
                          disabled={instanceBusy === instance.id}
                          onChange={() => runInstanceAction(instance, isActive ? "stop" : "start")}
                        />
                        <span className="nx-toggle-slider" />
                      </label>
                    </div>
                    <div style={{ fontSize: 12, color: statusColor, marginBottom: 2 }}>
                      {isBusy ? <span className="nx-spinner" /> : <span className="nx-status-dot" style={{ background: statusColor }} />}
                      {statusLabel}
                    </div>
                    {status && <div className="nx-instance-card-sub">Uptime: {formatUptime(status.uptime_seconds)}</div>}
                    {instanceVersions[instance.id]?.installed && !instanceVersions[instance.id].up_to_date && (
                      <button
                        className="nx-update-available-btn"
                        disabled={updatingInstanceId === instance.id}
                        onClick={() => updateInstance(instance)}
                      >
                        {updatingInstanceId === instance.id
                          ? "Aktualisiert…"
                          : `Update auf v${instanceVersions[instance.id].latest}`}
                      </button>
                    )}
                    <div className="nx-instance-actions" style={{ marginTop: 10 }}>
                      <button
                        style={{ flex: 1 }}
                        onClick={() => setOpenInstanceId(openInstanceId === instance.id ? null : instance.id)}
                      >
                        Verwalten
                      </button>
                      <div style={{ position: "relative" }}>
                        <button
                          className="nx-icon-btn"
                          onClick={(e) => {
                            e.stopPropagation();
                            setOpenMenuId(openMenuId === instance.id ? null : instance.id);
                          }}
                        >
                          ⋯
                        </button>
                        {openMenuId === instance.id && (
                          <div className="nx-context-menu" onClick={(e) => e.stopPropagation()}>
                            <button disabled={instanceBusy === instance.id} onClick={() => { setOpenMenuId(null); runInstanceAction(instance, "restart"); }}>
                              Neustart
                            </button>
                            <button onClick={() => { setOpenMenuId(null); setBrowsingInstance({ instance, target: "install" }); }}>
                              Hauptverzeichnis öffnen
                            </button>
                            <button onClick={() => { setOpenMenuId(null); setBrowsingInstance({ instance, target: "backups" }); }}>
                              Backup-Ordner öffnen
                            </button>
                            <button disabled={instanceBusy === instance.id} onClick={() => { setOpenMenuId(null); handleForget(instance); }}>
                              Entfernen
                            </button>
                            <button
                              className="nx-btn-danger"
                              disabled={instanceBusy === instance.id}
                              onClick={() => { setOpenMenuId(null); handleUninstall(instance); }}
                            >
                              Deinstallieren
                            </button>
                          </div>
                        )}
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>}

            {!openInstanceId && showStoreDialog && selectedServerId && (
              <GameStoreDialog
                serverId={selectedServerId}
                onClose={() => setShowStoreDialog(false)}
                onInstalled={(instance) => {
                  setInstances((prev) => [...prev, instance]);
                  setShowStoreDialog(false);
                }}
                onInstallStart={(game) => {
                  setInstallProgress("");
                  setInstallingGame(game);
                }}
                onInstallProgress={setInstallProgress}
                onInstallDone={() => {
                  setInstallingGame(null);
                  setInstallProgress("");
                }}
              />
            )}

            {openInstanceId &&
              (() => {
                const instance = instances.find((i) => i.id === openInstanceId);
                return instance ? (
                  <InstanceDetail
                    serverId={selectedServer.id}
                    instance={instance}
                    status={instanceStatus[instance.id]}
                    busy={instanceBusy === instance.id}
                    cpuHistory={cpuHistory[selectedServer.id] ?? []}
                    ramHistory={ramHistory[selectedServer.id] ?? []}
                    diskUsedGb={selectedStats?.disk_used_gb}
                    diskTotalGb={selectedStats?.disk_total_gb}
                    subtitle={
                      instanceVersions[instance.id]?.installed
                        ? `v${instanceVersions[instance.id].installed}`
                        : gameSubtitles[instance.game_id]
                    }
                    configSchema={gameTemplates[instance.game_id]?.config}
                    otherInstancesRamMb={instances
                      .filter((i) => i.id !== instance.id)
                      .reduce((sum, i) => sum + i.ram_limit_mb, 0)}
                    otherInstancesCpuPercent={instances
                      .filter((i) => i.id !== instance.id)
                      .reduce((sum, i) => sum + i.cpu_limit_percent, 0)}
                    onAction={(action) => runInstanceAction(instance, action)}
                    onClose={() => setOpenInstanceId(null)}
                  />
                ) : null;
              })()}
          </div>
        ) : (
          <div className="nx-empty-state">
            <div>Kein Server ausgewählt</div>
          </div>
        )}
      </main>

      {showAddDialog && (
        <AddServerDialog
          onClose={() => setShowAddDialog(false)}
          onCreated={(server) => {
            setServers((prev) => [...prev, server]);
            setSelectedServerId(server.id);
            setShowAddDialog(false);
            invoke<boolean>("check_firewall_active", { serverId: server.id })
              .then((active) => {
                if (!active) setFirewallPromptServer(server);
              })
              .catch(() => {});
          }}
        />
      )}

      {firewallPromptServer && (
        <FirewallPromptDialog
          serverName={firewallPromptServer.name}
          serverId={firewallPromptServer.id}
          onClose={() => setFirewallPromptServer(null)}
        />
      )}

      {editingServer && (
        <EditServerDialog
          server={editingServer}
          onClose={() => setEditingServer(null)}
          onUpdated={(updated) => {
            setServers((prev) => prev.map((s) => (s.id === updated.id ? updated : s)));
            setEditingServer(null);
          }}
        />
      )}

      {sftpServer && (
        <SftpBrowserDialog serverId={sftpServer.id} serverName={sftpServer.name} onClose={() => setSftpServer(null)} />
      )}

      {showPatchNotes && <PatchNotesDialog onClose={() => setShowPatchNotes(false)} />}

      {browsingInstance && selectedServerId && (
        <DirectoryBrowserDialog
          serverId={selectedServerId}
          instanceId={browsingInstance.instance.id}
          instanceName={browsingInstance.instance.display_name}
          target={browsingInstance.target}
          onClose={() => setBrowsingInstance(null)}
        />
      )}
    </div>
  );
}

export default App;
