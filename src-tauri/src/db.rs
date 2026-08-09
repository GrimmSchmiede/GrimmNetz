use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub struct Db {
    pub conn: Connection,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ServerRecord {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub os_info: Option<String>,
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
                host TEXT NOT NULL,
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
        Ok(Self { conn })
    }

    pub fn insert_server(&self, record: &ServerRecord) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO servers (id, name, host, port, username, auth_method, os_info, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'password', ?6, datetime('now'))",
            params![record.id, record.name, record.host, record.port, record.username, record.os_info],
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

    pub fn list_servers(&self) -> rusqlite::Result<Vec<ServerRecord>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, host, port, username, os_info FROM servers ORDER BY created_at ASC")?;
        let rows = stmt.query_map([], |row| {
            Ok(ServerRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                host: row.get(2)?,
                port: row.get(3)?,
                username: row.get(4)?,
                os_info: row.get(5)?,
            })
        })?;
        rows.collect()
    }

    pub fn get_server(&self, id: &str) -> rusqlite::Result<ServerRecord> {
        self.conn.query_row(
            "SELECT id, name, host, port, username, os_info FROM servers WHERE id = ?1",
            params![id],
            |row| {
                Ok(ServerRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    host: row.get(2)?,
                    port: row.get(3)?,
                    username: row.get(4)?,
                    os_info: row.get(5)?,
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

    pub fn delete_instance(&self, id: &str) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM instances WHERE id = ?1", params![id])?;
        Ok(())
    }
}
