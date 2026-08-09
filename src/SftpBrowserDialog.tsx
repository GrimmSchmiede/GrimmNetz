import { useEffect, useState } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";

type DirEntry = {
  name: string;
  is_dir: boolean;
  size_bytes: number;
};

type Props = {
  serverId: string;
  serverName: string;
  onClose: () => void;
};

const ROOT = "/home/gameserver";

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

export default function SftpBrowserDialog({ serverId, serverName, onClose }: Props) {
  const [path, setPath] = useState(ROOT);
  const [entries, setEntries] = useState<DirEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [busyName, setBusyName] = useState<string | null>(null);
  const [uploadProgress, setUploadProgress] = useState<number | null>(null);

  function reload() {
    setLoading(true);
    setError("");
    invoke<DirEntry[]>("sftp_list", { serverId, path })
      .then(setEntries)
      .catch((err) => setError(String(err)))
      .finally(() => setLoading(false));
  }

  useEffect(reload, [serverId, path]);

  const segments = path === ROOT ? [] : path.slice(ROOT.length + 1).split("/");

  function goTo(index: number) {
    setPath(index < 0 ? ROOT : `${ROOT}/${segments.slice(0, index + 1).join("/")}`);
  }

  async function openEntry(entry: DirEntry) {
    if (entry.is_dir) {
      setPath(`${path}/${entry.name}`);
    }
  }

  async function downloadEntry(entry: DirEntry) {
    const localPath = await save({ defaultPath: entry.name });
    if (!localPath) return;
    setBusyName(entry.name);
    setError("");
    try {
      await invoke("sftp_download", { serverId, path: `${path}/${entry.name}`, localPath });
    } catch (err) {
      setError(String(err));
    } finally {
      setBusyName(null);
    }
  }

  async function deleteEntry(entry: DirEntry) {
    if (!confirm(`"${entry.name}" wirklich unwiderruflich löschen?`)) return;
    setBusyName(entry.name);
    setError("");
    try {
      await invoke("sftp_delete", { serverId, path: `${path}/${entry.name}` });
      reload();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusyName(null);
    }
  }

  async function uploadFile() {
    const localPath = await open({ multiple: false });
    if (!localPath || Array.isArray(localPath)) return;
    setError("");
    setUploadProgress(0);
    try {
      const onProgress = new Channel<{ event: "progress"; bytesSent: number; totalBytes: number }>();
      onProgress.onmessage = (event) => {
        if (event.totalBytes > 0) setUploadProgress(Math.round((event.bytesSent / event.totalBytes) * 100));
      };
      await invoke("sftp_upload", { serverId, remoteDir: path, localPath, onProgress });
      reload();
    } catch (err) {
      setError(String(err));
    } finally {
      setUploadProgress(null);
    }
  }

  async function createFolder() {
    const name = prompt("Name des neuen Ordners:");
    if (!name || !name.trim()) return;
    setError("");
    try {
      await invoke("sftp_mkdir", { serverId, path: `${path}/${name.trim()}` });
      reload();
    } catch (err) {
      setError(String(err));
    }
  }

  return (
    <div className="nx-modal-overlay" onClick={onClose}>
      <div className="nx-modal nx-dirbrowser-modal" onClick={(e) => e.stopPropagation()} style={{ width: 640 }}>
        <h2>Vollzugriff (SFTP)</h2>
        <div style={{ fontSize: 12, color: "var(--nx-text-muted)", marginTop: -8 }}>{serverName}</div>

        <div style={{ display: "flex", alignItems: "center", gap: 4, flexWrap: "wrap", fontSize: 12, margin: "10px 0" }}>
          <button className="nx-icon-btn" onClick={() => goTo(-1)}>
            /home/gameserver
          </button>
          {segments.map((seg, i) => (
            <span key={i} style={{ display: "flex", alignItems: "center", gap: 4 }}>
              <span style={{ color: "var(--nx-text-muted)" }}>/</span>
              <button className="nx-icon-btn" onClick={() => goTo(i)}>
                {seg}
              </button>
            </span>
          ))}
        </div>

        <div style={{ display: "flex", gap: 8, marginBottom: 10 }}>
          <button className="nx-btn-restart" onClick={uploadFile} disabled={uploadProgress !== null}>
            {uploadProgress !== null ? `Lädt hoch… ${uploadProgress}%` : "Datei hochladen"}
          </button>
          <button className="nx-icon-btn" onClick={createFolder}>
            + Neuer Ordner
          </button>
          <button className="nx-icon-btn" onClick={reload}>
            ⟳ Aktualisieren
          </button>
        </div>

        {loading && <p style={{ color: "var(--nx-text-muted)" }}>Lade…</p>}
        {error && <p style={{ color: "var(--nx-danger)", fontSize: 12 }}>{error}</p>}
        {!loading && !error && entries.length === 0 && (
          <p style={{ color: "var(--nx-text-muted)" }}>Verzeichnis ist leer.</p>
        )}
        {!loading && entries.length > 0 && (
          <div className="nx-dirbrowser-list">
            {entries.map((entry) => (
              <div
                key={entry.name}
                className="nx-dirbrowser-row"
                onDoubleClick={() => openEntry(entry)}
                style={{ cursor: entry.is_dir ? "pointer" : "default" }}
              >
                <span>{entry.is_dir ? "📁" : "📄"}</span>
                <span style={{ flex: 1 }}>{entry.name}</span>
                {!entry.is_dir && (
                  <span style={{ color: "var(--nx-text-muted)", fontSize: 12 }}>{formatSize(entry.size_bytes)}</span>
                )}
                <div style={{ display: "flex", gap: 4 }}>
                  {!entry.is_dir && (
                    <button className="nx-icon-btn" disabled={busyName === entry.name} onClick={() => downloadEntry(entry)}>
                      ⬇
                    </button>
                  )}
                  <button className="nx-icon-btn" disabled={busyName === entry.name} onClick={() => deleteEntry(entry)}>
                    🗑
                  </button>
                </div>
              </div>
            ))}
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
