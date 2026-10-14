use std::{fmt::Debug, fs, io, process};

use anyhow::{Context, Error};
use clap::{ArgMatches, CommandFactory, Parser, ValueEnum, parser::ValueSource};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum PinPromptKind {
    Tty,
    #[cfg(feature = "ui-gtk")]
    Ui,
}

impl Default for PinPromptKind {
    #[cfg(not(feature = "ui-gtk"))]
    fn default() -> PinPromptKind {
        PinPromptKind::Tty
    }

    #[cfg(feature = "ui-gtk")]
    fn default() -> PinPromptKind {
        PinPromptKind::Ui
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct ConfigSlice {
    pub endpoint: Option<String>,
    pub nssdb_path: Option<String>,
    pub log: Option<String>,
    pub max_connections: Option<u8>,
    pub max_idle_seconds: Option<u32>,
    pub pin_prompt: Option<PinPromptKind>,
    pub allow_soft_tokens: Option<bool>,
    pub show_window: Option<bool>,
    pub allow_missing_origin: Option<bool>,
    pub selection_path: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Config {
    pub endpoint: String,
    pub nssdb_path: String,
    pub log: String,
    pub max_connections: u8,
    pub max_idle_seconds: u32,
    pub pin_prompt: PinPromptKind,
    pub allow_soft_tokens: bool,
    pub show_window: bool,
    pub allow_missing_origin: bool,
    pub selection_path: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            endpoint: String::from("localhost:17165"),
            nssdb_path: default_nssdb_path(),
            log: String::from("info"),
            max_connections: 10,
            max_idle_seconds: 0,
            pin_prompt: PinPromptKind::default(),
            allow_soft_tokens: false,
            show_window: false,
            allow_missing_origin: false,
            selection_path: default_selection_path(),
        }
    }
}

impl Config {
    fn merge(self, slice: ConfigSlice) -> Config {
        Config {
            endpoint: slice.endpoint.unwrap_or(self.endpoint),
            nssdb_path: slice.nssdb_path.unwrap_or(self.nssdb_path),
            log: slice.log.unwrap_or(self.log),
            max_connections: slice.max_connections.unwrap_or(self.max_connections),
            max_idle_seconds: slice.max_idle_seconds.unwrap_or(self.max_idle_seconds),
            pin_prompt: slice.pin_prompt.unwrap_or(self.pin_prompt),
            allow_soft_tokens: slice.allow_soft_tokens.unwrap_or(self.allow_soft_tokens),
            show_window: slice.show_window.unwrap_or(self.show_window),
            allow_missing_origin: slice
                .allow_missing_origin
                .unwrap_or(self.allow_missing_origin),
            selection_path: slice.selection_path.unwrap_or(self.selection_path),
        }
    }
}

fn default_nssdb_path() -> String {
    dirs::home_dir()
        .map(|p| p.join(".pki/nssdb"))
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

fn default_selection_path() -> String {
    dirs::config_dir()
        .unwrap_or_default()
        .join("epoxy/certificate-selection.json")
        .to_string_lossy()
        .into_owned()
}

fn default_config_path() -> String {
    dirs::config_dir()
        .map(|p| p.join("epoxy.toml"))
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

/// SmartBox-compatible open source tool for signing ePorezi tax forms
#[derive(Clone, Debug, Parser, Serialize)]
#[command(version, about)]
pub struct ClapConfig {
    #[serde(skip)]
    #[command(subcommand)]
    command: Option<crate::diagnostics::cli::AppCommand>,

    /// Path to an optional config file
    #[serde(skip)]
    #[arg(short = 'c', long, default_value_t = default_config_path())]
    config_path: String,

    /// Endpoint to listen on. Ports supported by ePorezi: 17165, 20806, 65097
    #[arg(short = 'e', long, default_value_t = Config::default().endpoint)]
    endpoint: String,

    /// Path to an NSS database
    #[arg(short = 'd', long, default_value_t = Config::default().nssdb_path)]
    nssdb_path: String,

    /// App-private file for an explicitly remembered certificate/context (never a PIN)
    #[arg(long, default_value_t = default_selection_path())]
    selection_path: String,

    /// Logging directives (see https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html#directives)
    #[arg(short = 'l', long, default_value_t = Config::default().log)]
    log: String,

    /// Max accepted connections
    #[arg(long, default_value_t = Config::default().max_connections)]
    max_connections: u8,

    /// Shut down if idle for this duration (no new connections accepted)
    #[arg(long, default_value_t = Config::default().max_idle_seconds)]
    max_idle_seconds: u32,

    /// PIN prompt method
    #[arg(long, value_enum, default_value_t = Config::default().pin_prompt)]
    pin_prompt: PinPromptKind,

    #[arg(long, default_value_t = Config::default().allow_soft_tokens)]
    /// Include certificates from software PKCS#11 tokens for testing
    allow_soft_tokens: bool,

    /// Open the GTK control window at startup (requires UI PIN prompt)
    #[arg(long)]
    show_window: bool,

    /// Development only: admit Origin-less clients; wrong present Origins remain blocked
    #[arg(long, action = clap::ArgAction::Set, num_args = 0..=1, require_equals = true,
          default_missing_value = "true", default_value_t = false)]
    allow_missing_origin: bool,

    /// Print default config
    #[serde(skip)]
    #[arg(long, default_value_t = false)]
    print_config: bool,
}

fn get_cli_value<T>(matches: &ArgMatches, name: &'static str) -> Option<T>
where
    T: Clone + Send + Sync + 'static,
{
    if matches.value_source(name) != Some(ValueSource::CommandLine) {
        return None;
    }
    matches.get_one(name).cloned()
}

pub fn get_config() -> Result<Config, anyhow::Error> {
    let default_config = Config::default();

    let ClapConfig {
        config_path,
        print_config,
        command,
        ..
    } = ClapConfig::parse();

    if let Some(crate::diagnostics::cli::AppCommand::Diagnostics { command }) = command {
        let runtime = crate::diagnostics::control::runtime_dir()?;
        let status = crate::diagnostics::control::request(&runtime, &command)?;
        println!("{}", serde_json::to_string_pretty(&status)?);
        process::exit(0);
    }

    if print_config {
        println!("{}", toml::to_string_pretty(&default_config)?);
        process::exit(0);
    }

    let toml_slice = match fs::read(config_path) {
        Ok(bytes) => toml::from_slice(&bytes).context("invalid config file"),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(ConfigSlice::default()),
        Err(e) => Err(Error::new(e).context("failed to read config file")),
    }?;

    let matches = ClapConfig::command().get_matches();
    let cli_slice = ConfigSlice {
        endpoint: get_cli_value(&matches, "endpoint"),
        nssdb_path: get_cli_value(&matches, "nssdb_path"),
        log: get_cli_value(&matches, "log"),
        max_connections: get_cli_value(&matches, "max_connections"),
        max_idle_seconds: get_cli_value(&matches, "max_idle_seconds"),
        pin_prompt: get_cli_value(&matches, "pin_prompt"),
        allow_soft_tokens: get_cli_value(&matches, "allow_soft_tokens"),
        show_window: get_cli_value(&matches, "show_window"),
        allow_missing_origin: get_cli_value(&matches, "allow_missing_origin"),
        selection_path: get_cli_value(&matches, "selection_path"),
    };

    let config = default_config.merge(toml_slice).merge(cli_slice);

    config.validate()?;
    Ok(config)
}

impl Config {
    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.max_connections > 0,
            "--max-connections must be at least 1"
        );
        #[cfg(feature = "ui-gtk")]
        let gui = matches!(self.pin_prompt, PinPromptKind::Ui);
        #[cfg(not(feature = "ui-gtk"))]
        let gui = false;
        anyhow::ensure!(
            !self.show_window || gui,
            "--show-window requires a GTK build and --pin-prompt ui"
        );
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_origin_is_explicit_and_cli_false_overrides_toml_true() {
        assert!(!Config::default().allow_missing_origin);
        let toml: ConfigSlice = toml::from_str("allow_missing_origin = true").unwrap();
        let config = Config::default().merge(toml);
        assert!(config.allow_missing_origin);
        let matches = ClapConfig::command()
            .try_get_matches_from(["epoxy", "--allow-missing-origin=false"])
            .unwrap();
        let cli = ConfigSlice {
            allow_missing_origin: get_cli_value(&matches, "allow_missing_origin"),
            ..Default::default()
        };
        assert!(!config.merge(cli).allow_missing_origin);
        let matches = ClapConfig::command()
            .try_get_matches_from(["epoxy", "--allow-missing-origin"])
            .unwrap();
        assert_eq!(
            get_cli_value::<bool>(&matches, "allow_missing_origin"),
            Some(true)
        );
        assert!(
            Config {
                max_connections: 0,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
    #[test]
    fn show_window_requires_gui_prompt_and_honors_config_precedence() {
        let mut config = Config {
            pin_prompt: PinPromptKind::Tty,
            show_window: true,
            ..Default::default()
        };
        assert!(config.validate().is_err());
        config.show_window = false;
        assert!(config.validate().is_ok());
        let slice: ConfigSlice = toml::from_str("show_window = true").unwrap();
        assert!(config.merge(slice).show_window);
        #[cfg(feature = "ui-gtk")]
        {
            let config = Config::default().merge(ConfigSlice {
                show_window: Some(true),
                ..Default::default()
            });
            assert!(config.validate().is_ok());
        }
    }
}
