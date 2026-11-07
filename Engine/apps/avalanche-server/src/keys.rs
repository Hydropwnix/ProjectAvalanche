use std::path::{Path, PathBuf};

use ssx_net_proto::keyfile::{self, KeyFileError, NETCODE_KEY, Origin, SERVER_IDENTITY};
use ssx_net_proto::keys::{KEY_SIZE, ServerKey, ServerSecret, random_bytes};
use zeroize::Zeroizing;

pub const NETCODE_FILE: &str = "netcode.key";
pub const IDENTITY_FILE: &str = "identity.key";

pub struct DataKeys {
    pub netcode: Zeroizing<[u8; KEY_SIZE]>,
    pub identity: ServerSecret,
    pub created: Vec<PathBuf>,
}

impl DataKeys {
    pub fn identity_key(&self) -> ServerKey {
        self.identity.public()
    }
}

fn identity(path: &Path, bytes: &[u8; KEY_SIZE]) -> Result<ServerSecret, KeyFileError> {
    ServerSecret::from_bytes(*bytes).map_err(|_| KeyFileError::Foreign {
        path: path.to_path_buf(),
        what: SERVER_IDENTITY.what,
        why: "the key is a low-order point",
    })
}

pub fn load_or_create(data_dir: &Path) -> Result<DataKeys, KeyFileError> {
    let mut created = Vec::new();
    let netcode_path = data_dir.join(NETCODE_FILE);
    let (netcode, origin) = keyfile::load_or_create(&netcode_path, NETCODE_KEY, random_bytes)?;
    if origin == Origin::Created {
        created.push(netcode_path);
    }
    let identity_path = data_dir.join(IDENTITY_FILE);
    let (secret, origin) =
        keyfile::load_or_create(&identity_path, SERVER_IDENTITY, || ServerSecret::generate().to_bytes())?;
    if origin == Origin::Created {
        created.push(identity_path.clone());
    }
    Ok(DataKeys { netcode, identity: identity(&identity_path, &secret)?, created })
}

pub fn load(data_dir: &Path) -> Result<DataKeys, KeyFileError> {
    let netcode = keyfile::load(&data_dir.join(NETCODE_FILE), NETCODE_KEY)?;
    let identity_path = data_dir.join(IDENTITY_FILE);
    let secret = keyfile::load(&identity_path, SERVER_IDENTITY)?;
    Ok(DataKeys { netcode, identity: identity(&identity_path, &secret)?, created: Vec::new() })
}

pub fn generate(data_dir: &Path) -> Result<DataKeys, KeyFileError> {
    for name in [NETCODE_FILE, IDENTITY_FILE] {
        let path = data_dir.join(name);
        if std::fs::symlink_metadata(&path).is_ok() {
            return Err(KeyFileError::Exists { path });
        }
    }
    let netcode = Zeroizing::new(random_bytes());
    keyfile::create(&data_dir.join(NETCODE_FILE), NETCODE_KEY, &netcode)?;
    let identity = ServerSecret::generate();
    keyfile::create(&data_dir.join(IDENTITY_FILE), SERVER_IDENTITY, &identity.to_bytes())?;
    Ok(DataKeys { netcode, identity, created: vec![data_dir.join(NETCODE_FILE), data_dir.join(IDENTITY_FILE)] })
}
