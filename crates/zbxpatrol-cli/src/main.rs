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
#[command(next_help_heading = "Scope (default: all hosts; when several are given: --host > --hosts > --group)")]
struct ScopeArgs {
    /// Filter by host group (repeatable; lowest precedence)
    #[arg(long)]
    group: Vec<String>,
    /// Single host (exact name, case-sensitive; highest precedence)
    #[arg(long)]
    host: Option<String>,
    /// Multiple hosts, comma-separated
    #[arg(long, value_delimiter = ',')]
    hosts: Vec<String>,
}

impl ScopeArgs {
    /// 范围参数同时给出时按精确度取优先级：--host > --hosts > --group
    fn scope(&self) -> Scope {
        if let Some(h) = &self.host {
            Scope::Hosts(vec![h.clone()])
        } else if !self.hosts.is_empty() {
            Scope::Hosts(self.hosts.clone())
        } else if !self.group.is_empty() {
            Scope::Groups(self.group.clone())
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

    /// List hosts (system type, IP, groups; filter and paginate)
    Hosts {
        /// Filter by group name
        #[arg(long)]
        group: Option<String>,
        /// Filter by host/IP substring
        #[arg(long)]
        search: Option<String>,
        /// Page number (1-based; requires --size)
        #[arg(long)]
        page: Option<usize>,
        /// Page size (items per page)
        #[arg(long)]
        size: Option<usize>,
    },

    /// Discover available item keys (aggregated across hosts; --detail for per-host current values)
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

    /// Historical stats + full-size plot for item keys (use `items` to discover keys)
    #[command(after_help = "Examples:\n  zbxpatrol query --key system.cpu.util --last 7d\n  zbxpatrol query --key 'net.if*' --group <group> --csv out.csv\n  zbxpatrol query --host <host> --key system.cpu.util --last 24h --chart   # table + full-size plot\n\nUse `items --search <word>` to discover keys. cpu/mem/disk presets:\n  zbxpatrol query --host <host> --key system.cpu.util|vm.memory.util --chart")]
    Query {
        /// Item key (repeatable; supports wildcards like net.if*)
        #[arg(long = "key", required = true)]
        keys: Vec<String>,
        /// Also draw a full-size trend chart; requires the query to match exactly ONE series
        #[arg(long)]
        chart: bool,
        #[command(flatten)]
        scope: ScopeArgs,
        #[command(flatten)]
        time: TimeArgs,
        /// Export results to a CSV file
        #[arg(long)]
        csv: Option<PathBuf>,
    },

    /// Full-size trend plot for a single host+key (deep-dive; use `query` to compare across hosts)

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
        /// Extra custom metrics to append (comma-separated keys, wildcards ok) → dedicated sheet
        #[arg(long = "keys", value_delimiter = ',')]
        keys: Vec<String>,
        /// Export raw history samples (one row per data point)
        #[arg(long)]
        raw: bool,
        /// Also save structured JSON to this file
        #[arg(long = "data-json")]
        data_json: Option<PathBuf>,
        /// Also save a flat CSV to this file
        #[arg(long = "csv")]
        csv_out: Option<PathBuf>,
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
    after_help = "Run without subcommand for interactive wizard.\n\nExit codes: 0 OK | 2 config/credentials | 3 network/API | 4 partial data missing\nData on stdout, logs on stderr. Non-TTY auto-disables interaction.\nEach subcommand has -h with examples."
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
    // 管道场景（如 `... | head`）下游关闭时按 POSIX 惯例静默退出，而不是 panic
    #[cfg(unix)]
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL); }
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
        Hosts { group, search, page, size } => {
            actions::do_hosts(group, search, page, size, fmt).await
        }
        
        Completions { shell } => actions::do_completions(shell.as_deref().unwrap_or("bash")),
        Complete { kind, host, group } => actions::do_complete(&kind, host.as_deref(), group.as_deref()).await,
        Items { host, group, search, detail } => {
            actions::do_items(host, group, search, detail, fmt).await
        }
        Query { keys, chart, scope, time, csv } => match time.spec() {
            Ok(spec) => actions::do_query(keys, chart, scope.scope(), spec, csv, fmt).await,
            Err(e) => {
                eprintln!("zbxpatrol: {e}");
                2
            }
        },
        Report { scope, time, strictness, all_items, keys, raw, data_json, csv_out, out, patrol_config } => match time.spec() {
            Ok(spec) => {
                actions::do_report(actions::ReportParams {
                    scope: scope.scope(),
                    time: spec,
                    strictness: strictness.clone(),
                    all_items,
                    extra_keys: keys,
                    data_json,
                    csv_out,
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
