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
}
