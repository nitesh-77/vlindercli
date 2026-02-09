mod agent;
mod daemon;
mod fleet;
mod help;
mod model;
mod repl;
mod timeline;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "vlinder")]
#[command(about = "Run AI agents locally")]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug, PartialEq)]
pub enum Command {
    /// Manage and run agents
    Agent {
        #[command(subcommand)]
        cmd: agent::AgentCommand,
    },
    /// Run a fleet of agents
    Fleet {
        #[command(subcommand)]
        cmd: fleet::FleetCommand,
    },
    /// Interactive support — runs the bundled support fleet
    Support,
    /// Manage models from catalogs
    Model {
        #[command(subcommand)]
        cmd: model::ModelCommand,
    },
    /// Inspect and fork the system timeline
    Timeline {
        #[command(subcommand)]
        cmd: timeline::TimelineCommand,
    },
    /// Run the vlinder daemon
    Daemon,
}

pub fn run() {
    let cli = Cli::parse();

    match cli.command {
        Command::Agent { cmd } => agent::execute(cmd),
        Command::Fleet { cmd } => fleet::execute(cmd),
        Command::Support => help::execute(),
        Command::Model { cmd } => model::execute(cmd),
        Command::Timeline { cmd } => timeline::execute(cmd),
        Command::Daemon => daemon::execute(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::path::PathBuf;

    #[test]
    fn cli_agent_run_default_path() {
        let cli = Cli::try_parse_from(["vlinder", "agent", "run"]).unwrap();
        assert_eq!(
            cli.command,
            Command::Agent {
                cmd: agent::AgentCommand::Run { path: None }
            }
        );
    }

    #[test]
    fn cli_agent_run_with_short_path() {
        let cli = Cli::try_parse_from(["vlinder", "agent", "run", "-p", "/tmp/agent"]).unwrap();
        assert_eq!(
            cli.command,
            Command::Agent {
                cmd: agent::AgentCommand::Run {
                    path: Some(PathBuf::from("/tmp/agent")),
                }
            }
        );
    }

    #[test]
    fn cli_agent_run_with_long_path() {
        let cli =
            Cli::try_parse_from(["vlinder", "agent", "run", "--path", "./my-agent"]).unwrap();
        assert_eq!(
            cli.command,
            Command::Agent {
                cmd: agent::AgentCommand::Run {
                    path: Some(PathBuf::from("./my-agent")),
                }
            }
        );
    }

    #[test]
    fn cli_missing_subcommand_fails() {
        let result = Cli::try_parse_from(["vlinder"]);
        assert!(result.is_err());
    }

    #[test]
    fn cli_agent_missing_action_fails() {
        let result = Cli::try_parse_from(["vlinder", "agent"]);
        assert!(result.is_err());
    }

    #[test]
    fn cli_agent_list() {
        let cli = Cli::try_parse_from(["vlinder", "agent", "list"]).unwrap();
        assert_eq!(
            cli.command,
            Command::Agent {
                cmd: agent::AgentCommand::List
            }
        );
    }

    #[test]
    fn cli_timeline_log() {
        let cli = Cli::try_parse_from(["vlinder", "timeline", "log"]).unwrap();
        assert_eq!(
            cli.command,
            Command::Timeline {
                cmd: timeline::TimelineCommand::Log { agent: None }
            }
        );
    }

    #[test]
    fn cli_timeline_fork() {
        let cli = Cli::try_parse_from(["vlinder", "timeline", "fork", "abc123"]).unwrap();
        assert_eq!(
            cli.command,
            Command::Timeline {
                cmd: timeline::TimelineCommand::Fork {
                    commit: "abc123".to_string(),
                }
            }
        );
    }

    #[test]
    fn cli_agent_get() {
        let cli = Cli::try_parse_from(["vlinder", "agent", "get", "pensieve"]).unwrap();
        assert_eq!(
            cli.command,
            Command::Agent {
                cmd: agent::AgentCommand::Get {
                    name: "pensieve".to_string()
                }
            }
        );
    }

    #[test]
    fn cli_fleet_run_default_path() {
        let cli = Cli::try_parse_from(["vlinder", "fleet", "run"]).unwrap();
        assert_eq!(
            cli.command,
            Command::Fleet {
                cmd: fleet::FleetCommand::Run { path: None }
            }
        );
    }

    #[test]
    fn cli_fleet_run_with_path() {
        let cli = Cli::try_parse_from(["vlinder", "fleet", "run", "-p", "/tmp/fleet"]).unwrap();
        assert_eq!(
            cli.command,
            Command::Fleet {
                cmd: fleet::FleetCommand::Run {
                    path: Some(PathBuf::from("/tmp/fleet")),
                }
            }
        );
    }

    #[test]
    fn cli_fleet_new() {
        let cli = Cli::try_parse_from(["vlinder", "fleet", "new", "my-fleet"]).unwrap();
        assert_eq!(
            cli.command,
            Command::Fleet {
                cmd: fleet::FleetCommand::New {
                    name: "my-fleet".to_string(),
                }
            }
        );
    }

    #[test]
    fn cli_fleet_new_missing_name_fails() {
        let result = Cli::try_parse_from(["vlinder", "fleet", "new"]);
        assert!(result.is_err());
    }

    #[test]
    fn cli_fleet_missing_action_fails() {
        let result = Cli::try_parse_from(["vlinder", "fleet"]);
        assert!(result.is_err());
    }

    #[test]
    fn cli_support() {
        let cli = Cli::try_parse_from(["vlinder", "support"]).unwrap();
        assert_eq!(cli.command, Command::Support);
    }

    #[test]
    fn cli_agent_new_python() {
        let cli = Cli::try_parse_from(["vlinder", "agent", "new", "python", "my-agent"]).unwrap();
        assert_eq!(
            cli.command,
            Command::Agent {
                cmd: agent::AgentCommand::New {
                    language: agent::Language::Python,
                    name: "my-agent".to_string(),
                }
            }
        );
    }

    #[test]
    fn cli_agent_new_golang() {
        let cli = Cli::try_parse_from(["vlinder", "agent", "new", "golang", "bar"]).unwrap();
        assert_eq!(
            cli.command,
            Command::Agent {
                cmd: agent::AgentCommand::New {
                    language: agent::Language::Golang,
                    name: "bar".to_string(),
                }
            }
        );
    }

    #[test]
    fn cli_agent_new_js() {
        let cli = Cli::try_parse_from(["vlinder", "agent", "new", "js", "foo"]).unwrap();
        assert_eq!(
            cli.command,
            Command::Agent {
                cmd: agent::AgentCommand::New {
                    language: agent::Language::Js,
                    name: "foo".to_string(),
                }
            }
        );
    }

    #[test]
    fn cli_agent_new_ts() {
        let cli = Cli::try_parse_from(["vlinder", "agent", "new", "ts", "bun-agent"]).unwrap();
        assert_eq!(
            cli.command,
            Command::Agent {
                cmd: agent::AgentCommand::New {
                    language: agent::Language::Ts,
                    name: "bun-agent".to_string(),
                }
            }
        );
    }

    #[test]
    fn cli_agent_new_java() {
        let cli = Cli::try_parse_from(["vlinder", "agent", "new", "java", "jvm-agent"]).unwrap();
        assert_eq!(
            cli.command,
            Command::Agent {
                cmd: agent::AgentCommand::New {
                    language: agent::Language::Java,
                    name: "jvm-agent".to_string(),
                }
            }
        );
    }

    #[test]
    fn cli_agent_new_dotnet() {
        let cli = Cli::try_parse_from(["vlinder", "agent", "new", "dotnet", "net-agent"]).unwrap();
        assert_eq!(
            cli.command,
            Command::Agent {
                cmd: agent::AgentCommand::New {
                    language: agent::Language::Dotnet,
                    name: "net-agent".to_string(),
                }
            }
        );
    }

    #[test]
    fn cli_agent_new_invalid_language_fails() {
        let result = Cli::try_parse_from(["vlinder", "agent", "new", "ruby", "my-agent"]);
        assert!(result.is_err());
    }

    #[test]
    fn cli_agent_new_missing_name_fails() {
        let result = Cli::try_parse_from(["vlinder", "agent", "new", "python"]);
        assert!(result.is_err());
    }
}
