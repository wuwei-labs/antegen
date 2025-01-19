use {
    crate::config::CliConfig,
    antegen_utils::explorer::Explorer,
};

fn explorer(config: CliConfig) -> Explorer {
   Explorer::from(config.json_rpc_url)
}
