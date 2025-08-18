use terminal_link::Link;

const EXPLORER_URL: &str = "https://solscan.io";

#[derive(Default, Debug)]
pub struct Explorer {
    cluster: String,
    custom_rpc: Option<String>,
}

impl From<String> for Explorer {
    fn from(json_rpc_url: String) -> Self {
        match &json_rpc_url.to_lowercase() {
            url if url.contains("devnet") => Explorer::devnet(),
            url if url.contains("testnet") => Explorer::testnet(),
            url if url.contains("mainnet") => Explorer::mainnet(),
            _ => {
                Explorer::custom(json_rpc_url)
            }
        }
    }
}

impl Explorer {
    pub fn mainnet() -> Self {
        Self {
            cluster: "mainnet-beta".into(),
            ..Default::default()
        }
    }

    pub fn testnet() -> Self {
        Self {
            cluster: "testnet".into(),
            ..Default::default()
        }
    }

    pub fn devnet() -> Self {
        Self {
            cluster: "devnet".into(),
            ..Default::default()
        }
    }

    pub fn custom(custom_rpc: String) -> Self {
        Self {
            cluster: "custom".into(),
            custom_rpc: Some(custom_rpc),
        }
    }

    pub fn base(&self) -> String {
        let url = format!("{}?cluster={}", EXPLORER_URL, self.cluster);
        if self.cluster == "custom" {
            url + "&customUrl=" + self.custom_rpc.as_ref().unwrap()
        } else {
            url
        }
    }

    fn build_url(&self, path: &str) -> String {
        let url = format!("{}/{}?cluster={}", EXPLORER_URL, path, self.cluster);
        if self.cluster == "custom" {
            url + "&customUrl=" + self.custom_rpc.as_ref().unwrap()
        } else {
            url
        }
    }

    pub fn tx<T: std::fmt::Display>(&self, tx: T) -> String {
        let url = self.build_url(&format!("tx/{}", tx));
        Link::new(tx.to_string().as_str(), url.as_str()).to_string()
    }

    pub fn account<T: std::fmt::Display>(&self, account: T) -> String {
        let url = self.build_url(&format!("account/{}", account));
        Link::new(account.to_string().as_str(), url.as_str()).to_string()
    }

    pub fn portfolio<T: std::fmt::Display>(&self, account: T) -> String {
        let url = self.build_url(&format!("account/{}#portfolio", account));
        Link::new(account.to_string().as_str(), url.as_str()).to_string()
    }

    pub fn token<T: std::fmt::Display>(&self, token: T) -> String {
        let url = self.build_url(&format!("token/{}", token));
        Link::new(token.to_string().as_str(), url.as_str()).to_string()
    }
}
