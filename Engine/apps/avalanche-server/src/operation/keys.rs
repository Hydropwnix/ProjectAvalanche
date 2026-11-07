use std::path::Path;

use ssx_net_proto::keyfile::{self, KeyFileError, OPERATOR_KEY};
use ssx_net_proto::rcon::OperatorSecret;

pub fn generate(path: &Path) -> Result<OperatorSecret, KeyFileError> {
    if std::fs::symlink_metadata(path).is_ok() {
        return Err(KeyFileError::Exists { path: path.to_path_buf() });
    }
    let secret = OperatorSecret::generate();
    keyfile::create(path, OPERATOR_KEY, &secret.to_bytes())?;
    Ok(secret)
}

pub fn load(path: &Path) -> Result<OperatorSecret, String> {
    let bytes = keyfile::load(path, OPERATOR_KEY).map_err(|e| e.to_string())?;
    OperatorSecret::from_bytes(*bytes).map_err(|e| format!("{}: {e}", path.display()))
}
