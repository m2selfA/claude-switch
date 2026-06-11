use super::*;
use clap::Parser;
use std::path::PathBuf;

#[test]
fn parses_nested_provider_commands() {
    let cli = Cli::try_parse_from([
        "cswitch",
        "provider",
        "add",
        "Example",
        "--url",
        "https://api.example.invalid",
        "--key",
        "sk-test-generated-key-777777777777777777777777",
    ])
    .unwrap();

    match cli.command {
        Some(Commands::Provider {
            command: ProviderCommands::Add { name, url, key },
        }) => {
            assert_eq!(name, "Example");
            assert_eq!(url, "https://api.example.invalid");
            assert_eq!(key, "sk-test-generated-key-777777777777777777777777");
        }
        _ => panic!("unexpected command parse result"),
    }
}

#[test]
fn parses_duplicate_command() {
    let cli = Cli::try_parse_from([
        "cswitch",
        "duplicate",
        "work",
        "work copy",
        "--alias",
        "work-copy",
    ])
    .unwrap();

    match cli.command {
        Some(Commands::Duplicate {
            source,
            new_name,
            alias,
        }) => {
            assert_eq!(source, "work");
            assert_eq!(new_name, "work copy");
            assert_eq!(alias.as_deref(), Some("work-copy"));
        }
        _ => panic!("unexpected duplicate parse result"),
    }
}

#[test]
fn parses_config_settings_commands() {
    let cli = Cli::try_parse_from(["cswitch", "config", "settings", "show", "--json"]).unwrap();
    match cli.command {
        Some(Commands::Config {
            command:
                ConfigCommands::Settings {
                    command: ConfigSettingsCommands::Show { json },
                },
        }) => assert!(json),
        _ => panic!("unexpected config settings show parse result"),
    }

    let cli = Cli::try_parse_from([
        "cswitch",
        "config",
        "settings",
        "set",
        "--allow-local-runtime-hot-switch",
        "--json",
    ])
    .unwrap();
    match cli.command {
        Some(Commands::Config {
            command:
                ConfigCommands::Settings {
                    command:
                        ConfigSettingsCommands::Set {
                            allow_local_runtime_hot_switch,
                            json,
                            ..
                        },
                },
        }) => {
            assert!(allow_local_runtime_hot_switch);
            assert!(json);
        }
        _ => panic!("unexpected config settings set parse result"),
    }
}

#[test]
fn parses_provider_rename_key_command() {
    let cli = Cli::try_parse_from([
        "cswitch",
        "provider",
        "rename-key",
        "prov_12345678",
        "key_12345678",
        "--name",
        "Team A",
    ])
    .unwrap();

    match cli.command {
        Some(Commands::Provider {
            command: ProviderCommands::RenameKey { id, key_id, name },
        }) => {
            assert_eq!(id, "prov_12345678");
            assert_eq!(key_id, "key_12345678");
            assert_eq!(name, "Team A");
        }
        _ => panic!("unexpected provider rename-key parse result"),
    }
}

#[test]
fn old_flat_provider_commands_no_longer_parse() {
    assert!(
        Cli::try_parse_from([
            "cswitch",
            "provider-add",
            "Example",
            "--url",
            "https://api.example.invalid",
            "--key",
            "sk-test-generated-key-888888888888888888888888",
        ])
        .is_err()
    );

    assert!(Cli::try_parse_from(["cswitch", "providers"]).is_err());
}

#[test]
fn aliases_remote_option_parses() {
    let cli =
        Cli::try_parse_from(["cswitch", "aliases", "--remote", "devbox", "--verbose"]).unwrap();
    match cli.command {
        Some(Commands::Aliases {
            local,
            remote,
            verbose,
        }) => {
            assert!(!local);
            assert_eq!(remote, vec!["devbox"]);
            assert!(verbose);
        }
        _ => panic!("unexpected aliases parse result"),
    }
}

#[test]
fn aliases_verbose_only_parses() {
    let cli = Cli::try_parse_from(["cswitch", "aliases", "--verbose"]).unwrap();
    match cli.command {
        Some(Commands::Aliases {
            local,
            remote,
            verbose,
        }) => {
            assert!(!local);
            assert!(remote.is_empty());
            assert!(verbose);
        }
        _ => panic!("unexpected aliases parse result"),
    }
}

#[test]
fn aliases_multiple_remote_options_parse() {
    let cli = Cli::try_parse_from([
        "cswitch", "aliases", "--remote", "host1", "--remote", "host2",
    ])
    .unwrap();
    match cli.command {
        Some(Commands::Aliases {
            local,
            remote,
            verbose,
        }) => {
            assert!(!local);
            assert_eq!(remote, vec!["host1", "host2"]);
            assert!(!verbose);
        }
        _ => panic!("unexpected aliases parse result"),
    }
}

#[test]
fn aliases_local_option_parses() {
    let cli = Cli::try_parse_from(["cswitch", "aliases", "--local"]).unwrap();
    match cli.command {
        Some(Commands::Aliases {
            local,
            remote,
            verbose,
        }) => {
            assert!(local);
            assert!(remote.is_empty());
            assert!(!verbose);
        }
        _ => panic!("unexpected aliases parse result"),
    }
}

#[test]
fn aliases_local_and_remote_options_parse() {
    let cli = Cli::try_parse_from([
        "cswitch",
        "aliases",
        "--local",
        "--remote",
        "host1",
        "--remote",
        "host2",
        "--verbose",
    ])
    .unwrap();
    match cli.command {
        Some(Commands::Aliases {
            local,
            remote,
            verbose,
        }) => {
            assert!(local);
            assert_eq!(remote, vec!["host1", "host2"]);
            assert!(verbose);
        }
        _ => panic!("unexpected aliases parse result"),
    }
}

#[test]
fn parses_nested_mcp_add_command() {
    let cli = Cli::try_parse_from([
        "cswitch",
        "mcp",
        "add",
        "github",
        "--type",
        "stdio",
        "--command",
        "npx",
        "--arg",
        "-y",
        "--arg",
        "@modelcontextprotocol/server-github",
        "--env",
        "GITHUB_TOKEN=${GITHUB_TOKEN}",
        "--always-load",
        "false",
    ])
    .unwrap();

    match cli.command {
        Some(Commands::Mcp {
            command:
                McpCommands::Add {
                    name,
                    server_type,
                    command,
                    args,
                    env,
                    always_load,
                    ..
                },
        }) => {
            assert_eq!(name, "github");
            assert_eq!(server_type, "stdio");
            assert_eq!(command.as_deref(), Some("npx"));
            assert_eq!(args, vec!["-y", "@modelcontextprotocol/server-github"]);
            assert_eq!(env, vec!["GITHUB_TOKEN=${GITHUB_TOKEN}"]);
            assert_eq!(always_load, Some(false));
        }
        _ => panic!("unexpected mcp add parse result"),
    }
}

#[test]
fn parses_nested_mcp_link_command() {
    let cli = Cli::try_parse_from([
        "cswitch",
        "mcp",
        "link",
        "work",
        "github",
        "filesystem",
        "--replace",
    ])
    .unwrap();

    match cli.command {
        Some(Commands::Mcp {
            command:
                McpCommands::Link {
                    profile,
                    mcps,
                    replace,
                },
        }) => {
            assert_eq!(profile, "work");
            assert_eq!(mcps, vec!["github", "filesystem"]);
            assert!(replace);
        }
        _ => panic!("unexpected mcp link parse result"),
    }
}

#[test]
fn parses_diagnostics_and_shell_commands() {
    let cli = Cli::try_parse_from(["cswitch", "doctor", "--json", "--strict"]).unwrap();
    match cli.command {
        Some(Commands::Doctor { json, strict }) => {
            assert!(json);
            assert!(strict);
        }
        _ => panic!("unexpected doctor parse result"),
    }

    let cli = Cli::try_parse_from(["cswitch", "config", "inspect", "--json"]).unwrap();
    match cli.command {
        Some(Commands::Config {
            command: ConfigCommands::Inspect { json },
        }) => assert!(json),
        _ => panic!("unexpected config inspect parse result"),
    }

    let cli = Cli::try_parse_from([
        "cswitch",
        "config",
        "export",
        "--profile",
        "work",
        "--output",
        "bundle.json",
        "--include-secrets",
    ])
    .unwrap();
    match cli.command {
        Some(Commands::Config {
            command:
                ConfigCommands::Export {
                    profiles,
                    output,
                    include_secrets,
                },
        }) => {
            assert_eq!(profiles, vec!["work"]);
            assert_eq!(output, Some(PathBuf::from("bundle.json")));
            assert!(include_secrets);
        }
        _ => panic!("unexpected config export parse result"),
    }

    let cli =
        Cli::try_parse_from(["cswitch", "config", "import", "bundle.json", "--replace"]).unwrap();
    match cli.command {
        Some(Commands::Config {
            command:
                ConfigCommands::Import {
                    input,
                    replace,
                    dry_run,
                    json,
                },
        }) => {
            assert_eq!(input, PathBuf::from("bundle.json"));
            assert!(replace);
            assert!(!dry_run);
            assert!(!json);
        }
        _ => panic!("unexpected config import parse result"),
    }

    let cli = Cli::try_parse_from([
        "cswitch",
        "config",
        "import",
        "bundle.json",
        "--replace",
        "--json",
    ])
    .unwrap();
    match cli.command {
        Some(Commands::Config {
            command:
                ConfigCommands::Import {
                    input,
                    replace,
                    dry_run,
                    json,
                },
        }) => {
            assert_eq!(input, PathBuf::from("bundle.json"));
            assert!(replace);
            assert!(!dry_run);
            assert!(json);
        }
        _ => panic!("unexpected config import json parse result"),
    }

    let cli = Cli::try_parse_from([
        "cswitch",
        "config",
        "import",
        "bundle.json",
        "--dry-run",
        "--json",
    ])
    .unwrap();
    match cli.command {
        Some(Commands::Config {
            command:
                ConfigCommands::Import {
                    input,
                    replace,
                    dry_run,
                    json,
                },
        }) => {
            assert_eq!(input, PathBuf::from("bundle.json"));
            assert!(!replace);
            assert!(dry_run);
            assert!(json);
        }
        _ => panic!("unexpected config import dry-run parse result"),
    }

    let cli =
        Cli::try_parse_from(["cswitch", "config", "validate", "bundle.json", "--strict"]).unwrap();
    match cli.command {
        Some(Commands::Config {
            command:
                ConfigCommands::Validate {
                    input,
                    json,
                    strict,
                },
        }) => {
            assert_eq!(input, PathBuf::from("bundle.json"));
            assert!(!json);
            assert!(strict);
        }
        _ => panic!("unexpected config validate parse result"),
    }

    let cli = Cli::try_parse_from([
        "cswitch",
        "config",
        "recover-shims",
        "shims",
        "--write",
        "--replace",
        "--json",
    ])
    .unwrap();
    match cli.command {
        Some(Commands::Config {
            command:
                ConfigCommands::RecoverShims {
                    shim_dir,
                    write,
                    replace,
                    json,
                },
        }) => {
            assert_eq!(shim_dir, PathBuf::from("shims"));
            assert!(write);
            assert!(replace);
            assert!(json);
        }
        _ => panic!("unexpected recover-shims parse result"),
    }

    let cli = Cli::try_parse_from([
        "cswitch",
        "config",
        "migrate-auth",
        "--write",
        "--json",
        "--remote",
        "gm00",
        "--remote",
        "devbox",
    ])
    .unwrap();
    match cli.command {
        Some(Commands::Config {
            command:
                ConfigCommands::MigrateAuth {
                    write,
                    json,
                    remote,
                },
        }) => {
            assert!(write);
            assert!(json);
            assert_eq!(remote, vec!["gm00", "devbox"]);
        }
        _ => panic!("unexpected migrate-auth parse result"),
    }

    let cli =
        Cli::try_parse_from(["cswitch", "statusline", "--profile", "work", "--json"]).unwrap();
    match cli.command {
        Some(Commands::Statusline { profile, dir, json }) => {
            assert_eq!(profile.as_deref(), Some("work"));
            assert!(dir.is_none());
            assert!(json);
        }
        _ => panic!("unexpected statusline parse result"),
    }

    let cli = Cli::try_parse_from(["cswitch", "shell", "hook", "--shell", "bash"]).unwrap();
    match cli.command {
        Some(Commands::Shell {
            command: ShellCommands::Hook { shell },
        }) => assert_eq!(shell, "bash"),
        _ => panic!("unexpected shell hook parse result"),
    }

    let cli = Cli::try_parse_from(["cswitch", "process", "list"]).unwrap();
    match cli.command {
        Some(Commands::Process {
            command: ProcessCommands::List,
        }) => {}
        _ => panic!("unexpected process list parse result"),
    }

    let cli = Cli::try_parse_from([
        "cswitch",
        "process",
        "switch",
        "proc_ab12cd34",
        "--provider",
        "prov_12345678",
        "--key",
        "key_12345678",
        "--model",
        "claude-3-7-sonnet",
    ])
    .unwrap();
    match cli.command {
        Some(Commands::Process {
            command:
                ProcessCommands::Switch {
                    session_id,
                    provider,
                    key,
                    model,
                },
        }) => {
            assert_eq!(session_id, "proc_ab12cd34");
            assert_eq!(provider, "prov_12345678");
            assert_eq!(key, "key_12345678");
            assert_eq!(model, "claude-3-7-sonnet");
        }
        _ => panic!("unexpected process switch parse result"),
    }

    let cli = Cli::try_parse_from(["cswitch", "runtime", "auth", "proc_ab12cd34"]).unwrap();
    match cli.command {
        Some(Commands::Runtime {
            command: RuntimeCommands::Auth { session_id },
        }) => assert_eq!(session_id, "proc_ab12cd34"),
        _ => panic!("unexpected runtime auth parse result"),
    }

    let cli = Cli::try_parse_from([
        "cswitch",
        "use",
        "local",
        "--local-gateway-mode",
        "fetch-only",
        "--",
        "--resume",
    ])
    .unwrap();
    match cli.command {
        Some(Commands::Use {
            name,
            no_extras,
            local_gateway_mode,
            args,
        }) => {
            assert_eq!(name, "local");
            assert!(!no_extras);
            assert_eq!(local_gateway_mode.as_deref(), Some("fetch-only"));
            assert_eq!(args, vec!["--resume"]);
        }
        _ => panic!("unexpected use parse result"),
    }

    let cli = Cli::try_parse_from([
        "cswitch",
        "shim",
        "launch",
        "--profile-id",
        "profile-generated",
        "--local-gateway-mode",
        "search-fetch",
        "--no-extras",
        "--",
        "--resume",
    ])
    .unwrap();
    match cli.command {
        Some(Commands::Shim {
            command:
                ShimCommands::Launch {
                    probe,
                    profile_id,
                    no_extras,
                    local_gateway_mode,
                    args,
                },
        }) => {
            assert!(!probe);
            assert_eq!(profile_id.as_deref(), Some("profile-generated"));
            assert!(no_extras);
            assert_eq!(local_gateway_mode.as_deref(), Some("search-fetch"));
            assert_eq!(args, vec!["--resume"]);
        }
        _ => panic!("unexpected shim launch parse result"),
    }
}

#[test]
fn parses_mcp_export_import_validate_commands() {
    let cli = Cli::try_parse_from(["cswitch", "mcp", "export", "github", "--output", "mcp.json"])
        .unwrap();
    match cli.command {
        Some(Commands::Mcp {
            command:
                McpCommands::Export {
                    queries,
                    all,
                    output,
                },
        }) => {
            assert_eq!(queries, vec!["github"]);
            assert!(!all);
            assert_eq!(output, Some(PathBuf::from("mcp.json")));
        }
        _ => panic!("unexpected mcp export parse result"),
    }

    let cli = Cli::try_parse_from(["cswitch", "mcp", "import", "mcp.json", "--replace"]).unwrap();
    match cli.command {
        Some(Commands::Mcp {
            command: McpCommands::Import { input, replace },
        }) => {
            assert_eq!(input, PathBuf::from("mcp.json"));
            assert!(replace);
        }
        _ => panic!("unexpected mcp import parse result"),
    }

    let cli = Cli::try_parse_from(["cswitch", "mcp", "validate", "--all", "--strict"]).unwrap();
    match cli.command {
        Some(Commands::Mcp {
            command:
                McpCommands::Validate {
                    queries,
                    all,
                    strict,
                },
        }) => {
            assert!(queries.is_empty());
            assert!(all);
            assert!(strict);
        }
        _ => panic!("unexpected mcp validate parse result"),
    }
}
