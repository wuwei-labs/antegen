use {
    crate::errors::CliError,
    crate::config::CliConfig,
    antegen_utils::explorer::Explorer,
};

pub fn thread_url<T: std::fmt::Display>(thread: T, config: CliConfig) -> Result<(),
    CliError> {
    println!("thread: {}", explorer(config).thread_url(thread, antegen_thread_program::ID));
    Ok(())
}

fn explorer(config: CliConfig) -> Explorer {
   Explorer::from(config.json_rpc_url)
}
