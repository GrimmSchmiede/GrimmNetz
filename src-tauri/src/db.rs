use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub struct Db {
    pub conn: Connection,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ServerRecord {
    pub id: String,
    pub name: String,
    pub host: Option<String>,
    pub port: u16,
    pub username: String,
    pub os_info: Option<String>,
    /// SHA-256 fingerprint of the SSH host key seen on first connect (trust-on-first-use).
    /// Every later connection must match this - a mismatch means the server's host key
    /// changed, which is either a legitimate reinstall or a man-in-the-middle attack, and
    /// either way must never be silently accepted.
    #[serde(default)]
    pub known_host_fingerprint: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct InstanceRecord {
    pub id: String,
    pub server_id: String,
    pub game_id: String,
    pub display_name: String,
    pub install_path: String,
    pub systemd_unit: String,
    pub cpu_limit_percent: u32,
    pub ram_limit_mb: u32,
}

impl Db {
    /// Opens (or creates) the local SQLCipher-encrypted database.
    /// `key` comes from the OS keyring (see `keyring_store`), never from disk.
    pub fn open(path: PathBuf, key: &str) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "key", key)?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS servers (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                host TEXT,
                port INTEGER NOT NULL DEFAULT 22,
                username TEXT NOT NULL,
                auth_method TEXT NOT NULL, -- 'password' | 'key'
                os_info TEXT,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS instances (
                id TEXT PRIMARY KEY,
                server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
                game_id TEXT NOT NULL,
                display_name TEXT NOT NULL,
                install_path TEXT NOT NULL,
                systemd_unit TEXT NOT NULL,
                cpu_limit_percent INTEGER,
                ram_limit_mb INTEGER,
                created_at TEXT NOT NULL
            );
            ",
        )?;
        // Migration for DBs created before os_info existed.
        let _ = conn.execute("ALTER TABLE servers ADD COLUMN os_info TEXT", []);
        // Migration for DBs created before host-key pinning existed.
        let _ = conn.execute("ALTER TABLE servers ADD COLUMN known_host_fingerprint TEXT", []);

        // Migration: `host` muss nullable sein, damit Weg B (Server-Key vor VM-Existenz
        // generieren, siehe ssh_keys.rs/prepare_key_only_server) einen Eintrag ohne IP anlegen
        // kann. SQLite kann NOT NULL nicht per ALTER entfernen, daher Tabellen-Neuerstellung -
        // nur nötig, wenn die alte NOT-NULL-Variante noch besteht (idempotent über PRAGMA-Check).
        let host_is_not_null: bool = conn
            .prepare("SELECT \"notnull\" FROM pragma_table_info('servers') WHERE name = 'host'")?
            .query_row([], |row| row.get::<_, i64>(0))
            .unwrap_or(0)
            == 1;
        if host_is_not_null {
            conn.execute_batch(
                "
                CREATE TABLE servers_new (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    host TEXT,
                    port INTEGER NOT NULL DEFAULT 22,
                    username TEXT NOT NULL,
                    auth_method TEXT NOT NULL,
                    os_info TEXT,
                    known_host_fingerprint TEXT,
                    created_at TEXT NOT NULL
                );
                INSERT INTO servers_new SELECT id, name, host, port, username, auth_method, os_info, known_host_fingerprint, created_at FROM servers;
                DROP TABLE servers;
                ALTER TABLE servers_new RENAME TO servers;
                ",
            )?;
        }
        Ok(Self { conn })
    }

    pub fn insert_server(&self, record: &ServerRecord) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO servers (id, name, host, port, username, auth_method, os_info, known_host_fingerprint, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'password', ?6, ?7, datetime('now'))",
            params![
                record.id,
                record.name,
                record.host,
                record.port,
                record.username,
                record.os_info,
                record.known_host_fingerprint,
            ],
        )?;
        Ok(())
    }

    /// Pins (or repins, e.g. after `update_server` deliberately re-trusts a changed server) the
    /// SSH host key fingerprint seen on connect.
    pub fn set_host_fingerprint(&self, id: &str, fingerprint: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE servers SET known_host_fingerprint = ?2 WHERE id = ?1",
            params![id, fingerprint],
        )?;
        Ok(())
    }

    /// Updates a server's connection details (name/host/port/username) in place - lets the
    /// user fix things like a changed provider IP or renamed login without re-provisioning
    /// the `gameserver` user, systemd units, or anything else already set up on the server.
    pub fn update_server(&self, id: &str, name: &str, host: &str, port: u16, username: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE servers SET name = ?2, host = ?3, port = ?4, username = ?5 WHERE id = ?1",
            params![id, name, host, port, username],
        )?;
        Ok(())
    }

    /// Trägt Host/Port für einen per Weg B (Key-vor-VM) angelegten Server nach, sobald die VM
    /// beim Provider existiert und der User die IP in der App einträgt.
    pub fn set_host(&self, id: &str, host: &str, port: u16) -> rusqlite::Result<()> {
        self.conn
            .execute("UPDATE servers SET host = ?1, port = ?2 WHERE id = ?3", params![host, port, id])?;
        Ok(())
    }

    /// Same host+port already added? Reconnecting to a server you already manage under a
    /// second entry causes real damage further down the line (e.g. `discover_instances`
    /// re-inserting the same already-known instance under a different server_id and hitting
    /// the DB's unique constraint) - so this is checked before `add_server` does anything.
    pub fn find_server_by_host_port(&self, host: &str, port: u16) -> rusqlite::Result<Option<ServerRecord>> {
        self.conn
            .query_row(
                "SELECT id, name, host, port, username, os_info, known_host_fingerprint \
                 FROM servers WHERE host = ?1 AND port = ?2",
                params![host, port],
                |row| {
                    Ok(ServerRecord {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        host: row.get(2)?,
                        port: row.get(3)?,
                        username: row.get(4)?,
                        os_info: row.get(5)?,
                        known_host_fingerprint: row.get(6)?,
                    })
                },
            )
            .optional()
    }

    pub fn list_servers(&self) -> rusqlite::Result<Vec<ServerRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, host, port, username, os_info, known_host_fingerprint FROM servers ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ServerRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                host: row.get(2)?,
                port: row.get(3)?,
                username: row.get(4)?,
                os_info: row.get(5)?,
                known_host_fingerprint: row.get(6)?,
            })
        })?;
        rows.collect()
    }

    pub fn get_server(&self, id: &str) -> rusqlite::Result<ServerRecord> {
        self.conn.query_row(
            "SELECT id, name, host, port, username, os_info, known_host_fingerprint FROM servers WHERE id = ?1",
            params![id],
            |row| {
                Ok(ServerRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    host: row.get(2)?,
                    port: row.get(3)?,
                    username: row.get(4)?,
                    os_info: row.get(5)?,
                    known_host_fingerprint: row.get(6)?,
                })
            },
        )
    }

    pub fn delete_server(&self, id: &str) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM servers WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn insert_instance(&self, record: &InstanceRecord) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO instances (id, server_id, game_id, display_name, install_path, systemd_unit, cpu_limit_percent, ram_limit_mb, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))",
            params![
                record.id,
                record.server_id,
                record.game_id,
                record.display_name,
                record.install_path,
                record.systemd_unit,
                record.cpu_limit_percent,
                record.ram_limit_mb,
            ],
        )?;
        Ok(())
    }

    pub fn update_instance_resources(&self, id: &str, cpu_limit_percent: u32, ram_limit_mb: u32) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE instances SET cpu_limit_percent = ?1, ram_limit_mb = ?2 WHERE id = ?3",
            params![cpu_limit_percent, ram_limit_mb, id],
        )?;
        Ok(())
    }

    pub fn list_instances(&self, server_id: &str) -> rusqlite::Result<Vec<InstanceRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, server_id, game_id, display_name, install_path, systemd_unit, cpu_limit_percent, ram_limit_mb
             FROM instances WHERE server_id = ?1 ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map(params![server_id], |row| {
            Ok(InstanceRecord {
                id: row.get(0)?,
                server_id: row.get(1)?,
                game_id: row.get(2)?,
                display_name: row.get(3)?,
                install_path: row.get(4)?,
                systemd_unit: row.get(5)?,
                cpu_limit_percent: row.get(6)?,
                ram_limit_mb: row.get(7)?,
            })
        })?;
        rows.collect()
    }

    /// Looks up a single instance by its primary key. Used by `attach_install_stream` to detect
    /// an already-completed install on reconnect, where re-tailing install.log from byte 0 would
    /// otherwise re-match the DONE marker and attempt a duplicate `insert_instance`.
    pub fn get_instance(&self, id: &str) -> rusqlite::Result<Option<InstanceRecord>> {
        self.conn
            .query_row(
                "SELECT id, server_id, game_id, display_name, install_path, systemd_unit, cpu_limit_percent, ram_limit_mb
                 FROM instances WHERE id = ?1",
                params![id],
                |row| {
                    Ok(InstanceRecord {
                        id: row.get(0)?,
                        server_id: row.get(1)?,
                        game_id: row.get(2)?,
                        display_name: row.get(3)?,
                        install_path: row.get(4)?,
                        systemd_unit: row.get(5)?,
                        cpu_limit_percent: row.get(6)?,
                        ram_limit_mb: row.get(7)?,
                    })
                },
            )
            .optional()
    }

    pub fn find_instance_by_unit(&self, server_id: &str, systemd_unit: &str) -> rusqlite::Result<Option<InstanceRecord>> {
        self.conn
            .query_row(
                "SELECT id, server_id, game_id, display_name, install_path, systemd_unit, cpu_limit_percent, ram_limit_mb
                 FROM instances WHERE server_id = ?1 AND systemd_unit = ?2",
                params![server_id, systemd_unit],
                |row| {
                    Ok(InstanceRecord {
                        id: row.get(0)?,
                        server_id: row.get(1)?,
                        game_id: row.get(2)?,
                        display_name: row.get(3)?,
                        install_path: row.get(4)?,
                        systemd_unit: row.get(5)?,
                        cpu_limit_percent: row.get(6)?,
                        ram_limit_mb: row.get(7)?,
                    })
                },
            )
            .optional()
    }

    pub fn delete_instance(&self, id: &str) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM instances WHERE id = ?1", params![id])?;
        Ok(())
    }
}
