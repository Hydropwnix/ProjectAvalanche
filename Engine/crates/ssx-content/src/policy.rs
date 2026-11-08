#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientMods {
    Presentation,
    Any,
    None,
}

impl ClientMods {
    pub const SESSION_DEFAULT: ClientMods = ClientMods::Presentation;

    pub fn id_basis(self) -> ClientMods {
        match self {
            ClientMods::Any | ClientMods::Presentation => ClientMods::Presentation,
            ClientMods::None => ClientMods::None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            ClientMods::Presentation => "presentation",
            ClientMods::Any => "any",
            ClientMods::None => "none",
        }
    }

    pub fn parse(s: &str) -> Option<ClientMods> {
        match s {
            "presentation" => Some(ClientMods::Presentation),
            "any" => Some(ClientMods::Any),
            "none" => Some(ClientMods::None),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientScripts {
    Allowed,
    Refused,
    Pending,
    Unstated,
}

impl ClientScripts {
    pub const SESSION_DEFAULT: ClientScripts = ClientScripts::Pending;

    pub fn runs(self) -> bool {
        self == ClientScripts::Allowed
    }

    pub fn reason(self) -> &'static str {
        match self {
            ClientScripts::Allowed => "the server allows client scripts",
            ClientScripts::Refused => "the server does not allow client scripts (its client_scripts policy)",
            ClientScripts::Pending => "the server's client_scripts policy is not read yet (the join reads it)",
            ClientScripts::Unstated => "the server states no client_scripts policy (it serves no resource list)",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_allowing_server_runs_client_scripts() {
        assert!(ClientScripts::Allowed.runs());
        assert!(!ClientScripts::Refused.runs());
        assert!(
            !ClientScripts::SESSION_DEFAULT.runs(),
            "until the join reads the server's policy, a session runs none"
        );
        assert!(!ClientScripts::Unstated.runs());
    }
}
