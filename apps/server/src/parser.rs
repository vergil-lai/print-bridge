use clap::{error::ErrorKind, CommandFactory, Parser, Subcommand};

/// Headless 产品入口参数。
#[derive(Debug, Parser)]
#[command(name = "print-bridge", about = "PrintBridge headless Agent")]
pub struct ServerArgs {
    #[command(subcommand)]
    pub command: Option<ServerCommand>,
}

/// 只有 headless 产品提供的进程命令。
#[derive(Debug, Subcommand)]
pub enum ServerCommand {
    /// 前台运行 PrintBridge Agent。
    Serve,
}

impl ServerArgs {
    /// 解析可测试参数，并在无子命令时生成帮助。
    pub fn try_parse_product_from<I, T>(args: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        let parsed = Self::try_parse_from(args);
        match parsed {
            Ok(args) if args.command.is_none() => {
                Err(Self::command().error(ErrorKind::DisplayHelp, Self::command().render_help()))
            }
            result => result,
        }
    }
}
