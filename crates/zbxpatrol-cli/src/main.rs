//! zbxpatrol CLI entry: subcommand dispatch, exit codes, .env loading, stderr logging.

mod actions;
mod lang;
mod render;
mod serve;
mod wizard;

use clap::{Parser, Subcommand, ValueEnum};
use std::io::IsTerminal;
use std::path::PathBuf;
use zbxpatrol_core::errors::PatrolError;
use zbxpatrol_core::timerange::{Period, TimeSpec};
use zbxpatrol_core::types::Scope;

#[derive(Copy, Clone, PartialEq, Eq, Debug, ValueEnum)]
enum Format {
    Table,
    Json,
    Csv,
}

/// Time range (pick one; priority: from/to > last > period)
#[derive(clap::Args, Debug, Default)]
#[command(next_help_heading = "Time Range (from/to > last > period, default day)")]
struct TimeArgs {
    /// Preset period: day|week|month|year (24h/7d/30d/365d)
    #[arg(long)]
    period: Option<String>,
    /// Relative duration: 48h / 15d / 12M / 30m
    #[arg(long)]
    last: Option<String>,
    /// Start time YYYY-MM-DD[ HH:MM[:SS]] (local timezone)
    #[arg(long)]
    from: Option<String>,
    /// End time (default: now)
    #[arg(long)]
    to: Option<String>,
}

impl TimeArgs {
    fn spec(&self) -> Result<TimeSpec, PatrolError> {
        if self.from.is_some() {
            return Ok(TimeSpec::FromTo { from: self.from.clone().unwrap(), to: self.to.clone() });
        }
        if let Some(l) = &self.last {
            return Ok(TimeSpec::Last(l.clone()));
        }
        let p = match self.period.as_deref() {
            None | Some("day") => Period::Day,
            Some("week") => Period::Week,
            Some("month") => Period::Month,
            Some("year") => Period::Year,
            Some(other) => {
                return Err(PatrolError::Config(format!(
                    "unknown period {other:?} (expected day|week|month|year)"
                )))
            }
        };
        Ok(TimeSpec::Period(p))
    }
}

/// Scope (--group repeatable; --host single; --hosts comma-separated; default: all)
#[derive(clap::Args, Debug, Default)]
#[command(next_help_heading = "Scope (default: all hosts)")]
struct ScopeArgs {
    /// Filter by host group (repeatable)
    #[arg(long)]
    group: Vec<String>,
    /// Single host (exact name, case-sensitive)
    #[arg(long)]
    host: Option<String>,
    /// Multiple hosts, comma-separated
    #[arg(long, value_delimiter = ',')]
    hosts: Vec<String>,
}

impl ScopeArgs {
    fn scope(&self) -> Scope {
        if !self.group.is_empty() {
            Scope::Groups(self.group.clone())
        } else if let Some(h) = &self.host {
            Scope::Hosts(vec![h.clone()])
        } else if !self.hosts.is_empty() {
            Scope::Hosts(self.hosts.clone())
        } else {
            Scope::All
        }
    }
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run connectivity, credentials and permission self-check
    Check,

    /// Start the local HTTP API server for third-party programs
    Serve {
        /// Listen address
        #[arg(long, default_value = "127.0.0.1:8787")]
        listen: String,
        /// Bearer auth token (required when exposing externally)
        #[arg(long, env = "PATROL_TOKEN")]
        token: Option<String>,
    },

    /// List host groups with optional search and pagination
    Groups {
        /// Filter by name substring
        #[arg(long)]
        search: Option<String>,
        /// Page number (1-based; requires --size)
        #[arg(long)]
        page: Option<usize>,
        /// Page size (items per page)
        #[arg(long)]
        size: Option<usize>,
    },

    /// List monitoring item keys (aggregated across hosts; --detail for per-item)
    Items {
        /// Filter by single host
        #[arg(long)]
        host: Option<String>,
        /// Filter by group name
        #[arg(long)]
        group: Option<String>,
        /// Filter by key/name substring
        #[arg(long)]
        search: Option<String>,
        /// Show per-item rows with current values (requires --host)
        #[arg(long)]
        detail: bool,
    },

    /// Query stats (cur/avg/max/min + trend sparkline) for any item key(s)
    Query {
        /// Item key (repeatable; supports wildcards like net.if*)
        #[arg(long = "key", required = true)]
        keys: Vec<String>,
        #[command(flatten)]
        scope: ScopeArgs,
        #[command(flatten)]
        time: TimeArgs,
        /// Export results to a CSV file
        #[arg(long)]
        csv: Option<PathBuf>,
    },

    /// Show a single-host trend chart (requires --host; no --group)
    #[command(after_help = "Examples:\n  zbxpatrol chart --host <host> --metric cpu --last 7d\n  zbxpatrol chart --host <host> --key 'net.if.in[\"ens3\"]' --from 2026-09-01 --to 2026-09-15\n  zbxpatrol chart --host <host> --key 'vfs.fs*pused*' --last 24h   # wildcard must match exactly one")]
    Chart {
        /// Host name
        #[arg(long)]
        host: String,
        /// Preset metric: cpu | mem | disk (mutually exclusive with --key)
        #[arg(long, default_value = "cpu")]
        metric: String,
        /// Any item key (wildcard ok, must match exactly one; overrides --metric)
        #[arg(long)]
        key: Option<String>,
        #[command(flatten)]
        time: TimeArgs,
    },

    /// Generate shell completion scripts (bash/zsh; --group/--host complete real names)
    #[command(after_help = "Install:\n\nbash:\n  zbxpatrol completions bash | sudo tee /etc/bash_completion.d/zbxpatrol\n  source /etc/bash_completion.d/zbxpatrol\n\nzsh (Kali/Ubuntu default):\n  mkdir -p ~/.zfunc\n  zbxpatrol completions zsh > ~/.zfunc/_zbxpatrol\n  echo 'fpath=(~/.zfunc $fpath)' >> ~/.zshrc\n  echo 'autoload -Uz compinit && compinit' >> ~/.zshrc\n  exec zsh\n\nVerify: zbxpatrol <TAB>")]
    Completions {
        /// Shell type: bash | zsh (default: bash)
        shell: Option<String>,
    },

    /// [hidden] Shell completion data source: output group/host/item names
    #[command(hide = true, name = "__complete", alias = "complete")]
    Complete {
        /// groups | hosts | items
        kind: String,
        /// Filter items by host (for items)
        #[arg(long)]
        host: Option<String>,
        /// Filter hosts by group (for hosts)
        #[arg(long)]
        group: Option<String>,
    },

    /// Generate an inspection report (Excel + console table + optional JSON/CSV)
    #[command(after_help = "Examples:\n  zbxpatrol report                                            # all hosts, daily\n  zbxpatrol report --group <group> --period week --strictness strict\n  zbxpatrol report --hosts <host1>,<host2> --from 2026-09-01 --to 2026-09-15\n  zbxpatrol report --group <group> --all-items --data-json out.json")]
    Report {
        #[command(flatten)]
        scope: ScopeArgs,
        #[command(flatten)]
        time: TimeArgs,
        /// Scoring strictness: loose(90%) | standard(80%) | strict(70%)
        #[arg(long)]
        strictness: Option<String>,
        /// Include an "All Items" sheet with every numeric item
        #[arg(long)]
        all_items: bool,
        /// Export raw history samples (one row per data point)
        #[arg(long)]
        raw: bool,
        /// Also save structured JSON to this file
        #[arg(long = "data-json")]
        data_json: Option<PathBuf>,
        /// Output directory (default: ./reports)
        #[arg(long, default_value = "./reports")]
        out: PathBuf,
        /// Optional patrol.toml config (custom metric rules / scoring rules)
        #[arg(long = "config")]
        patrol_config: Option<PathBuf>,
    },
}

#[derive(Parser, Debug)]
#[command(
    name = "zbxpatrol",
    version,
    about = "Zabbix server inspection & reporting tool (run without subcommand for interactive wizard)",
    disable_help_subcommand = true,
    after_help = "Commands:\n  check        Connectivity, credentials and permission self-check\n  serve        Start local HTTP API server\n  groups       List host groups (search, paginate)\n  items        List monitoring item keys (by host or group)\n  query        Get stats for any item key(s) — cur/avg/max/min + trend sparkline\n  chart        Show single-host metric trend chart (ASCII)\n  report       Generate inspection report (Excel + console + JSON/CSV)\n  completions  Generate shell completion (bash/zsh)\n\nRun without subcommand for interactive wizard.\n\nExit codes: 0 OK | 2 config/credentials | 3 network/API | 4 partial data missing\nData on stdout, logs on stderr. Non-TTY auto-disables interaction.\nEach subcommand has -h with examples."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// UI language: en|zh (default: en)
    #[arg(long, global = true, default_value = "en")]
    lang: String,
    /// Output format: table|json|csv (json/csv for programmatic use)
    #[arg(long, global = true, value_enum, default_value = "table")]
    format: Format,
    /// Suppress progress output
    #[arg(long, global = true)]
    quiet: bool,
    /// Disable interaction (auto in non-TTY)
    #[arg(long, global = true)]
    no_interactive: bool,
    /// Debug logging
    #[arg(long, global = true)]
    verbose: bool,
}

fn main() {
    // Config loading order (higher wins, never overwritten):
    //   1. process environment variables
    //   2. .env in current directory (project level)
    //   3. ~/.zbxpatrol/config.env (user level, generated by interactive setup)
    let _ = dotenvy::dotenv();
    if let Some(home_cfg) = zbxpatrol_core::env::home_config_path() {
        let _ = dotenvy::from_path(home_cfg);
    }
    let cli = Cli::parse();
    match lang::parse(&cli.lang) {
        Some(l) => lang::set(l),
        None => { eprintln!("zbxpatrol: unknown lang {:?} (en|zh)", cli.lang); std::process::exit(2); }
    }
    actions::set_interactive_allowed(!cli.no_interactive);
    let level = if cli.verbose {
        "debug"
    } else if cli.quiet {
        "warn"
    } else {
        "info"
    };
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level)),
        )
        .init();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let code = rt.block_on(run(cli));
    std::process::exit(code);
}

async fn run(cli: Cli) -> i32 {
    let Some(cmd) = cli.command else {
        // No subcommand: TTY → wizard; non-TTY → error (never hang)
        if std::io::stdin().is_terminal() && !cli.no_interactive {
            return wizard::run(&cli).await;
        }
        eprintln!("error: no subcommand given in non-interactive mode (try: zbxpatrol report --format json)");
        eprintln!("usage: zbxpatrol --help");
        return 2;
    };

    use Command::*;
    let fmt = cli.format;
    match cmd {
        Check => actions::do_check(fmt).await,
        Serve { listen, token } => serve::run(&listen, token).await,
        Groups { search, page, size } => actions::do_groups(search, page, size, fmt).await,
        
        Completions { shell } => actions::do_completions(shell.as_deref().unwrap_or("bash")),
        Complete { kind, host, group } => actions::do_complete(&kind, host.as_deref(), group.as_deref()).await,
        Chart { host, metric, key, time } => match time.spec() {
            Ok(spec) => actions::do_chart(host, metric, key, spec, fmt).await,
            Err(e) => {
                eprintln!("zbxpatrol: {e}");
                2
            }
        },
        Items { host, group, search, detail } => {
            actions::do_items(host, group, search, detail, fmt).await
        }
        Query { keys, scope, time, csv } => match time.spec() {
            Ok(spec) => actions::do_query(keys, scope.scope(), spec, csv, fmt).await,
            Err(e) => {
                eprintln!("zbxpatrol: {e}");
                2
            }
        },
        Report { scope, time, strictness, all_items, raw, data_json, out, patrol_config } => match time.spec() {
            Ok(spec) => {
                actions::do_report(actions::ReportParams {
                    scope: scope.scope(),
                    time: spec,
                    strictness: strictness.clone(),
                    all_items,
                    extra_keys: vec![],
                    data_json,
                    csv_out: None,
                    out,
                    patrol_config,
                    fmt,
                    quiet: cli.quiet,
                    console_only: false,
                    raw,
                })
                .await
            }
            Err(e) => {
                eprintln!("zbxpatrol: {e}");
                2
            }
        },
    }
}
