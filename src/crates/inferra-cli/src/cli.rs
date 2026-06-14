use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

const ROOT_AFTER_HELP: &str = "\
Examples:
  inferra
  inferra status
  inferra runtime start
  inferra runtime open
  inferra serve
  inferra collectors status
  inferra ai ask \"What changed in the last hour?\"
  inferra service repair";

#[derive(Parser, Debug)]
#[command(
    name = "inferra",
    version,
    about = "Inferra runtime and incident investigation CLI",
    styles = crate::ui::clap_styles(),
    after_help = ROOT_AFTER_HELP
)]
pub struct Cli {
    #[arg(long, global = true, help = "Path to inferra.toml")]
    pub config: Option<PathBuf>,
    #[arg(long, global = true, help = "Emit machine-readable JSON only")]
    pub json: bool,
    #[arg(long, global = true, help = "Override the bundled UI directory")]
    pub ui_dist: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    #[command(about = "Show a welcome screen with version, runtime status, and next commands")]
    Status,
    #[command(about = "Start the local Inferra HTTP runtime and dashboard (foreground dev mode)")]
    Serve,
    #[command(about = "Start, stop, and inspect the API + web dashboard runtime")]
    Runtime {
        #[command(subcommand)]
        action: Option<RuntimeAction>,
    },
    #[command(
        about = "Manage the Windows service install (API + dashboard run inside the service)"
    )]
    Service {
        #[command(subcommand)]
        action: Option<ServiceAction>,
        #[arg(long, hide = true)]
        service_run: bool,
    },
    #[command(about = "Create or update inferra.toml")]
    Setup {
        #[arg(long, help = "Skip confirmation prompts")]
        yes: bool,
        #[arg(long, help = "Reserved for compatibility with older setup flows")]
        skip_connection_test: bool,
        #[arg(long, help = "Override storage.data_dir before writing the config")]
        data_dir: Option<PathBuf>,
    },
    #[command(name = "init-db", about = "Initialize the SQLite databases")]
    InitDb,
    #[command(about = "Inspect incidents")]
    Incidents {
        #[command(subcommand)]
        action: IncidentAction,
    },
    #[command(about = "Inspect raw events")]
    Events {
        #[command(subcommand)]
        action: EventAction,
    },
    #[command(about = "Inspect service health and recent activity")]
    Services {
        #[command(subcommand)]
        action: ServiceDataAction,
    },
    #[command(about = "Inspect or control collectors")]
    Collectors {
        #[command(subcommand)]
        action: CollectorAction,
    },
    #[command(about = "Read or update merged configuration values")]
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    #[command(about = "Explore workspace-to-service mappings")]
    Workspace {
        #[command(subcommand)]
        action: WorkspaceAction,
    },
    #[command(about = "Run AI diagnostics and investigations")]
    Ai {
        #[command(subcommand)]
        action: AiAction,
    },
}

#[derive(Subcommand, Debug)]
pub enum RuntimeAction {
    #[command(about = "Show whether the API and dashboard are running")]
    Status,
    #[command(about = "Start the installed Windows service (API + dashboard)")]
    Start,
    #[command(about = "Stop the installed Windows service")]
    Stop,
    #[command(about = "Restart the installed Windows service")]
    Restart,
    #[command(about = "Open the dashboard in your default browser")]
    Open,
}

#[derive(Subcommand, Debug)]
pub enum ServiceAction {
    #[command(about = "Install the Windows service")]
    Install {
        #[arg(long, default_value = "auto")]
        startup: String,
    },
    #[command(about = "Remove the Windows service")]
    Remove,
    #[command(about = "Start the Windows service")]
    Start,
    #[command(about = "Stop the Windows service")]
    Stop,
    #[command(about = "Restart the Windows service")]
    Restart,
    #[command(about = "Show Windows service status")]
    Status,
    #[command(about = "Validate service prerequisites and suggest next steps")]
    Repair,
}

#[derive(Subcommand, Debug)]
pub enum IncidentAction {
    #[command(about = "List active incidents")]
    List(ListArgs),
    #[command(about = "Show one incident with hypotheses and state log")]
    Show { incident_id: String },
}

#[derive(Subcommand, Debug)]
pub enum EventAction {
    #[command(about = "List events")]
    List(EventListArgs),
    #[command(about = "Show one event")]
    Show { event_id: String },
}

#[derive(Subcommand, Debug)]
pub enum ServiceDataAction {
    #[command(about = "List services")]
    List(ListArgs),
    #[command(about = "Show one service with recent events")]
    Show { service_id: String },
    #[command(about = "List recent events for a service")]
    Events {
        service_id: String,
        #[command(flatten)]
        args: ListArgs,
    },
}

#[derive(Subcommand, Debug)]
pub enum CollectorAction {
    #[command(about = "Show collector status")]
    Status,
    #[command(about = "Request collector start through the local API")]
    Start,
    #[command(about = "Request collector stop through the local API")]
    Stop,
}

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    #[command(about = "Show the merged configuration")]
    Show,
    #[command(about = "Get one config value by dotted path")]
    Get { key: String },
    #[command(about = "Set one config value by dotted path")]
    Set { key: String, value: String },
    #[command(about = "Apply a named preset")]
    Preset { name: String },
}

#[derive(Subcommand, Debug)]
pub enum WorkspaceAction {
    #[command(about = "Build the workspace map")]
    Map,
    #[command(about = "Show service mapping status")]
    Services,
    #[command(about = "Inspect a file or folder path through the API")]
    Inspect { path: String },
    #[command(about = "List detected projects")]
    Projects {
        #[arg(long, default_value_t = 4)]
        max_depth: usize,
        #[arg(long, default_value_t = 100)]
        max_results: usize,
    },
}

#[derive(Subcommand, Debug)]
pub enum AiAction {
    #[command(about = "Show AI provider status")]
    Status,
    #[command(about = "Run the AI doctor checks")]
    Doctor,
    #[command(about = "Ask the investigator a free-form question")]
    Ask {
        question: Option<String>,
        #[arg(long, default_value = "overview")]
        scope: String,
        #[arg(long)]
        mode: Option<String>,
        #[arg(long)]
        monitor_seconds: Option<u64>,
    },
    #[command(about = "Generate a report for one incident")]
    Report {
        incident_id: String,
        #[arg(long)]
        mode: Option<String>,
        #[arg(long)]
        monitor_seconds: Option<u64>,
    },
    #[command(about = "Run an investigation against the local runtime")]
    Investigate {
        #[command(subcommand)]
        target: Option<InvestigateTarget>,
        #[arg(long)]
        mode: Option<String>,
        /// Wall-clock seconds to sample CPU/memory for the investigation bundle (0 skips timed monitor).
        #[arg(long)]
        monitor_seconds: Option<u64>,
    },
}

#[derive(Subcommand, Debug)]
pub enum InvestigateTarget {
    #[command(about = "Investigate the latest incident or runtime state")]
    Latest,
    #[command(about = "Investigate one incident")]
    Incident { incident_id: String },
    #[command(about = "Investigate one service")]
    Service { service_id: String },
}

#[derive(Args, Debug, Clone)]
pub struct ListArgs {
    #[arg(long, default_value_t = 25)]
    pub limit: usize,
}

#[derive(Args, Debug, Clone)]
pub struct EventListArgs {
    #[arg(long, default_value_t = 100)]
    pub limit: usize,
    #[arg(long)]
    pub service: Option<String>,
    #[arg(long)]
    pub severity: Option<i64>,
    #[arg(long)]
    pub search: Option<String>,
    #[arg(long)]
    pub source_type: Option<String>,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn root_command_defaults_to_landing_screen() {
        let cli = Cli::try_parse_from(["inferra"]).expect("parse root");
        assert!(cli.command.is_none());
    }

    #[test]
    fn parser_accepts_service_repair_as_native_command() {
        let cli = Cli::try_parse_from(["inferra", "service", "repair"]).expect("parse repair");
        match cli.command {
            Some(Command::Service {
                action: Some(ServiceAction::Repair),
                ..
            }) => {}
            other => panic!("unexpected parse result: {other:?}"),
        }
    }

    #[test]
    fn parser_rejects_unknown_command_groups() {
        let error = Cli::try_parse_from(["inferra", "--json", "guide", "--profile", "operator"])
            .expect_err("guide should be rejected");
        let rendered = error.to_string();
        assert!(rendered.contains("unrecognized subcommand"));
    }

    #[test]
    fn parser_accepts_incidents_and_ai_commands() {
        let cli = Cli::try_parse_from(["inferra", "incidents", "list"]).expect("parse incidents");
        assert!(matches!(
            cli.command,
            Some(Command::Incidents {
                action: IncidentAction::List(_)
            })
        ));

        let ai = Cli::try_parse_from(["inferra", "ai", "investigate", "service", "api"])
            .expect("parse ai investigate");
        assert!(matches!(
            ai.command,
            Some(Command::Ai {
                action: AiAction::Investigate {
                    target: Some(InvestigateTarget::Service { .. }),
                    ..
                }
            })
        ));
    }
}
