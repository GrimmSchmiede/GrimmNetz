import { useEffect, useRef, useState } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type { BackupEntry, ConfigSchema, InstanceRecord, InstanceStatus, MinecraftLiveStatus } from "./types";

type LogEvent = { event: "line"; text: string } | { event: "closed" };

const LOG_LEVEL_COLORS: Record<string, string> = {
  ERROR: "var(--nx-danger)",
  ERR: "var(--nx-danger)",
  FATAL: "var(--nx-danger)",
  SEVERE: "var(--nx-danger)",
  WARN: "#facc15",
  WARNING: "#facc15",
  INFO: "var(--nx-success)",
};

// Log formats differ wildly per game, so instead of strictly parsing columns, just
// highlight the first recognizable level keyword wherever it appears in the line -
// robust across Paper's "[HH:MM:SS INFO]:", plain "[WARN]", "ERROR:", etc.
function highlightLogLine(line: string) {
  const match = line.match(/\b(ERROR|ERR|FATAL|SEVERE|WARNING|WARN|INFO)\b/);
  if (!match || match.index == null) return line;
  const color = LOG_LEVEL_COLORS[match[0]];
  const before = line.slice(0, match.index);
  const after = line.slice(match.index + match[0].length);
  return (
    <>
      {before}
      <span style={{ color, fontWeight: 600 }}>{match[0]}</span>
      {after}
    </>
  );
}

type Props = {
  serverId: string;
  instance: InstanceRecord;
  status?: InstanceStatus;
  busy: boolean;
  cpuHistory: number[];
  ramHistory: number[];
  diskUsedGb?: number;
  diskTotalGb?: number;
  subtitle?: string;
  configSchema?: ConfigSchema;
  otherInstancesRamMb?: number;
  otherInstancesCpuPercent?: number;
  onAction: (action: "start" | "stop" | "restart") => void;
  onClose: () => void;
};

// Rough hosting-practice rule of thumb per game - base RAM for an empty server plus a per-slot
// cost, read from whatever the config schema calls the player-slot field. Only ever shown as a
// non-blocking hint text, never enforced - a modded server can need far more than this guesses.
const RAM_RECOMMENDATION: Record<string, { slotField: string; baseMb: number; perSlotMb: number }> = {
  "minecraft-paper": { slotField: "max-players", baseMb: 3072, perSlotMb: 200 },
  "7dtd": { slotField: "ServerMaxPlayerCount", baseMb: 6144, perSlotMb: 300 },
};

// Step function instead of linear - most game server engines don't scale cleanly across many
// cores, so "more cores = better" isn't true past a point. Tiers are [maxSlots, cores], sorted
// ascending; the first tier whose maxSlots >= the configured slot count applies, otherwise the
// last (highest) tier.
const CPU_RECOMMENDATION: Record<string, { slotField: string; tiers: [number, number][] }> = {
  "minecraft-paper": { slotField: "max-players", tiers: [[5, 2], [14, 3], [Infinity, 4]] },
  "7dtd": { slotField: "ServerMaxPlayerCount", tiers: [[4, 4], [Infinity, 6]] },
};

export default function InstanceDetail({
  serverId,
  instance,
  status,
  busy,
  cpuHistory,
  ramHistory,
  diskUsedGb,
  diskTotalGb,
  subtitle,
  configSchema,
  otherInstancesRamMb = 0,
  otherInstancesCpuPercent = 0,
  onAction,
  onClose,
}: Props) {
  const [tab, setTab] = useState<"status" | "config" | "management" | "console" | "backups">("status");
  const [backups, setBackups] = useState<BackupEntry[]>([]);
  const [backupsLoading, setBackupsLoading] = useState(false);
  const [backupCreating, setBackupCreating] = useState(false);
  const [backupBusyName, setBackupBusyName] = useState<string | null>(null);
  const [backupError, setBackupError] = useState("");
  const [backupSavedPath, setBackupSavedPath] = useState("");
  const [uploadProgress, setUploadProgress] = useState<number | null>(null);
  const [mcLiveStatus, setMcLiveStatus] = useState<MinecraftLiveStatus | null>(null);
  const [configValues, setConfigValues] = useState<Record<string, string>>({});
  const [configLoading, setConfigLoading] = useState(false);
  const [configSaving, setConfigSaving] = useState(false);
  const [configError, setConfigError] = useState("");
  const [configSaved, setConfigSaved] = useState(false);
  const [hostCapacity, setHostCapacity] = useState<{ cpuCores: number; ramTotalMb: number } | null>(null);
  const [cpuLimitPercent, setCpuLimitPercent] = useState(instance.cpu_limit_percent);
  const [ramLimitMb, setRamLimitMb] = useState(instance.ram_limit_mb);
  const [resourcesSaving, setResourcesSaving] = useState(false);
  const [resourcesSaved, setResourcesSaved] = useState(false);
  const [resourcesError, setResourcesError] = useState("");
  const [scheduledRestarts, setScheduledRestarts] = useState<{ id: string; time: string }[]>([]);
  const [restartsLoading, setRestartsLoading] = useState(false);
  const [newRestartTime, setNewRestartTime] = useState("04:00");
  const [restartAdding, setRestartAdding] = useState(false);
  const [restartError, setRestartError] = useState("");
  const [broadcastMessage, setBroadcastMessage] = useState("");
  const [broadcastDelay, setBroadcastDelay] = useState("5");
  const [broadcastAction, setBroadcastAction] = useState<"restart" | "stop" | null>(null);
  const [broadcastSent, setBroadcastSent] = useState<"restart" | "stop" | null>(null);
  const [broadcastError, setBroadcastError] = useState("");
  const [lines, setLines] = useState<string[]>([]);
  const [autoScroll, setAutoScroll] = useState(true);
  const [logAttempt, setLogAttempt] = useState(0);
  const [logError, setLogError] = useState(false);
  const consoleRef = useRef<HTMLDivElement>(null);
  const startedForAttempt = useRef(-1);

  const isActive = status?.state === "active";
  const isFailed = status?.state === "failed";
  const statusColor = isActive ? "var(--nx-success)" : isFailed ? "var(--nx-danger)" : "var(--nx-text-muted)";
  const statusLabel = isActive ? "Online" : isFailed ? "Fehler" : status ? "Gestoppt" : "Unbekannt";

  function formatStartedAt(raw: string | null | undefined): string {
    if (!raw) return "–";
    const d = new Date(raw);
    if (isNaN(d.getTime())) return raw;
    return d.toLocaleString("de-DE", { day: "2-digit", month: "2-digit", year: "numeric", hour: "2-digit", minute: "2-digit" });
  }

  function formatUptimeShort(seconds: number): string {
    if (seconds <= 0) return "–";
    const days = Math.floor(seconds / 86400);
    const hours = Math.floor((seconds % 86400) / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    const secs = Math.floor(seconds % 60);
    if (days > 0) return `${days}T ${hours}h ${minutes}m ${secs}s`;
    if (hours > 0) return `${hours}h ${minutes}m ${secs}s`;
    return `${minutes}m ${secs}s`;
  }

  useEffect(() => {
    // Streams for the whole lifetime of the detail view (not gated by which tab is active) -
    // Status & Ressourcen and Live-Konsole both show the same live log buffer, no reason to
    // tear down and reconnect when switching tabs.
    if (startedForAttempt.current === logAttempt) return;
    startedForAttempt.current = logAttempt;
    setLogError(false);

    const channel = new Channel<LogEvent>();
    channel.onmessage = (event) => {
      if (event.event === "line") {
        setLines((prev) => [...prev.slice(-499), event.text]);
      }
    };

    invoke("stream_instance_logs", {
      serverId,
      unitName: instance.systemd_unit,
      onEvent: channel,
    }).catch((err) => {
      setLines((prev) => [...prev, `[Fehler] ${String(err)}`]);
      setLogError(true);
    });
  }, [logAttempt, serverId, instance.systemd_unit]);

  function retryLogs() {
    setLines([]);
    setLogAttempt((n) => n + 1);
  }

  function loadBackups() {
    setBackupsLoading(true);
    setBackupError("");
    invoke<BackupEntry[]>("list_backups", { serverId, instanceId: instance.id })
      .then(setBackups)
      .catch((err) => setBackupError(String(err)))
      .finally(() => setBackupsLoading(false));
  }

  useEffect(() => {
    if (tab === "backups") loadBackups();
  }, [tab, serverId, instance.id]);

  async function createBackup() {
    setBackupCreating(true);
    setBackupError("");
    try {
      await invoke("create_backup", {
        serverId,
        instanceId: instance.id,
        installPath: instance.install_path,
      });
      loadBackups();
    } catch (err) {
      setBackupError(String(err));
    } finally {
      setBackupCreating(false);
    }
  }

  async function uploadBackup() {
    const defaultPath = await invoke<string>("get_local_backup_dir", { instanceId: instance.id }).catch(() => undefined);
    const path = await open({
      multiple: false,
      defaultPath,
      filters: [{ name: "Backup", extensions: ["gz"] }],
    });
    if (!path || Array.isArray(path)) return;

    setBackupCreating(true);
    setBackupError("");
    setUploadProgress(0);
    try {
      const channel = new Channel<{ event: "progress"; bytesSent: number; totalBytes: number }>();
      channel.onmessage = (event) => {
        if (event.event === "progress" && event.totalBytes > 0) {
          setUploadProgress(Math.round((event.bytesSent / event.totalBytes) * 100));
        }
      };
      await invoke("upload_backup", { serverId, instanceId: instance.id, localPath: path, onProgress: channel });
      loadBackups();
    } catch (err) {
      setBackupError(String(err));
    } finally {
      setBackupCreating(false);
      setUploadProgress(null);
    }
  }

  async function downloadBackup(name: string) {
    setBackupBusyName(name);
    setBackupError("");
    setBackupSavedPath("");
    try {
      const path = await invoke<string>("download_backup", { serverId, instanceId: instance.id, filename: name });
      setBackupSavedPath(path);
    } catch (err) {
      setBackupError(String(err));
    } finally {
      setBackupBusyName(null);
    }
  }

  async function deleteBackup(name: string) {
    if (!confirm(`Backup "${name}" auf dem Server löschen?`)) return;
    setBackupBusyName(name);
    setBackupError("");
    try {
      await invoke("delete_backup", { serverId, instanceId: instance.id, filename: name });
      setBackups((prev) => prev.filter((b) => b.name !== name));
    } catch (err) {
      setBackupError(String(err));
    } finally {
      setBackupBusyName(null);
    }
  }

  async function restoreBackup(name: string) {
    if (
      !confirm(
        `Backup "${name}" wiederherstellen? Der aktuelle Stand von "${instance.display_name}" wird dabei komplett überschrieben und der Server kurz neugestartet. Das kann nicht rückgängig gemacht werden.`
      )
    )
      return;
    setBackupBusyName(name);
    setBackupError("");
    try {
      await invoke("restore_backup", {
        serverId,
        instanceId: instance.id,
        installPath: instance.install_path,
        unitName: instance.systemd_unit,
        filename: name,
      });
    } catch (err) {
      setBackupError(String(err));
    } finally {
      setBackupBusyName(null);
    }
  }

  function formatBackupSize(bytes: number): string {
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
    return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
  }

  function formatBackupDate(unixSeconds: number): string {
    return new Date(unixSeconds * 1000).toLocaleString("de-DE", {
      day: "2-digit",
      month: "2-digit",
      year: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  }

  useEffect(() => {
    if (tab !== "management") return;
    invoke<{ cpu_cores: number; ram_total_mb: number }>("get_host_capacity", { serverId })
      .then((cap) => setHostCapacity({ cpuCores: cap.cpu_cores, ramTotalMb: cap.ram_total_mb }))
      .catch(() => {});
    setRestartsLoading(true);
    setRestartError("");
    invoke<{ id: string; time: string }[]>("list_scheduled_restarts", { serverId, unitName: instance.systemd_unit })
      .then(setScheduledRestarts)
      .catch((err) => setRestartError(String(err)))
      .finally(() => setRestartsLoading(false));
  }, [tab, serverId, instance.systemd_unit]);

  async function addRestart() {
    setRestartAdding(true);
    setRestartError("");
    try {
      await invoke("add_scheduled_restart", { serverId, unitName: instance.systemd_unit, time: newRestartTime });
      const updated = await invoke<{ id: string; time: string }[]>("list_scheduled_restarts", {
        serverId,
        unitName: instance.systemd_unit,
      });
      setScheduledRestarts(updated);
    } catch (err) {
      setRestartError(String(err));
    } finally {
      setRestartAdding(false);
    }
  }

  async function sendBroadcastAction(action: "restart" | "stop") {
    setBroadcastAction(action);
    setBroadcastError("");
    setBroadcastSent(null);
    try {
      await invoke("broadcast_and_execute", {
        serverId,
        unitName: instance.systemd_unit,
        message: broadcastMessage,
        delayMinutes: Number(broadcastDelay),
        action,
      });
      setBroadcastSent(action);
      setBroadcastMessage("");
    } catch (err) {
      setBroadcastError(String(err));
    } finally {
      setBroadcastAction(null);
    }
  }

  async function removeRestart(id: string) {
    setRestartError("");
    try {
      await invoke("remove_scheduled_restart", { serverId, unitName: instance.systemd_unit, id });
      setScheduledRestarts((prev) => prev.filter((r) => r.id !== id));
    } catch (err) {
      setRestartError(String(err));
    }
  }

  async function saveResources() {
    setResourcesSaving(true);
    setResourcesError("");
    setResourcesSaved(false);
    try {
      await invoke("update_instance_resources", {
        serverId,
        instanceId: instance.id,
        unitName: instance.systemd_unit,
        installPath: instance.install_path,
        cpuLimitPercent,
        ramLimitMb,
      });
      setResourcesSaved(true);
    } catch (err) {
      setResourcesError(String(err));
    } finally {
      setResourcesSaving(false);
    }
  }

  useEffect(() => {
    if ((tab !== "config" && tab !== "management") || !configSchema) return;
    setConfigLoading(true);
    setConfigError("");
    invoke<Record<string, string>>("get_instance_config", {
      serverId,
      gameId: instance.game_id,
      installPath: instance.install_path,
    })
      .then(setConfigValues)
      .catch((err) => setConfigError(String(err)))
      .finally(() => setConfigLoading(false));
  }, [tab, configSchema, serverId, instance.game_id, instance.install_path]);

  async function saveConfig() {
    setConfigSaving(true);
    setConfigError("");
    setConfigSaved(false);
    try {
      await invoke("save_instance_config", {
        serverId,
        gameId: instance.game_id,
        installPath: instance.install_path,
        values: configValues,
      });
      setConfigSaved(true);
    } catch (err) {
      setConfigError(String(err));
    } finally {
      setConfigSaving(false);
    }
  }

  useEffect(() => {
    if (autoScroll && consoleRef.current) {
      consoleRef.current.scrollTop = consoleRef.current.scrollHeight;
    }
  }, [lines, autoScroll]);

  function renderLogPanel() {
    return (
      <div className="nx-log-panel">
        <div className="nx-console" ref={consoleRef}>
          {lines.length === 0 && (
            <div style={{ color: "var(--nx-text-muted)", margin: "auto" }}>Warte auf Log-Ausgabe…</div>
          )}
          {lines.map((line, i) => (
            <div key={i} className="nx-log-line" title={line}>
              {highlightLogLine(line)}
            </div>
          ))}
        </div>
        <div
          className="nx-log-footer"
          style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginTop: 8 }}
        >
          <label style={{ fontSize: 12, color: "var(--nx-text-muted)" }}>
            <input type="checkbox" checked={autoScroll} onChange={(e) => setAutoScroll(e.target.checked)} /> Automatisch
            scrollen
          </label>
          <div style={{ display: "flex", gap: 8 }}>
            {logError && (
              <button className="nx-icon-btn" onClick={retryLogs}>
                ⟳ Erneut verbinden
              </button>
            )}
            <button className="nx-icon-btn" onClick={() => setLines([])} title="Löscht nur die Ansicht, nicht die Server-Logs">
              Log leeren
            </button>
          </div>
        </div>
      </div>
    );
  }

  useEffect(() => {
    if (instance.game_id !== "minecraft-paper") return;
    const poll = () => {
      invoke<MinecraftLiveStatus>("get_minecraft_live_status", {
        serverId,
        installPath: instance.install_path,
      })
        .then(setMcLiveStatus)
        .catch(() => {});
    };
    poll();
    const interval = setInterval(poll, 10000);
    return () => clearInterval(interval);
  }, [serverId, instance.game_id, instance.install_path]);

  return (
    <div className="nx-instance-detail">
      <div className="nx-instance-detail-header">
        <button className="nx-back-btn" onClick={onClose}>
          ← Zurück
        </button>
        <h2 style={{ margin: 0 }}>
          {instance.display_name}
          {subtitle && <span style={{ fontWeight: 400, color: "var(--nx-text-muted)" }}> ({subtitle})</span>}
        </h2>
        <span style={{ fontSize: 12, color: statusColor }}>
          <span className="nx-status-dot" style={{ background: statusColor }} /> {statusLabel}
        </span>
        <div style={{ marginLeft: "auto", display: "flex", gap: 8 }}>
          {isActive ? (
            <button className="nx-btn-stop" disabled={busy} onClick={() => onAction("stop")}>
              ⏸ Stoppen
            </button>
          ) : (
            <button className="nx-btn-start" disabled={busy} onClick={() => onAction("start")}>
              ▶ Starten
            </button>
          )}
          <button className="nx-btn-restart" disabled={busy} onClick={() => onAction("restart")}>
            ⟳ Neustarten
          </button>
        </div>
      </div>

      <div className="nx-tabs">
        <button className={`nx-tab ${tab === "status" ? "active" : ""}`} onClick={() => setTab("status")}>
          Status & Ressourcen
        </button>
        <button className={`nx-tab ${tab === "config" ? "active" : ""}`} onClick={() => setTab("config")}>
          Konfiguration
        </button>
        <button className={`nx-tab ${tab === "management" ? "active" : ""}`} onClick={() => setTab("management")}>
          Verwaltung
        </button>
        <button className={`nx-tab ${tab === "console" ? "active" : ""}`} onClick={() => setTab("console")}>
          Live-Konsole
        </button>
        <button className={`nx-tab ${tab === "backups" ? "active" : ""}`} onClick={() => setTab("backups")}>
          Backups
        </button>
      </div>

      {tab === "status" && (
        <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
          <div style={{ display: "flex", alignItems: "stretch", gap: 16 }}>
            <div className="nx-status-layout-main">
              <div className="nx-status-grid">
                <div className="nx-chart-card">
                  <div className="nx-chart-card-head">
                    <div className="nx-chart-title">CPU Auslastung</div>
                    <select className="nx-range-select" disabled defaultValue="1h">
                      <option value="1h">1 Stunde</option>
                    </select>
                  </div>
                  <div className="nx-chart-value">{(cpuHistory[cpuHistory.length - 1] ?? 0).toFixed(0)}%</div>
                  <Sparkline values={cpuHistory} max={Math.max(100, instance.cpu_limit_percent)} />
                </div>
                <div className="nx-chart-card">
                  <div className="nx-chart-card-head">
                    <div className="nx-chart-title">RAM Auslastung</div>
                    <select className="nx-range-select" disabled defaultValue="1h">
                      <option value="1h">1 Stunde</option>
                    </select>
                  </div>
                  <div className="nx-chart-value">
                    {((ramHistory[ramHistory.length - 1] ?? 0) / 1024).toFixed(1)} GB
                    <span className="nx-chart-value-max"> / {(instance.ram_limit_mb / 1024).toFixed(0)} GB</span>
                  </div>
                  <Sparkline values={ramHistory} max={instance.ram_limit_mb} />
                </div>
                <div className="nx-fact-card">
                  <div className="nx-fact-row">
                    <span>Prozess ID</span>
                    <span>{status?.pid ?? "–"}</span>
                  </div>
                  <div className="nx-fact-row" title={instance.game_id !== "minecraft-paper" ? "Nur für Minecraft verfügbar" : undefined}>
                    <span>Spieler Online</span>
                    <span>
                      {mcLiveStatus?.players_online != null ? `${mcLiveStatus.players_online} / ${mcLiveStatus.players_max}` : "–"}
                    </span>
                  </div>
                  <div className="nx-fact-row" title={instance.game_id !== "minecraft-paper" ? "Nur für Minecraft verfügbar" : undefined}>
                    <span>Welt</span>
                    <span>{mcLiveStatus?.world ?? "–"}</span>
                  </div>
                  <div className="nx-fact-row">
                    <span>Startzeit</span>
                    <span>{formatStartedAt(status?.started_at)}</span>
                  </div>
                  <div className="nx-fact-row">
                    <span>Laufzeit</span>
                    <span>{formatUptimeShort(status?.uptime_seconds ?? 0)}</span>
                  </div>
                </div>
              </div>
            </div>

            <div className="nx-resource-bar-card nx-resource-bar-card-compact" style={{ width: 320, flexShrink: 0 }}>
              <div className="nx-resource-bar">
                <div className="nx-resource-bar-item">
                  <span className="nx-resource-bar-icon-box">⚙️</span>
                  <div>
                    <div className="nx-resource-bar-label">CPU Limit</div>
                    <div className="nx-resource-bar-value">{instance.cpu_limit_percent}%</div>
                  </div>
                </div>
                <div className="nx-resource-bar-item">
                  <span className="nx-resource-bar-icon-box">🧠</span>
                  <div>
                    <div className="nx-resource-bar-label">RAM Limit</div>
                    <div className="nx-resource-bar-value">{instance.ram_limit_mb} MB</div>
                  </div>
                </div>
                {diskTotalGb !== undefined && diskTotalGb > 0 && (
                  <div className="nx-resource-bar-item">
                    <span className="nx-resource-bar-icon-box">💾</span>
                    <div style={{ flex: 1 }}>
                      <div className="nx-resource-bar-label">Speicherplatz</div>
                      <div className="nx-resource-bar-value">
                        {diskUsedGb} GB / {diskTotalGb} GB
                      </div>
                      <div className="nx-disk-bar">
                        <div
                          className="nx-disk-bar-fill"
                          style={{ width: `${Math.min(100, ((diskUsedGb ?? 0) / diskTotalGb) * 100)}%` }}
                        />
                      </div>
                    </div>
                  </div>
                )}
              </div>
            </div>
          </div>

          {renderLogPanel()}
        </div>
      )}

      {tab === "config" && (
        <div className="nx-config-form">
          {!configSchema && (
            <p style={{ color: "var(--nx-text-muted)" }}>
              Für {instance.display_name} gibt es noch keine Konfigurationsoberfläche. Folgt in einem späteren Update.
            </p>
          )}
          {configSchema && configLoading && (
            <p style={{ color: "var(--nx-text-muted)" }}>Lade Konfiguration…</p>
          )}
          {configSchema && !configLoading && (
            <>
              {configSchema.fields.map((field) => (
                <label key={field.key} className="nx-config-field">
                  <span className="nx-config-label">{field.label}</span>
                  {field.type === "bool" ? (
                    <input
                      type="checkbox"
                      checked={configValues[field.key] === "true"}
                      onChange={(e) =>
                        setConfigValues((prev) => ({ ...prev, [field.key]: e.target.checked ? "true" : "false" }))
                      }
                    />
                  ) : field.type === "select" && field.options ? (
                    <select
                      value={configValues[field.key] ?? ""}
                      onChange={(e) => setConfigValues((prev) => ({ ...prev, [field.key]: e.target.value }))}
                    >
                      {field.options.map((opt) => (
                        <option key={opt.value} value={opt.value}>
                          {opt.label}
                        </option>
                      ))}
                    </select>
                  ) : (
                    <input
                      type={field.type === "password" ? "password" : field.type === "number" ? "number" : "text"}
                      value={configValues[field.key] ?? ""}
                      onChange={(e) => setConfigValues((prev) => ({ ...prev, [field.key]: e.target.value }))}
                    />
                  )}
                  {field.hint && (
                    <span style={{ color: "var(--nx-text-muted)", fontSize: 11, marginTop: 2 }}>{field.hint}</span>
                  )}
                </label>
              ))}
              <div style={{ display: "flex", alignItems: "center", gap: 12, marginTop: 8 }}>
                <button className="nx-btn-restart" disabled={configSaving} onClick={saveConfig}>
                  {configSaving ? "Speichert…" : "Speichern"}
                </button>
                {configSaved && <span style={{ color: "var(--nx-success)", fontSize: 12 }}>Gespeichert ✓</span>}
                <span style={{ color: "var(--nx-text-muted)", fontSize: 12 }}>
                  Wird beim nächsten Neustart des Servers wirksam.
                </span>
              </div>
              {configError && <p style={{ color: "var(--nx-danger)", fontSize: 12 }}>{configError}</p>}
            </>
          )}
        </div>
      )}

      {tab === "management" && (
        <div>
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 24, marginBottom: 24, paddingBottom: 24, borderBottom: "1px solid var(--nx-border)" }}>
            <div className="nx-config-field">
              <span className="nx-config-label" style={{ fontWeight: 600, color: "var(--nx-text)" }}>
                CPU
              </span>
              {(() => {
                const rec = CPU_RECOMMENDATION[instance.game_id];
                const slots = rec ? Number(configValues[rec.slotField]) : NaN;
                if (!rec || !slots || Number.isNaN(slots)) return null;
                const recommendedCores = rec.tiers.find(([maxSlots]) => slots <= maxSlots)?.[1] ?? rec.tiers[rec.tiers.length - 1][1];
                const recommendedPercent = recommendedCores * 100;
                return (
                  <div style={{ fontSize: 11, color: "var(--nx-text-muted)", marginTop: 6 }}>
                    💡 Empfohlene CPU für {slots} Spieler: mind. {recommendedCores.toFixed(1)} Kerne
                    {cpuLimitPercent < recommendedPercent && (
                      <span style={{ color: "#facc15" }}> — aktuell zugewiesen liegt darunter, kann zu Tick-Lags führen</span>
                    )}
                  </div>
                );
              })()}
              {hostCapacity && cpuLimitPercent + otherInstancesCpuPercent > hostCapacity.cpuCores * 100 && (
                <div style={{ fontSize: 11, color: "var(--nx-danger)", marginTop: 6 }}>
                  ⚠️ Zusammen mit den anderen Servern ({(otherInstancesCpuPercent / 100).toFixed(1)} Kerne) übersteigt das
                  die verfügbaren Kerne ({hostCapacity.cpuCores}) — kann andere Server beeinträchtigen.
                </div>
              )}
              <label style={{ display: "flex", flexDirection: "column", gap: 4, marginTop: 10 }}>
                <span>
                  Prozessorkerne: {(cpuLimitPercent / 100).toFixed(1)}
                  {hostCapacity && <span style={{ color: "var(--nx-text-muted)" }}> / {hostCapacity.cpuCores} Kerne verfügbar</span>}
                </span>
                <input
                  type="range"
                  min={50}
                  max={(hostCapacity?.cpuCores ?? Math.max(cpuLimitPercent / 100, 4)) * 100}
                  step={50}
                  value={cpuLimitPercent}
                  onChange={(e) => setCpuLimitPercent(Number(e.target.value))}
                />
              </label>
            </div>

            <div className="nx-config-field">
              <span className="nx-config-label" style={{ fontWeight: 600, color: "var(--nx-text)" }}>
                RAM
              </span>
              {(() => {
                const rec = RAM_RECOMMENDATION[instance.game_id];
                const slots = rec ? Number(configValues[rec.slotField]) : NaN;
                if (!rec || !slots || Number.isNaN(slots)) return null;
                const recommendedMb = rec.baseMb + slots * rec.perSlotMb;
                return (
                  <div style={{ fontSize: 11, color: "var(--nx-text-muted)", marginTop: 6 }}>
                    💡 Empfohlener RAM für {slots} Spieler: mind. {(recommendedMb / 1024).toFixed(1)} GB
                    {ramLimitMb < recommendedMb && (
                      <span style={{ color: "#facc15" }}> — aktuell zugewiesen liegt darunter, kann zu Lags führen</span>
                    )}
                  </div>
                );
              })()}
              {hostCapacity && ramLimitMb + otherInstancesRamMb > hostCapacity.ramTotalMb && (
                <div style={{ fontSize: 11, color: "var(--nx-danger)", marginTop: 6 }}>
                  ⚠️ Zusammen mit den anderen Servern auf diesem Host ({(otherInstancesRamMb / 1024).toFixed(1)} GB) übersteigt das
                  den verfügbaren RAM ({(hostCapacity.ramTotalMb / 1024).toFixed(0)} GB) — kann andere Server beeinträchtigen.
                </div>
              )}
              <label style={{ display: "flex", flexDirection: "column", gap: 4, marginTop: 10 }}>
                <span>
                  Arbeitsspeicher: {(ramLimitMb / 1024).toFixed(1)} GB
                  {hostCapacity && <span style={{ color: "var(--nx-text-muted)" }}> / {(hostCapacity.ramTotalMb / 1024).toFixed(0)} GB verfügbar</span>}
                </span>
                <input
                  type="range"
                  min={512}
                  max={hostCapacity?.ramTotalMb ?? Math.max(ramLimitMb, 8192)}
                  step={512}
                  value={ramLimitMb}
                  onChange={(e) => setRamLimitMb(Number(e.target.value))}
                />
              </label>
            </div>
          </div>
          <div style={{ display: "flex", alignItems: "center", gap: 12, marginBottom: 24 }}>
            <button className="nx-btn-restart" disabled={resourcesSaving} onClick={saveResources}>
              {resourcesSaving ? "Speichert…" : "Ressourcen speichern"}
            </button>
            {resourcesSaved && <span style={{ color: "var(--nx-success)", fontSize: 12 }}>Gespeichert ✓ (Neustart erfolgt)</span>}
            {resourcesError && <span style={{ color: "var(--nx-danger)", fontSize: 12 }}>{resourcesError}</span>}
          </div>

          <div style={{ marginBottom: 24, paddingBottom: 24, borderBottom: "1px solid var(--nx-border)" }}>
            <span className="nx-config-label" style={{ fontWeight: 600, color: "var(--nx-text)" }}>
              Neustart / Stop mit Ansage
            </span>
            <p style={{ color: "var(--nx-text-muted)", fontSize: 11, marginTop: 6 }}>
              Schickt deine Nachricht sofort ins Spiel (z.B. "Wegen Wartungsarbeiten starten wir den Server gleich neu"),
              wartet die gewählte Zeit, damit Spieler sich abmelden können, und führt die Aktion dann aus.
            </p>
            <textarea
              value={broadcastMessage}
              onChange={(e) => setBroadcastMessage(e.target.value)}
              placeholder="Nachricht an die Spieler…"
              rows={2}
              style={{
                width: "100%",
                marginTop: 10,
                background: "var(--nx-panel-alt)",
                border: "1px solid var(--nx-border)",
                borderRadius: "var(--nx-radius)",
                padding: "8px 10px",
                color: "var(--nx-text)",
                fontFamily: "inherit",
                fontSize: 13,
                resize: "vertical",
              }}
            />
            <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 10 }}>
              <select
                value={broadcastDelay}
                onChange={(e) => setBroadcastDelay(e.target.value)}
                style={{ background: "var(--nx-panel-alt)", border: "1px solid var(--nx-border)", borderRadius: "var(--nx-radius)", padding: "6px 10px", color: "var(--nx-text)" }}
              >
                {[1, 2, 3, 4, 5, 10, 15].map((m) => (
                  <option key={m} value={m}>
                    in {m} Min
                  </option>
                ))}
              </select>
              <button
                className="nx-btn-restart"
                disabled={!broadcastMessage.trim() || broadcastAction !== null}
                onClick={() => sendBroadcastAction("restart")}
              >
                {broadcastAction === "restart" ? "…" : "Neustart senden"}
              </button>
              <button
                className="nx-btn-restart"
                disabled={!broadcastMessage.trim() || broadcastAction !== null}
                onClick={() => sendBroadcastAction("stop")}
              >
                {broadcastAction === "stop" ? "…" : "Stop senden"}
              </button>
            </div>
            {broadcastSent && (
              <p style={{ color: "var(--nx-success)", fontSize: 12, marginTop: 8 }}>
                Angekündigt ✓ — {broadcastSent === "restart" ? "Neustart" : "Stop"} erfolgt in {broadcastDelay} Min.
              </p>
            )}
            {broadcastError && <p style={{ color: "var(--nx-danger)", fontSize: 12, marginTop: 8 }}>{broadcastError}</p>}
          </div>

          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 24 }}>
            <div>
              <span className="nx-config-label" style={{ fontWeight: 600, color: "var(--nx-text)" }}>
                Geplante Neustarts
              </span>
              <p style={{ color: "var(--nx-text-muted)", fontSize: 11, marginTop: 6 }}>
                Startet den Server automatisch täglich um die gewählte Uhrzeit neu (z.B. nachts, um Speicherlecks
                oder Lag-Aufbau vorzubeugen). Läuft direkt auf dem Server per systemd-Timer, unabhängig davon,
                ob GrimmNetz gerade offen ist.
              </p>
              <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 10 }}>
                <input
                  type="time"
                  value={newRestartTime}
                  onChange={(e) => setNewRestartTime(e.target.value)}
                  style={{ background: "var(--nx-panel-alt)", border: "1px solid var(--nx-border)", borderRadius: "var(--nx-radius)", padding: "6px 10px", color: "var(--nx-text)" }}
                />
                <button className="nx-btn-restart" disabled={restartAdding} onClick={addRestart}>
                  {restartAdding ? "…" : "Hinzufügen"}
                </button>
              </div>
              {restartError && <p style={{ color: "var(--nx-danger)", fontSize: 12, marginTop: 6 }}>{restartError}</p>}
            </div>

            <div>
              {restartsLoading && <p style={{ color: "var(--nx-text-muted)", fontSize: 12 }}>Lade…</p>}
              {!restartsLoading && scheduledRestarts.length === 0 && (
                <p style={{ color: "var(--nx-text-muted)", fontSize: 12 }}>Keine geplanten Neustarts.</p>
              )}
              {!restartsLoading &&
                scheduledRestarts.map((r) => (
                  <div
                    key={r.id}
                    style={{
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "space-between",
                      padding: "8px 12px",
                      background: "var(--nx-panel-alt)",
                      border: "1px solid var(--nx-border)",
                      borderRadius: "var(--nx-radius)",
                      marginBottom: 8,
                    }}
                  >
                    <span>Täglich um {r.time} Uhr</span>
                    <button className="nx-icon-btn" onClick={() => removeRestart(r.id)} aria-label="Entfernen">
                      ✕
                    </button>
                  </div>
                ))}
            </div>
          </div>
        </div>
      )}

      {tab === "console" && renderLogPanel()}

      {tab === "backups" && (
        <div>
          <div style={{ display: "flex", alignItems: "center", gap: 12, marginBottom: 14 }}>
            <button className="nx-btn-restart" disabled={backupCreating} onClick={createBackup}>
              {backupCreating ? "…" : "Backup erstellen"}
            </button>
            <button className="nx-btn-start" disabled={backupCreating} onClick={uploadBackup}>
              {uploadProgress != null ? `${uploadProgress}%` : backupCreating ? "…" : "Backup hochladen"}
            </button>
            <span style={{ fontSize: 12, color: "var(--nx-text-muted)" }}>
              Sichert das komplette Server-Verzeichnis als .tar.gz auf dem Server, oder lade eine lokal gespeicherte .tar.gz-Datei wieder hoch.
            </span>
          </div>

          {backupError && <p style={{ color: "var(--nx-danger)", fontSize: 12 }}>{backupError}</p>}
          {backupSavedPath && (
            <p style={{ color: "var(--nx-success)", fontSize: 12 }}>Heruntergeladen nach: {backupSavedPath}</p>
          )}

          {backupsLoading && <p style={{ color: "var(--nx-text-muted)" }}>Lade Backups…</p>}
          {!backupsLoading && backups.length === 0 && (
            <p style={{ color: "var(--nx-text-muted)" }}>Noch keine Backups vorhanden.</p>
          )}
          {!backupsLoading && backups.length > 0 && (
            <div className="nx-backup-list">
              {backups.map((b) => (
                <div key={b.name} className="nx-backup-row">
                  <div className="nx-backup-row-info">
                    <div>{formatBackupDate(b.created_at)}</div>
                    <div style={{ fontSize: 12, color: "var(--nx-text-muted)" }}>
                      {b.name} · {formatBackupSize(b.size_bytes)}
                    </div>
                  </div>
                  <div style={{ display: "flex", gap: 8 }}>
                    <button className="nx-btn-start" disabled={backupBusyName === b.name} onClick={() => downloadBackup(b.name)}>
                      {backupBusyName === b.name ? "…" : "Herunterladen"}
                    </button>
                    <button className="nx-btn-restart" disabled={backupBusyName === b.name} onClick={() => restoreBackup(b.name)}>
                      Wiederherstellen
                    </button>
                    <button className="nx-btn-stop" disabled={backupBusyName === b.name} onClick={() => deleteBackup(b.name)}>
                      Löschen
                    </button>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function Sparkline({ values, max }: { values: number[]; max: number }) {
  const width = 240;
  const height = 90;
  const axisWidth = 34;
  const axisHeight = 16;
  const gradientId = "nx-spark-fill";
  if (values.length === 0) {
    return <div style={{ color: "var(--nx-text-muted)", fontSize: 12 }}>Noch keine Daten</div>;
  }
  const coords = values.map((v, i) => {
    const x = (i / Math.max(1, values.length - 1)) * width;
    const y = height - (Math.min(v, max) / max) * height;
    return [x, y];
  });
  const linePoints = coords.map(([x, y]) => `${x},${y}`).join(" ");
  const areaPoints = `0,${height} ${linePoints} ${width},${height}`;
  const yTicks = [0, 0.25, 0.5, 0.75, 1];
  // Exactly 4 labels, each range/4 apart, ending at "now" - e.g. at the "1 Stunde" range
  // that's one every 15 minutes (18:00/17:45/17:30/17:15), at "2 Stunden" every 30 minutes.
  const now = Date.now();
  const rangeMinutes = 60;
  const stepSeconds = (rangeMinutes * 60) / 4;
  const xTicks = Array.from({ length: 4 }, (_, i) => new Date(now - (3 - i) * stepSeconds * 1000));

  return (
    <div style={{ display: "flex", flex: 1, minHeight: 0 }}>
      <div
        style={{
          width: axisWidth,
          display: "flex",
          flexDirection: "column",
          justifyContent: "space-between",
          paddingBottom: axisHeight,
          fontSize: 10,
          color: "var(--nx-text-muted)",
          textAlign: "right",
          paddingRight: 4,
        }}
      >
        {yTicks.map((t) => (
          <span key={t}>{Math.round((1 - t) * 100)}%</span>
        ))}
      </div>
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <svg
          width="100%"
          height="100%"
          viewBox={`0 0 ${width} ${height}`}
          preserveAspectRatio="none"
          style={{ flex: 1, minHeight: 0 }}
        >
          <defs>
            <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor="var(--nx-accent)" stopOpacity="0.35" />
              <stop offset="100%" stopColor="var(--nx-accent)" stopOpacity="0" />
            </linearGradient>
          </defs>
          <polygon points={areaPoints} fill={`url(#${gradientId})`} />
          <polyline points={linePoints} fill="none" stroke="var(--nx-accent)" strokeWidth="2" />
        </svg>
        <div style={{ display: "flex", justifyContent: "space-between", height: axisHeight, flexShrink: 0, fontSize: 10, color: "var(--nx-text-muted)" }}>
          {xTicks.map((d, i) => (
            <span key={i}>{d.toLocaleTimeString("de-DE", { hour: "2-digit", minute: "2-digit" })}</span>
          ))}
        </div>
      </div>
    </div>
  );
}
