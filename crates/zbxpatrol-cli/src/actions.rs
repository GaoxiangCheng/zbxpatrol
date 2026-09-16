//! 子命令实现（CLI / 交互向导 / HTTP serve 共用）。
//! 约定：数据走 stdout，日志与进度走 stderr；错误返回退出码（2/3）。

use crate::render;
use crate::Format;
use futures::StreamExt;
use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::Arc;
use zbxpatrol_core::env::AppConfig;
use zbxpatrol_core::errors::{PatrolError, Result};
use zbxpatrol_core::patrol_config::PatrolToml;
use zbxpatrol_core::pipeline::{self, InspectOptions};
use zbxpatrol_core::timerange::{TimeRange, TimeSpec};
use zbxpatrol_core::types::{HostInfo, ItemRec, Scope};
use serde_json::json;
use zbxpatrol_core::zabbix::ZabbixClient;

fn die(e: PatrolError) -> i32 {
    eprintln!("zbxpatrol: {e}");
    e.exit_code()
}

/// 是否允许交互（--no-interactive 或非 TTY 时关闭）
static INTERACTIVE_ALLOWED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn set_interactive_allowed(v: bool) {
    INTERACTIVE_ALLOWED.store(v, std::sync::atomic::Ordering::Relaxed);
}

fn can_prompt() -> bool {
    INTERACTIVE_ALLOWED.load(std::sync::atomic::Ordering::Relaxed) && std::io::stdin().is_terminal()
}

/// 读取配置；缺失必填项且可交互时，进入首次初始化（提示输入并保存家目录）
pub async fn load_app_config() -> Result<AppConfig> {
    match AppConfig::from_env() {
        Ok(c) => Ok(c),
        Err(PatrolError::MissingEnv(m)) => {
            if can_prompt() {
                ensure_config().await?;
                AppConfig::from_env()
            } else {
                Err(PatrolError::MissingEnv(m))
            }
        }
        Err(e) => Err(e),
    }
}

/// 交互式初始化：逐项提示输入，验证连通后写入 ~/.zbxpatrol/config.env（600）
pub async fn ensure_config() -> Result<()> {
    use dialoguer::{Input, Password, Select};
    let save_path = zbxpatrol_core::env::home_config_path().ok_or_else(|| {
        PatrolError::Config("无法确定家目录（HOME 未设置），请改用环境变量".into())
    })?;
    println!("未检测到 Zabbix 连接配置，进入首次初始化（将保存到 {}）", save_path.display());
    let url: String = Input::new()
        .with_prompt("Zabbix 地址（如 https://zabbix.example.com）")
        .validate_with(|v: &String| if v.starts_with("http") { Ok(()) } else { Err("需以 http(s):// 开头") })
        .interact()
        .map_err(|e| PatrolError::Config(format!("输入取消：{e}")))?;
    let user: String = Input::new()
        .with_prompt("用户名（需开启 API 访问权限）")
        .default("admin".into())
        .interact()
        .map_err(|e| PatrolError::Config(format!("输入取消：{e}")))?;
    let pass: String = Password::new()
        .with_prompt("密码（输入不回显）")
        .interact()
        .map_err(|e| PatrolError::Config(format!("输入取消：{e}")))?;
    let insecure = Select::new()
        .with_prompt("是否跳过 TLS 证书校验（自签/证书链不完整时选是）")
        .items(&["否（默认，严格校验）", "是（跳过校验）"])
        .default(0)
        .interact()
        .map(|i| i == 1)
        .unwrap_or(false);
    let tz: String = Input::new()
        .with_prompt("时区")
        .default("Asia/Shanghai".into())
        .interact()
        .unwrap_or_else(|_| "Asia/Shanghai".into());

    // 先应用并验证连通，成功才落盘
    std::env::set_var("ZBX_URL", &url);
    std::env::set_var("ZBX_USER", &user);
    std::env::set_var("ZBX_PASSWORD", &pass);
    std::env::set_var("ZBX_INSECURE", if insecure { "true" } else { "false" });
    std::env::set_var("ZBX_TZ", &tz);
    let cfg = AppConfig::from_env()?;
    let client = ZabbixClient::shared(&cfg)?;
    client.login().await?;
    let groups = client.get_groups().await?;
    client.logout().await;

    if let Some(dir) = save_path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| PatrolError::Config(format!("创建 {} 失败：{e}", dir.display())))?;
    }
    let content = format!(
        "# zbxpatrol 配置（交互初始化生成）\nZBX_URL={url}\nZBX_USER={user}\nZBX_PASSWORD={pass}\nZBX_INSECURE={}\nZBX_TZ={tz}\n",
        if insecure { "true" } else { "false" }
    );
    std::fs::write(&save_path, content)
        .map_err(|e| PatrolError::Config(format!("写入 {} 失败：{e}", save_path.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&save_path, std::fs::Permissions::from_mode(0o600));
    }
    println!("✓ 连接验证成功（{} 个主机群组），配置已保存：{}", groups.len(), save_path.display());
    Ok(())
}

/// 配置 + 登录客户端
pub async fn connect() -> Result<(AppConfig, Arc<ZabbixClient>)> {
    let cfg = load_app_config().await?;
    let client = ZabbixClient::shared(&cfg)?;
    client.login().await?;
    Ok((cfg, client))
}

fn progress_bar(quiet: bool, total: usize) -> Option<indicatif::ProgressBar> {
    if quiet || !std::io::stderr().is_terminal() || total == 0 {
        return None;
    }
    let pb = indicatif::ProgressBar::new(total as u64);
    pb.set_style(
        indicatif::ProgressStyle::with_template("{spinner:.green} 巡检进度 {pos}/{len} 台")
            .unwrap_or_else(|_| indicatif::ProgressStyle::default_bar()),
    );
    Some(pb)
}

// ---------- check ----------

pub async fn do_check(fmt: Format) -> i32 {
    let cfg = match load_app_config().await {
        Ok(c) => c,
        Err(e) => return die(e),
    };
    let client = match ZabbixClient::shared(&cfg) {
        Ok(c) => c,
        Err(e) => return die(e),
    };
    let mut steps: Vec<(String, bool, String)> = Vec::new();
    let mut fail_code = 0;
    match client.api_version().await {
        Ok(v) => steps.push((format!("连接 {}", cfg.url), true, format!("Zabbix API 版本 {v}"))),
        Err(e) => {
            steps.push((format!("连接 {}", cfg.url), false, e.to_string()));
            fail_code = 3;
        }
    }
    if fail_code == 0 {
        match client.login().await {
            Ok(_) => steps.push(("登录（ZBX_USER/ZBX_PASSWORD）".into(), true, "认证成功".into())),
            Err(e) => {
                steps.push(("登录（ZBX_USER/ZBX_PASSWORD）".into(), false, e.to_string()));
                fail_code = 2;
            }
        }
    }
    if fail_code == 0 {
        match client.get_hosts(None, None).await {
            Ok(hs) => steps.push(("数据读取权限".into(), true, format!("可见主机 {} 台", hs.len()))),
            Err(e) => {
                steps.push(("数据读取权限".into(), false, e.to_string()));
                fail_code = 3;
            }
        }
        client.logout().await;
    }
    if fmt == Format::Json {
        let arr: Vec<serde_json::Value> = steps
            .iter()
            .map(|(s, ok, d)| serde_json::json!({"step": s, "ok": ok, "detail": d}))
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({"ok": fail_code == 0, "steps": arr})).unwrap()
        );
    } else {
        for (s, ok, d) in &steps {
            let mark = if *ok { "✓" } else { "✗" };
            eprintln!(" {mark} {s}  {d}");
        }
        if fail_code == 0 {
            eprintln!("自检通过：环境就绪。");
        }
    }
    fail_code
}

// ---------- groups / hosts ----------

/// 过滤 + 分页（page 从 1 起；size=None 或 0 表示不分页）
fn filter_page<T, F: Fn(&T) -> String>(list: Vec<T>, search: Option<&str>, page: Option<usize>, size: Option<usize>, label: F) -> (Vec<T>, bool) {
    let mut out: Vec<T> = list;
    if let Some(s) = search.filter(|s| !s.is_empty()) {
        let s = s.to_lowercase();
        out.retain(|t| label(t).to_lowercase().contains(&s));
    }
    let total = out.len();
    if let Some(n) = size.filter(|n| *n > 0) {
        let p = page.unwrap_or(1); // 只给 --size 时默认第 1 页
        let start = p.saturating_sub(1) * n;
        if start < out.len() {
            out = out.split_off(start);
            out.truncate(n);
        } else {
            out.clear();
        }
    }
    let truncated = total > out.len();
    (out, truncated)
}

pub async fn do_groups(search: Option<String>, page: Option<usize>, size: Option<usize>, fmt: Format) -> i32 {
    let (_cfg, client) = match connect().await {
        Ok(v) => v,
        Err(e) => return die(e),
    };
    let r = client.get_groups().await;
    client.logout().await;
    match r {
        Ok(groups) => {
            let (groups, truncated) = filter_page(groups, search.as_deref(), page, size, |g| g.name.clone());
            if truncated && fmt == Format::Table {
                eprintln!("（已分页，更多结果请用 --page/--size）");
            }
            render::groups(&groups, fmt);
            0
        }
        Err(e) => die(e),
    }
}

/// hosts 子命令：主机列表（系统类型/IP/群组；过滤与分页）
pub async fn do_hosts(
    group: Option<String>,
    search: Option<String>,
    page: Option<usize>,
    size: Option<usize>,
    fmt: Format,
) -> i32 {
    let (cfg, client) = match connect().await {
        Ok(v) => v,
        Err(e) => return die(e),
    };
    let scope = match &group {
        Some(g) => Scope::Groups(vec![g.clone()]),
        None => Scope::All,
    };
    let r = zbxpatrol_core::discovery::resolve_hosts(&client, &scope).await;
    match r {
        Ok(hosts) => {
            let (mut hosts, truncated) =
                filter_page(hosts, search.as_deref(), page, size, |h| {
                    format!("{} {} {}", h.host, h.name, h.ip)
                });
            fetch_os_family(&client, &mut hosts, cfg.concurrency).await;
            client.logout().await;
            if truncated && fmt == Format::Table {
                eprintln!("（已分页，更多结果请用 --page/--size）");
            }
            render::hosts_list(&hosts, fmt);
            0
        }
        Err(e) => {
            client.logout().await;
            die(e)
        }
    }
}

/// 为主机列表补充 OS 类型（Linux/Windows…；并发拉取 system.uname/sw.os）
pub async fn fetch_os_family(client: &Arc<ZabbixClient>, hosts: &mut [HostInfo], concurrency: usize) {
    let sem = Arc::new(tokio::sync::Semaphore::new(concurrency.max(1)));
    let mut jobs = Vec::new();
    for h in hosts.iter() {
        let client = client.clone();
        let hostid = h.hostid.clone();
        let sem = sem.clone();
        jobs.push(async move {
            let _p = sem.acquire_owned().await.unwrap();
            let params = json!({
                "hostids": [hostid],
                "output": ["itemid","key_","name","value_type","units","lastvalue","lastclock"],
                "filter": {"key_": ["system.uname", "system.sw.os", "system.sw.os[short]"]},
                "limit": 5,
            });
            let v = client.call("item.get", params).await?;
            Ok::<_, PatrolError>(parse_items(&v, ""))
        });
    }
    let mut stream = futures::stream::iter(jobs).buffer_unordered(concurrency.max(1));
    let mut idx = 0usize;
    while let Some(r) = stream.next().await {
        if let Ok(items) = r {
            if let Some(h) = hosts.get_mut(idx) {
                h.os_family = zbxpatrol_core::discovery::os_family_of(&items).0;
            }
        }
        idx += 1;
    }
}

fn parse_items(v: &serde_json::Value, hostid: &str) -> Vec<ItemRec> {
    v.as_array()
        .map(|arr| {
            arr.iter()
                .map(|it| ItemRec {
                    itemid: it["itemid"].as_str().unwrap_or_default().into(),
                    hostid: hostid.into(),
                    key: it["key_"].as_str().unwrap_or_default().into(),
                    name: it["name"].as_str().unwrap_or_default().into(),
                    value_type: it["value_type"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0),
                    units: it["units"].as_str().unwrap_or_default().into(),
                    lastvalue: it["lastvalue"].as_str().map(|s| s.to_string()),
                    lastclock: it["lastclock"].as_str().and_then(|s| s.parse().ok()),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 主机列表（含系统类型），CLI 与向导共用
pub async fn hosts_with_os(client: &Arc<ZabbixClient>, group: Option<&str>, concurrency: usize) -> Result<Vec<HostInfo>> {
    let groupids = match group {
        Some(g) => {
            let gs = client.get_groups().await?;
            match gs.iter().find(|x| x.name == g) {
                Some(found) => Some(vec![found.groupid.clone()]),
                None => {
                    return Err(PatrolError::Config(format!("未找到群组 {g:?}（zbxpatrol groups 查看）")));
                }
            }
        }
        None => None,
    };
    let mut hosts = client.get_hosts(groupids.as_deref(), None).await?;
    fetch_os_family(client, &mut hosts, concurrency).await;
    Ok(hosts)
}


// ---------- shell 补全 ----------

/// 供 bash/zsh 补全调用：输出群组名或主机名（快速路径，不取 OS，失败静默）
pub async fn do_complete(kind: &str, host: Option<&str>, group: Option<&str>) -> i32 {
    let cfg = match AppConfig::from_env() {
        Ok(c) => c,
        Err(_) => return 0,
    };
    let client = match ZabbixClient::shared(&cfg) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    if let Err(e) = client.login().await {
        let _ = e;
        return 0;
    }
    let out: Vec<String> = match kind {
        "groups" => client.get_groups().await.map(|gs| gs.into_iter().map(|g| g.name).collect()).unwrap_or_default(),
        "hosts" => {
            // If --group is given, filter hosts to that group
            let groupids = match group {
                Some(g) => {
                    let gs = client.get_groups().await.unwrap_or_default();
                    gs.iter().find(|x| x.name == g).map(|f| vec![f.groupid.clone()])
                }
                None => None,
            };
            client
                .get_hosts(groupids.as_deref(), None)
                .await
                .map(|hs| hs.into_iter().map(|h| h.host).collect())
                .unwrap_or_default()
        }
        "items" => match host {
            Some(h) => {
                let hs = client.get_hosts(None, Some(&[h.to_string()])).await.unwrap_or_default();
                match hs.first() {
                    Some(hi) => client
                        .get_items(&hi.hostid)
                        .await
                        .map(|its| its.into_iter().filter(|i| i.numeric()).map(|i| i.key).collect())
                        .unwrap_or_default(),
                    None => Vec::new(),
                }
            }
            None => Vec::new(),
        },
        _ => Vec::new(),
    };
    client.logout().await;
    for line in out {
        println!("{line}");
    }
    0
}

const BASH_COMPLETION: &str = r#"# zbxpatrol bash completion (subcommands/options/values; dynamically filters already-used options)
_zbxpatrol() {
    local cur prev sub bin i opt used
    COMPREPLY=()
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev="${COMP_WORDS[COMP_CWORD-1]}"
    sub="${COMP_WORDS[1]}"
    bin="${COMP_WORDS[0]}"
    local global="--format --quiet --no-interactive --verbose --help --lang"
    local subopts=""
    case "$sub" in
        groups)    subopts="--search --page --size" ;;
        hosts)     subopts="--group --search --page --size" ;;
        items)     subopts="--host --group --search --detail" ;;
        query)     subopts="--key --host --group --hosts --period --last --from --to --csv" ;;
        chart)     subopts="--host --metric --key --period --last --from --to" ;;
        report)    subopts="--group --host --hosts --strictness --all-items --raw --data-json --out --config --period --last --from --to" ;;
        serve)     subopts="--listen --token" ;;
    esac

    # Value completion for options that take a specific value
    case "$prev" in
        --group)
            COMPREPLY=($(compgen -W "$("$bin" __complete groups 2>/dev/null)" -- "$cur")); return 0 ;;
        --host|--hosts)
            local g="" h="" j
            for ((j=1; j<COMP_CWORD; j++)); do
                if [ "${COMP_WORDS[j]}" = "--group" ]; then g="${COMP_WORDS[j+1]}"; fi
            done
            local hargs=""
            [ -n "$g" ] && hargs="--group $g"
            COMPREPLY=($(compgen -W "$("$bin" __complete hosts $hargs 2>/dev/null)" -- "$cur")); return 0 ;;
        --metric)
            COMPREPLY=($(compgen -W "cpu mem disk" -- "$cur")); return 0 ;;
        --strictness)
            COMPREPLY=($(compgen -W "loose standard strict" -- "$cur")); return 0 ;;
        --period)
            COMPREPLY=($(compgen -W "day week month year" -- "$cur")); return 0 ;;
        --format)
            COMPREPLY=($(compgen -W "table json csv" -- "$cur")); return 0 ;;
        --lang)
            COMPREPLY=($(compgen -W "en zh" -- "$cur")); return 0 ;;
        --key)
            # Complete item keys based on --host or --hosts for chart/query
            local h="" j
            for ((j=1; j<COMP_CWORD; j++)); do
                if [ "${COMP_WORDS[j]}" = "--host" ] || [ "${COMP_WORDS[j]}" = "--hosts" ]; then h="${COMP_WORDS[j+1]}"; fi
            done
            # --hosts can be comma-separated; take the first host
            h="${h%%,*}"
            if [ -n "$h" ]; then
                COMPREPLY=($(compgen -W "$("$bin" __complete items --host "$h" 2>/dev/null)" -- "$cur"))
                return 0
            fi
            return 0 ;;
        --search|--from|--to|--last|--out|--data-json|--csv|--config|--listen|--token|--page|--size)
            return 0 ;;
    esac

    # Level 1: subcommand completion
    if [ "$COMP_CWORD" -eq 1 ]; then
        local subs="check serve groups items query chart report completions"
        COMPREPLY=($(compgen -W "$subs" -- "$cur"))
        return 0
    fi

    # Level 2+: offer remaining options, filtering out already-used ones
    local available="$subopts $global"
    for ((i=2; i<COMP_CWORD; i++)); do
        opt="${COMP_WORDS[i]}"
        case "$opt" in
            --*) available="${available//$opt /}" ;;
        esac
    done
    COMPREPLY=($(compgen -W "$available" -- "$cur"))
    return 0
}
complete -F _zbxpatrol zbxpatrol
"#;

const ZSH_COMPLETION: &str = r#"#compdef zbxpatrol
# zbxpatrol zsh completion (dynamic group/host/item names; filters used options)
_zbxpatrol() {
    local -a subs
    subs=(check serve groups items query chart report completions)
    if (( CURRENT == 2 )); then
        _describe 'command' subs
        return
    fi
    local sub="$words[2]"
    case $words[CURRENT-1] in
        --group)
            local -a gs
            gs=(${(f)"$($words[1] __complete groups 2>/dev/null)"})
            _describe 'group' gs
            return ;;
        --host|--hosts)
            local g="" i
            for (( i=2; i<CURRENT; i++ )); do
                [[ "$words[i]" == "--group" ]] && g="$words[i+1]"
            done
            local -a hs
            if [[ -n "$g" ]]; then
                hs=(${(f)"$($words[1] __complete hosts --group "$g" 2>/dev/null)"})
            else
                hs=(${(f)"$($words[1] __complete hosts 2>/dev/null)"})
            fi
            _describe 'host' hs
            return ;;
        --key|--keys)
            local h="" i
            for (( i=2; i<CURRENT; i++ )); do
                [[ "$words[i]" == "--host" ]] && h="$words[i+1]"
            done
            if [[ -n "$h" ]]; then
                local -a ks
                ks=(${(f)"$($words[1] __complete items --host "$h" 2>/dev/null)"})
                _describe 'key' ks
            fi
            return ;;
        --metric)
            _values 'metric' cpu mem disk
            return ;;
        --strictness)
            _values 'strictness' loose standard strict
            return ;;
        --period)
            _values 'period' day week month year
            return ;;
        --format)
            _values 'format' table json csv
            return ;;
        --lang)
            _values 'lang' en zh
            return ;;
    esac
    local -a opts
    opts=(--format --lang --quiet --no-interactive --verbose --help)
    case "$sub" in
        groups) opts+=(--search --page --size) ;;
        hosts)  opts+=(--group --search --page --size) ;;
        items)  opts+=(--host --group --search --detail) ;;
        query)  opts+=(--key --host --group --hosts --period --last --from --to --csv) ;;
        chart)  opts+=(--host --metric --key --period --last --from --to) ;;
        report) opts+=(--group --host --hosts --strictness --all-items --raw --data-json --out --config --period --last --from --to) ;;
        serve)  opts+=(--listen --token) ;;
    esac
    _describe 'option' opts
}
compdef _zbxpatrol zbxpatrol
_zbxpatrol "$@"
"#;

pub fn do_completions(shell: &str) -> i32 {
    match shell {
        "zsh" => {
            println!("{ZSH_COMPLETION}");
            0
        }
        "bash" => {
            println!("{BASH_COMPLETION}");
            0
        }
        other => {
            eprintln!("zbxpatrol: 不支持的 shell {other:?}（仅支持 bash | zsh）");
            2
        }
    }
}

// ---------- 趋势图 ----------

pub async fn do_chart(host: String, metric: String, key: Option<String>, spec: TimeSpec, fmt: Format) -> i32 {
    let (cfg, client) = match connect().await {
        Ok(v) => v,
        Err(e) => return die(e),
    };
    let range = match TimeRange::resolve(&spec, cfg.tz()) {
        Ok(r) => r,
        Err(e) => return die(e),
    };
    let r = pipeline::host_metric_series(client.clone(), &host, &metric, &range, 72, key.as_deref()).await;
    client.logout().await;
    match r {
        Ok(s) => {
            if fmt == Format::Csv {
                return die(PatrolError::Config(
                    "chart 不支持 csv 输出（支持 table | json）".into(),
                ));
            }
            if fmt == Format::Json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "title": s.title, "unit": s.unit,
                        "from": s.from, "till": s.till, "series": s.series,
                    }))
                    .unwrap()
                );
            } else {
                render::ascii_chart(&s.title, &s.unit, &s.series, &range.fmt_from(), &range.fmt_till());
            }
            0
        }
        Err(e) => die(e),
    }
}

// ---------- items（数据函数供 CLI 与向导复用） ----------

pub struct ItemAggRow {
    pub key: String,
    pub name: String,
    pub unit: String,
    pub vt_label: String,
    pub hosts: usize,
}

impl ItemAggRow {
    #[allow(dead_code)]
    pub fn unit_label(&self) -> String {
        if self.unit.is_empty() { "—".into() } else { self.unit.clone() }
    }
}

pub struct ItemDetailRow {
    pub key: String,
    pub name: String,
    pub lastvalue: String,
    pub units: String,
}

/// 范围内全部监控项按 key 聚合（filter 匹配 key/名称子串）
pub async fn items_aggregated_data(scope: &Scope, filter: &str) -> Result<Vec<ItemAggRow>> {
    let (cfg, client) = connect().await?;
    let hosts = zbxpatrol_core::discovery::resolve_hosts(&client, scope).await?;
    let sem = Arc::new(tokio::sync::Semaphore::new(cfg.concurrency));
    let mut jobs = Vec::new();
    for h in hosts {
        let client = client.clone();
        let sem = sem.clone();
        jobs.push(async move {
            let _p = sem.acquire_owned().await.unwrap();
            let items = client.get_items(&h.hostid).await?;
            Ok::<_, PatrolError>(items)
        });
    }
    let mut all: Vec<ItemRec> = Vec::new();
    let mut stream = futures::stream::iter(jobs).buffer_unordered(cfg.concurrency);
    while let Some(r) = stream.next().await {
        all.extend(r?);
    }
    client.logout().await;
    let mut agg: BTreeMap<String, (String, String, u8, usize)> = BTreeMap::new();
    for it in &all {
        let e = agg
            .entry(it.key.clone())
            .or_insert((it.name.clone(), it.units.clone(), it.value_type, 0));
        e.3 += 1;
    }
    let fl = filter.to_lowercase();
    let mut rows: Vec<ItemAggRow> = agg
        .into_iter()
        .filter(|(k, (name, _, _, _))| {
            fl.is_empty() || k.to_lowercase().contains(&fl) || name.to_lowercase().contains(&fl)
        })
        .map(|(key, (name, unit, vt, hosts))| ItemAggRow {
            key,
            name,
            unit,
            vt_label: match vt {
                0 => "float".into(),
                3 => "uint".into(),
                _ => "text".into(),
            },
            hosts,
        })
        .collect();
    rows.sort_by(|a, b| b.hosts.cmp(&a.hosts).then(a.key.cmp(&b.key)));
    Ok(rows)
}

/// 单主机逐条明细（含当前值）；numeric_only=true 仅返回数值型（出图/巡检用）
pub async fn items_detail_data(host: &str, filter: &str) -> Result<Vec<ItemDetailRow>> {
    items_detail_data_opt(host, filter, false).await
}

pub async fn items_detail_data_opt(
    host: &str,
    filter: &str,
    numeric_only: bool,
) -> Result<Vec<ItemDetailRow>> {
    let (_cfg, client) = connect().await?;
    let hosts = client.get_hosts(None, Some(&[host.to_string()])).await?;
    let Some(h) = hosts.first() else {
        client.logout().await;
        return Err(PatrolError::Config(format!("未找到主机 {host:?}")));
    };
    let items = client.get_items(&h.hostid).await?;
    client.logout().await;
    let fl = filter.to_lowercase();
    Ok(items
        .into_iter()
        .filter(|it| !numeric_only || it.numeric())
        .filter(|it| {
            fl.is_empty()
                || it.key.to_lowercase().contains(&fl)
                || it.name.to_lowercase().contains(&fl)
        })
        .map(|it| ItemDetailRow {
            key: it.key,
            name: it.name,
            lastvalue: it.lastvalue.unwrap_or_default(),
            units: it.units,
        })
        .collect())
}

pub async fn do_items(
    host: Option<String>,
    group: Option<String>,
    search: Option<String>,
    detail: bool,
    fmt: Format,
) -> i32 {
    let scope = if let Some(h) = host {
        Scope::Hosts(vec![h])
    } else if let Some(g) = group {
        Scope::Groups(vec![g])
    } else {
        Scope::All
    };
    let filter = search.unwrap_or_default();
    if detail {
        let h = match &scope {
            Scope::Hosts(hs) => hs.first().cloned(),
            _ => None,
        };
        let Some(h) = h else {
            eprintln!("--detail 需要配合 --host 使用");
            return 2;
        };
        match items_detail_data(&h, &filter).await {
            Ok(rows) => {
                let view: Vec<(String, String, String, String, String)> = rows
                    .iter()
                    .map(|r| ("—".into(), r.key.clone(), r.name.clone(), r.lastvalue.clone(), r.units.clone()))
                    .collect();
                render::items_detail(&view, fmt);
                0
            }
            Err(e) => die(e),
        }
    } else {
        match items_aggregated_data(&scope, &filter).await {
            Ok(rows) => {
                let view: Vec<(String, String, String, String, usize)> = rows
                    .iter()
                    .map(|r| (r.key.clone(), r.name.clone(), r.unit.clone(), r.vt_label.clone(), r.hosts))
                    .collect();
                render::items_aggregated(&view, fmt);
                0
            }
            Err(e) => die(e),
        }
    }
}

// ---------- query ----------

pub async fn do_query(
    keys: Vec<String>,
    scope: Scope,
    spec: TimeSpec,
    csv: Option<PathBuf>,
    fmt: Format,
) -> i32 {
    let (cfg, client) = match connect().await {
        Ok(v) => v,
        Err(e) => return die(e),
    };
    let range = match TimeRange::resolve(&spec, cfg.tz()) {
        Ok(r) => r,
        Err(e) => return die(e),
    };
    let r = pipeline::run_query(client.clone(), &scope, &keys, &range, cfg.concurrency).await;
    client.logout().await;
    let rows = match r {
        Ok(rows) => rows,
        Err(e) => return die(e),
    };
    if let Some(path) = csv {
        if let Err(e) = render::query_csv_file(&rows, &path) {
            return die(PatrolError::Config(format!("CSV 写入失败：{e}")));
        }
        eprintln!("CSV 已写入 {}", path.display());
    }
    if fmt == Format::Csv {
        render::query_csv_stdout(&rows);
        return 0;
    }
    render::query(&rows, &range, fmt);
    0
}

// ---------- report ----------

pub struct ReportParams {
    pub scope: Scope,
    pub time: TimeSpec,
    /// 评分严格度：loose|standard|strict（缺省 standard）
    pub strictness: Option<String>,
    pub all_items: bool,
    /// 附加自定义指标 key（支持通配）
    pub extra_keys: Vec<String>,
    pub data_json: Option<PathBuf>,
    /// 主机明细 CSV 文件导出
    pub csv_out: Option<PathBuf>,
    pub out: PathBuf,
    pub patrol_config: Option<PathBuf>,
    pub fmt: Format,
    pub quiet: bool,
    /// 仅控制台表格输出，不生成 xlsx 文件（向导用）
    pub console_only: bool,
    /// 导出原始逐条数据（--raw）
    pub raw: bool,
}

pub async fn do_report(p: ReportParams) -> i32 {
    let cfg = match load_app_config().await {
        Ok(c) => c,
        Err(e) => return die(e),
    };
    let mode = match p.strictness.as_deref().map(zbxpatrol_core::scoring::Strictness::parse) {
        Some(None) => {
            return die(PatrolError::Config(format!(
                "未知评分严格度 {:?}（支持 loose/standard/strict，即 宽松/标准/严格）",
                p.strictness
            )))
        }
        Some(Some(m)) => m,
        None => zbxpatrol_core::scoring::Strictness::Standard,
    };
    let patrol = match &p.patrol_config {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(s) => match PatrolToml::load_str(&s) {
                Ok(t) => t,
                Err(e) => return die(PatrolError::Config(e)),
            },
            Err(e) => return die(PatrolError::Config(format!("读取 {} 失败：{e}", path.display()))),
        },
        None => PatrolToml::default(),
    };
    let range = match TimeRange::resolve(&p.time, cfg.tz()) {
        Ok(r) => r,
        Err(e) => return die(e),
    };
    let client = match ZabbixClient::shared(&cfg) {
        Ok(c) => c,
        Err(e) => return die(e),
    };
    if let Err(e) = client.login().await {
        return die(e);
    }
    if !p.quiet {
        eprintln!("巡检范围：{}   时间区间：{}", p.scope.label(), range.fmt_human());
    }
    let host_count = {
        match zbxpatrol_core::discovery::resolve_hosts(&client, &p.scope).await {
            Ok(hs) => hs.len(),
            Err(e) => {
                client.logout().await;
                return die(e);
            }
        }
    };
    let pb = progress_bar(p.quiet, host_count);
    let progress: Option<pipeline::ProgressFn> = pb.as_ref().map(|pb| {
        let pb = pb.clone();
        Arc::new(move |done: usize, total: usize| {
            pb.set_length(total as u64);
            pb.set_position(done as u64);
        }) as pipeline::ProgressFn
    });
    let opts = InspectOptions {
        all_items: p.all_items,
        patrol,
        strictness: mode,
        charts: true,
        extra_keys: p.extra_keys.clone(),
        raw: p.raw,
    };
    let r = pipeline::run_inspection(client.clone(), &p.scope, &range, &opts, cfg.concurrency, progress).await;
    client.logout().await;
    if let Some(pb) = pb {
        pb.finish_and_clear();
    }
    let outcome = match r {
        Ok(o) => o,
        Err(e) => return die(e),
    };
    let data = &outcome.data;

    // CSV 附加导出（--csv / 向导输出选择），console_only 与常规路径均生效
    if let Some(csvp) = &p.csv_out {
        if let Err(e) = render::report_csv_file(data, csvp) {
            return die(PatrolError::Config(format!("CSV 写入失败：{e}")));
        }
    }

    // console_only：不生成 xlsx，仅控制台输出（可选 CSV/JSON 附加导出）
    if p.console_only {
        if let Some(csvp) = &p.csv_out {
            println!("CSV 文件 : {}", csvp.display());
        }
        if let Some(dj) = &p.data_json {
            let j = serde_json::to_string_pretty(data).unwrap_or_default();
            if let Err(e) = std::fs::write(dj, j) {
                eprintln!("zbxpatrol: JSON 写入失败：{e}");
            } else {
                println!("JSON 文件: {}", dj.display());
            }
        }
        render::report_summary(data, None, None);
        render::report_console_detail(data);
        if outcome.missing {
            eprintln!("警告：部分主机/指标数据缺失（详见标注）");
        }
        return if outcome.missing { 4 } else { 0 };
    }

    // 输出目录与文件名
    if let Err(e) = std::fs::create_dir_all(&p.out) {
        return die(PatrolError::Config(format!("创建输出目录失败：{e}")));
    }
    let safe_scope = data
        .scope_names
        .first()
        .cloned()
        .unwrap_or_else(|| match data.scope_type.as_str() {
            "all" => "全部主机".to_string(),
            _ => "自定义范围".to_string(),
        })
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' })
        .collect::<String>();
    let xlsx_path = p.out.join(format!("巡检报告_{}_{}.xlsx", range.fmt_compact(), safe_scope));
    if let Err(e) = render::xlsx::render_report(data, &xlsx_path) {
        return die(PatrolError::Config(format!("Excel 生成失败：{e}")));
    }
    if let Some(dj) = &p.data_json {
        let j = serde_json::to_string_pretty(data).unwrap_or_default();
        if let Err(e) = std::fs::write(dj, j) {
            return die(PatrolError::Config(format!("JSON 写入失败：{e}")));
        }
    }

    match p.fmt {
        Format::Json => println!("{}", serde_json::to_string_pretty(data).unwrap_or_default()),
        Format::Csv => render::report_csv_stdout(data),
        Format::Table => {
            render::report_summary(data, Some(&xlsx_path), p.data_json.as_deref());
            render::report_console_detail(data);
        }
    }
    if outcome.missing {
        eprintln!("警告：部分主机/指标数据缺失（详见报表标注），退出码 4");
        return 4;
    }
    0
}
