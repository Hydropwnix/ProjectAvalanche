use std::path::{Path, PathBuf};

use ssx_net_proto::keyfile::{self, KeyFileError, Origin, PLAYER_IDENTITY};
use ssx_net_proto::keys::{PlayerId, PlayerKey, PlayerSecret};

pub const IDENTITY_FILE: &str = "identity.key";

pub struct PlayerIdentity {
    secret: PlayerSecret,
    path: PathBuf,
    created: bool,
}

impl PlayerIdentity {
    pub fn load_or_create(app_dir: &Path) -> Result<Self, KeyFileError> {
        let path = app_dir.join(IDENTITY_FILE);
        let (seed, origin) = keyfile::load_or_create(&path, PLAYER_IDENTITY, || PlayerSecret::generate().to_bytes())?;
        let secret = PlayerSecret::from_bytes(*seed).map_err(|_| KeyFileError::Foreign {
            path: path.clone(),
            what: PLAYER_IDENTITY.what,
            why: "the key is a weak point",
        })?;
        Ok(Self { secret, path, created: origin == Origin::Created })
    }

    pub fn key(&self) -> PlayerKey {
        self.secret.public()
    }

    pub fn id(&self) -> PlayerId {
        self.key().id()
    }

    pub fn secret(&self) -> &PlayerSecret {
        &self.secret
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn created(&self) -> bool {
        self.created
    }
}
