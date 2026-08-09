use keyring::Entry;

const SERVICE: &str = "com.grimmschmiede.grimmnetz";

/// Stores an SSH password or key passphrase in the native OS keyring
/// (Windows Credential Manager / Gnome Keyring / KWallet). Never touches disk.
pub fn store_secret(server_id: &str, secret: &str) -> keyring::Result<()> {
    Entry::new(SERVICE, server_id)?.set_password(secret)
}

pub fn get_secret(server_id: &str) -> keyring::Result<String> {
    Entry::new(SERVICE, server_id)?.get_password()
}

pub fn delete_secret(server_id: &str) -> keyring::Result<()> {
    Entry::new(SERVICE, server_id)?.delete_credential()
}

/// The SQLCipher database encryption key itself also lives in the keyring,
/// generated once on first launch, never written in plaintext anywhere.
pub fn get_or_create_db_key() -> keyring::Result<String> {
    let entry = Entry::new(SERVICE, "__db_encryption_key__")?;
    match entry.get_password() {
        Ok(key) => Ok(key),
        Err(keyring::Error::NoEntry) => {
            let key = uuid::Uuid::new_v4().to_string();
            entry.set_password(&key)?;
            Ok(key)
        }
        Err(e) => Err(e),
    }
}
