#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientMods {
    Presentation,
    Any,
    None,
}

impl ClientMods {
    pub const SESSION_DEFAULT: ClientMods = ClientMods::Presentation;

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
    Unstated,
}

impl ClientScripts {
    pub const SESSION_DEFAULT: ClientScripts = ClientScripts::Unstated;

    pub fn runs(self) -> bool {
        self == ClientScripts::Allowed
    }

    pub fn reason(self) -> &'static str {
        match self {
            ClientScripts::Allowed => "the server allows client scripts",
            ClientScripts::Refused => "the server's client_scripts policy refuses client scripts",
            ClientScripts::Unstated => {
                "the server states no client_scripts policy (the protocol carries none before multiplayer phase 6)"
            }
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
        assert!(!ClientScripts::SESSION_DEFAULT.runs(), "until servers state a policy, a session runs none");
    }
}
