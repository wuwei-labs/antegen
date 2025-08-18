use {
    crate::config::CliConfig,
    crate::utils::Explorer,
};

fn explorer(config: CliConfig) -> Explorer {
   Explorer::from(config.json_rpc_url)
}
