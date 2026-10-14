use super::state::Level;
use clap::{Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Requests {
    #[default]
    Off,
    Masked,
}

#[derive(Clone, Debug, Deserialize, Serialize, Subcommand)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Status {},
    Enable {
        #[arg(long, value_enum, default_value = "debug")]
        level: Level,
        #[arg(long, value_enum, default_value = "off")]
        requests: Requests,
        #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..=3600))]
        ttl: u64,
    },
    Disable {},
}

#[derive(Clone, Debug, Subcommand)]
pub enum AppCommand {
    /// Change diagnostics in the running instance without restarting it
    Diagnostics {
        #[command(subcommand)]
        command: Command,
    },
}

#[cfg(test)]
mod tests {
    use crate::config::ClapConfig;
    use clap::Parser;
    #[test]
    fn cli_accepts_runtime_controls_and_preserves_server_arguments() {
        for args in [
            vec!["epoxy", "diagnostics", "status"],
            vec!["epoxy", "diagnostics", "disable"],
            vec!["epoxy", "diagnostics", "enable", "--requests", "masked"],
            vec!["epoxy", "--endpoint", "localhost:17165", "--log", "warn"],
        ] {
            assert!(ClapConfig::try_parse_from(args).is_ok());
        }
    }
    #[test]
    fn cli_rejects_invalid_levels_lifetimes_and_unmasked_mode() {
        for option in [
            ["--level", "verbose"],
            ["--ttl", "0"],
            ["--ttl", "3601"],
            ["--requests", "raw"],
        ] {
            assert!(
                ClapConfig::try_parse_from([
                    "epoxy",
                    "diagnostics",
                    "enable",
                    option[0],
                    option[1]
                ])
                .is_err()
            );
        }
    }
}
