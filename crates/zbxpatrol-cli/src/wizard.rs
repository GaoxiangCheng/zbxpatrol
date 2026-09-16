//! Interactive wizard: number/letter select, q back, ? help, L toggle lang, Tab complete, exit quit.
//! Language: English default, Chinese via --lang zh or pressing L in the wizard.

use crate::actions::{self, ReportParams};
use crate::lang::{t, is_zh};
use rustyline::completion::{Completer, Pair};
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{CompletionType, Config, Editor, Helper};
use rustyline::{Context as RlContext, Result as RlResult};
use std::collections::HashMap;
use zbxpatrol_core::timerange::{Period, TimeRange, TimeSpec};
use zbxpatrol_core::types::{GroupRec, HostInfo, Scope};

const PAGE_SIZE: usize = 50;

// ==================== rustyline Tab completion ====================

struct CandHelper {
    cands: std::cell::RefCell<Vec<String>>,
}

impl Completer for CandHelper {
    type Candidate = Pair;
    fn complete(&self, line: &str, pos: usize, _ctx: &RlContext<'_>) -> RlResult<(usize, Vec<Pair>)> {
        let prefix = &line[..pos];
        let lower = prefix.to_lowercase();
        let mut all: Vec<Pair> = self
            .cands
            .borrow()
            .iter()
            .filter(|c| c.to_lowercase().starts_with(&lower))
            .map(|c| Pair { display: c.clone(), replacement: c.clone() })
            .collect();
        if all.len() > 12 { all.truncate(12); }
        Ok((0, all))
    }
}
impl Hinter for CandHelper { type Hint = String; }
impl Highlighter for CandHelper {}
impl Validator for CandHelper {}
impl Helper for CandHelper {}

fn make_editor() -> Editor<CandHelper, rustyline::history::DefaultHistory> {
    let config = Config::builder()
        .completion_type(CompletionType::List)
        .auto_add_history(false)
        .build();
    let mut ed = Editor::<CandHelper, rustyline::history::DefaultHistory>::with_config(config)
        .expect("editor");
    ed.set_helper(Some(CandHelper { cands: std::cell::RefCell::new(vec![]) }));
    ed
}

fn hint_line(msg: &str) {
    println!("  \x1b[2m{msg}\x1b[0m");
}

fn ask(ed: &mut Editor<CandHelper, rustyline::history::DefaultHistory>, prompt: &str, cands: Vec<String>) -> Option<String> {
    if let Some(h) = ed.helper_mut() { *h.cands.borrow_mut() = cands; }
    match ed.readline(prompt) {
        Ok(line) => { let t = line.trim().to_string(); Some(if t.is_empty() { String::new() } else { t }) }
        Err(_) => None,
    }
}

// ==================== Pick & common commands ====================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pick {
    Sel(usize),
    Back,
    ToMain,
    ToGroups,
    ToHosts,
    Exit,
    /// Language toggled via L key; caller should rebuild menu with new language
    LangChanged,
}

fn common_input(_ed: &mut Editor<CandHelper, rustyline::history::DefaultHistory>, path: &str, l: &str, in_browse: bool, help: &str, hint: &str) -> Option<Pick> {
    match l.to_lowercase().as_str() {
        "?" | "？" | "help" => {
            println!("\n{}", t("-- Help --", "── 帮助 ──"));
            println!("  [{path}]");
            println!("{help}");
            let cmds = t("Commands: ? help | q back | exit quit | L lang | m main", "命令：? 帮助 | q 返回 | exit 退出 | L 切换语言 | m 主菜单");
            let extra = if in_browse { t(" | g groups | h hosts", " | g 群组 | h 主机") } else { "" };
            println!("{cmds}{extra}");
            println!("{}", t("  Tab lists candidates (max 12); unique auto-completes.", "  Tab 列出候选（最多12个），唯一自动补全。"));
            hint_line(hint);
            None
        }
        "" => Some(Pick::Sel(usize::MAX)),
        "q" => Some(Pick::Back),
        "exit" | "quit" => Some(Pick::Exit),
        "m" => Some(Pick::ToMain),
        "g" if in_browse => Some(Pick::ToGroups),
        "h" if in_browse => Some(Pick::ToHosts),
        "l" | "lang" | "语言" => {
            let new = crate::lang::toggle();
            println!("  → {}", new.label());
            Some(Pick::LangChanged)
        }
        _ => Some(Pick::Sel(usize::MAX)),
    }
}

fn parse_pick(s: &str, len: usize) -> Option<usize> {
    if let Ok(n) = s.parse::<usize>() {
        return if n >= 1 && n <= len { Some(n - 1) } else { None };
    }
    let b = s.as_bytes();
    if b.len() == 1 && b[0].is_ascii_alphabetic() {
        let n = (b[0].to_ascii_lowercase() - b'a' + 1) as usize;
        return if n >= 1 && n <= len { Some(n - 1) } else { None };
    }
    None
}

// ==================== menu / paged_pick / multi ====================

fn menu(ed: &mut Editor<CandHelper, rustyline::history::DefaultHistory>, path: &str, prompt: &str, items: &[&str], in_browse: bool) -> Pick {
    loop {
        // Print menu items every iteration (so ?/empty/L re-displays them)
        println!("\n{prompt}");
        for (i, it) in items.iter().enumerate() { println!("  {:>2}) {it}", i + 1); }
        let hint = if is_zh() {
            format!("1-{} 编号或字母 | ? 帮助 | q 返回 | exit 退出 | L 切换语言 | m 主菜单{}", items.len(),
                if in_browse { " | g 群组 | h 主机" } else { "" })
        } else {
            format!("1-{} or a,b… | ? help | q back | exit quit | L lang | m main{}", items.len(),
                if in_browse { " | g groups | h hosts" } else { "" })
        };
        hint_line(&hint);
        let line = match ask(ed, &format!("[{path}] > "), vec![]) { Some(l) => l, None => return Pick::Exit };
        if line.is_empty() { continue; }
        let help = if is_zh() { format!("  输入 1-{} 选择菜单项", items.len()) } else { format!("  Enter 1-{} to select", items.len()) };
        if let Some(p) = common_input(ed, path, &line, in_browse, &help, &hint) {
            if p != Pick::Sel(usize::MAX) { return p; }
        } else { continue; }
        match parse_pick(&line.to_lowercase(), items.len()) {
            Some(i) => return Pick::Sel(i),
            None => println!("{}", t("  invalid, ? for help", "  无效输入，? 查看帮助")),
        }
    }
}

fn paged_pick(ed: &mut Editor<CandHelper, rustyline::history::DefaultHistory>, path: &str, title: &str, items: &[String], in_browse: bool) -> Pick {
    let mut filtered: Vec<usize> = (0..items.len()).collect();
    let mut page = 0usize;
    let mut flt = String::new();
    loop {
        let pages = filtered.len().div_ceil(PAGE_SIZE).max(1);
        let start = page * PAGE_SIZE;
        let end = (start + PAGE_SIZE).min(filtered.len());
        if is_zh() {
            println!("\n== {title}（{} 条，第 {}/{} 页，过滤：{}）==", filtered.len(), page + 1, pages,
                if flt.is_empty() { "无" } else { &flt });
        } else {
            println!("\n== {title} ({} items, page {}/{}, filter: {}) ==", filtered.len(), page + 1, pages,
                if flt.is_empty() { "none" } else { &flt });
        }
        for (i, idx) in (start..end).enumerate() { println!("  {:>3}) {}", start + i + 1, items[filtered[idx]]); }
        let page_cands: Vec<String> = (start..end).map(|i| items[filtered[i]].clone()).collect();
        let page_hint = t("num or name (unique→select; else filter) | n/p page | f filter | ? help | L lang | exit quit",
                           "编号或名称（唯一→选中；否则过滤）| n/p 翻页 | f 过滤 | ? 帮助 | L 语言 | exit 退出");
        hint_line(page_hint);
        let line = match ask(ed, &format!("[{path}] > "), page_cands) { Some(l) => l, None => return Pick::Exit };
        if line.is_empty() { continue; }
        let help = if is_zh() {
            format!("  编号（1-{}）或名称；n 下一页 | p 上一页 | f 过滤", filtered.len())
        } else {
            format!("  number (1-{}) or name; n next | p prev | f filter", filtered.len())
        };
        match common_input(ed, path, &line, in_browse, &help, page_hint) {
            Some(Pick::Sel(usize::MAX)) => {}
            Some(p) => return p,
            None => continue,
        }
        let l = line.to_lowercase();
        match l.as_str() {
            "n" => { if page + 1 < pages { page += 1; } else { println!("{}", t("  Last page", "  已是最后一页")); } }
            "p" => { if page > 0 { page -= 1; } else { println!("{}", t("  First page", "  已是第一页")); } }
            "f" => {
                hint_line(t("Filter substring (empty=clear)", "过滤子串（留空=清除）"));
                let nf = ask(ed, "filter > ", vec![]).unwrap_or_default();
                flt = nf.to_lowercase();
                filtered = (0..items.len()).filter(|&i| flt.is_empty() || items[i].to_lowercase().contains(&flt)).collect();
                page = 0;
                if filtered.is_empty() { println!("{}", t("  No matches (f to reset)", "  无匹配（f 重设）")); }
            }
            other => {
                if let Ok(n) = other.parse::<usize>() {
                    if n >= 1 && n <= filtered.len() { return Pick::Sel(filtered[n - 1]); }
                }
                let eq = filtered.iter().copied().find(|&i| items[i].to_lowercase() == other);
                if let Some(i) = eq { return Pick::Sel(i); }
                let pref: Vec<usize> = filtered.iter().copied().filter(|&i| items[i].to_lowercase().starts_with(other)).collect();
                if pref.len() == 1 { return Pick::Sel(pref[0]) }
                let contains: Vec<usize> = filtered.iter().copied().filter(|&i| items[i].to_lowercase().contains(other)).collect();
                if contains.len() == 1 { return Pick::Sel(contains[0]) }
                let hits: Vec<usize> = (0..items.len()).filter(|&i| items[i].to_lowercase().contains(other)).collect();
                if hits.is_empty() {
                    println!("{}", t("  no match, keeping filter", "  无匹配，保留当前过滤"));
                } else {
                    flt = other.to_string();
                    filtered = hits;
                    page = 0;
                    if is_zh() { println!("  过滤 [{other}]：{} 条", filtered.len()); }
                    else { println!("  Filtered [{other}]: {} matches", filtered.len()); }
                }
            }
        }
    }
}

fn multi_menu(ed: &mut Editor<CandHelper, rustyline::history::DefaultHistory>, path: &str, prompt: &str, items: &[String]) -> Option<Vec<usize>> {
    let mut flt = String::new();
    loop {
        // Re-display filtered list every iteration
        let idxs: Vec<usize> = (0..items.len())
            .filter(|&i| flt.is_empty() || items[i].to_lowercase().contains(&flt))
            .collect();
        if idxs.is_empty() {
            println!("{}", t("  No matches (type new filter, empty=clear)", "  无匹配（输入新过滤词，留空=清除）"));
            hint_line(t("type filter text | empty=clear | q back", "输入过滤词 | 留空=清除 | q 返回"));
            let line = ask(ed, &format!("[{path}] > "), vec![])?;
            if line.is_empty() { flt.clear(); continue; }
            if line.eq_ignore_ascii_case("q") { return None; }
            flt = line.to_lowercase();
            continue;
        }
        let shown: Vec<String> = idxs.iter().map(|&i| items[i].clone()).collect();
        if is_zh() {
            println!("\n{prompt}（匹配 {} 条，过滤：{}）", shown.len(), if flt.is_empty() { "无" } else { &flt });
        } else {
            println!("\n{prompt} ({} matches, filter: {})", shown.len(), if flt.is_empty() { "none" } else { &flt });
        }
        for (i, it) in shown.iter().enumerate() { println!("  {:>3}) {it}", i + 1); }
        hint_line(t("comma-sep nums (1,3,5) | all=select shown | type=filters | q back",
                     "逗号分隔（1,3,5）| all=全选当前 | 输入文字=过滤 | q 返回"));
        let line = ask(ed, &format!("[{path}] > "), shown.clone())?;
        if line.is_empty() { continue; }
        match line.to_lowercase().as_str() {
            "q" => return None,
            "all" | "a" => return Some(idxs.clone()),
            other => {
                let picks: Vec<Option<usize>> = other.split([',', ' ', '，']).filter(|s| !s.is_empty()).map(|p| parse_pick(p, shown.len())).collect();
                if !picks.is_empty() && picks.iter().all(|p| p.is_some()) {
                    let mut sel: Vec<usize> = picks.into_iter().map(|p| p.unwrap()).collect();
                    sel.sort_unstable(); sel.dedup();
                    return Some(sel.into_iter().map(|p| idxs[p]).collect());
                }
                // Not valid numbers → treat as filter text
                flt = other.to_string();
            }
        }
    }
}

// ==================== Session cache ====================

pub struct Session {
    groups: Option<Vec<GroupRec>>,
    hosts_all: Option<Vec<HostInfo>>,
    hosts_by_group: HashMap<String, Vec<HostInfo>>,
}

impl Session {
    fn new() -> Self { Session { groups: None, hosts_all: None, hosts_by_group: HashMap::new() } }

    async fn groups(&mut self) -> Option<Vec<GroupRec>> {
        if self.groups.is_none() {
            println!("{}", t("  Loading groups…", "  正在加载群组列表…"));
            let (_, client) = actions::connect().await.ok()?;
            let gs = client.get_groups().await.ok()?;
            client.logout().await;
            self.groups = Some(gs);
        }
        self.groups.clone()
    }

    async fn hosts(&mut self, group: Option<&str>) -> Option<Vec<HostInfo>> {
        match group {
            None => {
                if self.hosts_all.is_none() {
                    println!("{}", t("  Loading hosts (with OS)…", "  正在加载主机列表（含系统类型）…"));
                    let (cfg, client) = actions::connect().await.ok()?;
                    let hs = actions::hosts_with_os(&client, None, cfg.concurrency).await.ok()?;
                    client.logout().await;
                    self.hosts_all = Some(hs);
                }
                self.hosts_all.clone()
            }
            Some(g) => {
                if !self.hosts_by_group.contains_key(g) {
                    println!("{}", t("  Loading hosts…", "  正在加载主机列表…"));
                    let (cfg, client) = actions::connect().await.ok()?;
                    let hs = actions::hosts_with_os(&client, Some(g), cfg.concurrency).await.ok()?;
                    client.logout().await;
                    self.hosts_by_group.insert(g.to_string(), hs);
                }
                self.hosts_by_group.get(g).cloned()
            }
        }
    }
}

fn host_label(h: &HostInfo) -> String {
    format!("{} ({}) [{}]", h.host, h.ip, if h.os_family.is_empty() { "?" } else { &h.os_family })
}

// ==================== Entry ====================

pub async fn run(_cli: &crate::Cli) -> i32 {
    if std::env::var("ZBX_URL").is_err() {
        if let Err(e) = actions::ensure_config().await {
            eprintln!("zbxpatrol: {e}");
            return 2;
        }
    }
    println!("{}", t(
        "zbxpatrol interactive (num/letter select, q back, ? help, L lang, Tab complete, Ctrl+C quit)",
        "zbxpatrol 交互模式（数字/字母选择，q 返回，? 帮助，L 切换语言，Tab 补全，Ctrl+C 退出）"));
    let mut sess = Session::new();
    let mut ed = make_editor();
    loop {
        let main_path = t("main", "主菜单").to_string();
        let main_title = t("Main Menu", "主菜单").to_string();
        let items: Vec<String> = if is_zh() {
            vec!["浏览数据（群组→主机→指标）".into(), "生成巡检报表".into(), "连通性自检".into()]
        } else {
            vec!["Browse data (groups > hosts > items)".into(), "Generate report".into(), "Self-check".into()]
        };
        let refs: Vec<&str> = items.iter().map(|s| s.as_str()).collect();
        match menu(&mut ed, &main_path, &main_title, &refs, false) {
            Pick::Sel(0) => { if browse_flow(&mut sess, &mut ed).await == Pick::Exit { println!("{}", t("goodbye", "再见")); return 0; } }
            Pick::Sel(1) => { if report_flow(&mut sess, &mut ed).await == Pick::Exit { println!("{}", t("goodbye", "再见")); return 0; } }
            Pick::Sel(2) => { actions::do_check(crate::Format::Table).await; }
            Pick::LangChanged => { continue; } // re-loop to rebuild menu in new language
            _ => { println!("{}", t("goodbye", "再见")); return 0; }
        }
    }
}

// ==================== Browse: groups → hosts → items ====================

async fn browse_flow(sess: &mut Session, ed: &mut Editor<CandHelper, rustyline::history::DefaultHistory>) -> Pick {
    let browse_path = t("main>browse", "主菜单>浏览").to_string();
    'groups: loop {
        let Some(groups) = sess.groups().await else { return Pick::Back };
        let mut rows = vec![t("All hosts", "全部主机").to_string()];
        rows.extend(groups.iter().map(|g| g.name.clone()));
        let group = match paged_pick(ed, &browse_path, t("Select group", "选择群组"), &rows, true) {
            Pick::Sel(0) => None,
            Pick::Sel(i) => Some(groups[i - 1].name.clone()),
            Pick::Back | Pick::ToMain | Pick::Exit | Pick::LangChanged => return Pick::Back,
            Pick::ToGroups | Pick::ToHosts => continue 'groups,
        };
        let gname = group.clone().unwrap_or_else(|| t("All hosts", "全部主机").to_string());
        let gpath = format!("{browse_path}>{gname}");

        'hosts: loop {
            let Some(hosts) = sess.hosts(group.as_deref()).await else { break 'hosts };
            if hosts.is_empty() { println!("{}", t("  No hosts", "  无主机")); break 'hosts; }
            let title = if group.is_some() { format!("[{gname}] {}", t("hosts", "主机")) } else { t("All hosts", "全部主机").to_string() };
            let rows: Vec<String> = hosts.iter().map(host_label).collect();
            let host = match paged_pick(ed, &gpath, &title, &rows, true) {
                Pick::Sel(i) => hosts[i].host.clone(),
                Pick::Back | Pick::ToHosts => break 'hosts,
                Pick::ToGroups => continue 'groups,
                Pick::ToMain | Pick::Exit | Pick::LangChanged => return Pick::Back,
            };
            let hpath = format!("{gpath}>{host}");
            match host_menu(ed, &host, &hpath).await {
                Pick::Back | Pick::ToHosts => continue 'hosts,
                Pick::ToGroups => continue 'groups,
                _ => return Pick::Back,
            }
        }
    }
}

// ==================== Host menu: 2 items only ====================

async fn host_menu(ed: &mut Editor<CandHelper, rustyline::history::DefaultHistory>, host: &str, path: &str) -> Pick {
    loop {
        let title = if is_zh() { format!("主机 {host}") } else { format!("Host {host}") };
        let items: Vec<String> = if is_zh() {
            vec!["查看监控项（选中出趋势图）".into(), "快速巡检（日报，仅控制台）".into()]
        } else {
            vec!["View items (filter → select → chart)".into(), "Quick inspect (daily, console)".into()]
        };
        let refs: Vec<&str> = items.iter().map(|s| s.as_str()).collect();
        match menu(ed, path, &title, &refs, true) {
            Pick::Back | Pick::ToHosts => return Pick::Back,
            Pick::Exit => return Pick::Exit,
            Pick::ToGroups | Pick::ToMain => return Pick::ToGroups,
            Pick::LangChanged => { continue; } // rebuild menu in new language
            Pick::Sel(0) => {
                if items_view(ed, host, path).await == Pick::Exit { return Pick::Exit; }
            }
            Pick::Sel(1) => {
                actions::do_report(ReportParams {
                    scope: Scope::Hosts(vec![host.to_string()]),
                    time: TimeSpec::Period(Period::Day),
                    strictness: None,
                    all_items: false,
                    extra_keys: vec![],
                    data_json: None,
                    csv_out: None,
                    out: "./reports".into(),
                    patrol_config: None,
                    fmt: crate::Format::Table,
                    quiet: false,
                    console_only: true,
                    raw: false,
                }).await;
            }
            _ => {}
        }
    }
}

async fn items_view(ed: &mut Editor<CandHelper, rustyline::history::DefaultHistory>, host: &str, path: &str) -> Pick {
    println!("{}", t("  Loading items…", "  正在加载监控项…"));
    let Ok(rows) = actions::items_detail_data_opt(host, "", true).await else {
        println!("{}", t("  Load failed", "  加载失败"));
        return Pick::Back;
    };
    if rows.is_empty() { println!("{}", t("  No items", "  无监控项")); return Pick::Back; }
    let keys: Vec<String> = rows.iter().map(|r| format!("{} = {}", r.key, r.lastvalue)).collect();
    let title = if is_zh() { format!("{host} 监控项") } else { format!("{host} items") };
    loop {
        match paged_pick(ed, path, &title, &keys, false) {
            Pick::Sel(i) => {
                let key = rows[i].key.clone();
                let Some(time) = pick_time(ed, path).await else { continue };
                actions::do_chart(host.to_string(), "cpu".into(), Some(key), time, crate::Format::Table).await;
            }
            p => return p,
        }
    }
}

// ==================== Report flow ====================

async fn report_flow(sess: &mut Session, ed: &mut Editor<CandHelper, rustyline::history::DefaultHistory>) -> Pick {
    let path = t("main>report", "主菜单>报表").to_string();
    let Some(scope) = pick_scope(sess, ed, &path).await else { return Pick::Back };
    let Some(time) = pick_time(ed, &path).await else { return Pick::Back };

    let st_title = t("Scoring Strictness", "评分严格度").to_string();
    let st_items: Vec<String> = if is_zh() {
        vec!["标准（基准80%）".into(), "宽松（基准90%）".into(), "严格（基准70%）".into()]
    } else {
        vec!["Standard (80%)".into(), "Loose (90%)".into(), "Strict (70%)".into()]
    };
    let st_refs: Vec<&str> = st_items.iter().map(|s| s.as_str()).collect();
    let strictness = match menu(ed, &path, &st_title, &st_refs, false) {
        Pick::Sel(i) => ["standard", "loose", "strict"][i.min(2)].to_string(),
        Pick::LangChanged => return Pick::Back,
        _ => return Pick::Back,
    };

    // No extra-items step — all items go to the "All Items" sheet by default
    let extra_keys = vec![];

    let out_title = t("Output", "输出方式").to_string();
    let out_items: Vec<String> = if is_zh() {
        vec!["控制台+Excel".into(), "仅控制台".into(), "仅Excel".into(), "控制台+CSV".into(), "控制台+JSON".into()]
    } else {
        vec!["Console + Excel".into(), "Console only".into(), "Excel only".into(), "Console + CSV".into(), "Console + JSON".into()]
    };
    let out_refs: Vec<&str> = out_items.iter().map(|s| s.as_str()).collect();
    let (console_only, csv_out, data_json) = match menu(ed, &path, &out_title, &out_refs, false) {
        Pick::Sel(0) => (false, None, None),
        Pick::Sel(1) => (true, None, None),
        Pick::Sel(2) => (false, None, None),
        Pick::Sel(3) => {
            let p = ask(ed, "CSV > ", vec![]).unwrap_or_default();
            (true, Some(std::path::PathBuf::from(if p.is_empty() { "report.csv".to_string() } else { p })), None)
        }
        Pick::Sel(_) => {
            let p = ask(ed, "JSON > ", vec![]).unwrap_or_default();
            (true, None, Some(std::path::PathBuf::from(if p.is_empty() { "report.json".to_string() } else { p })))
        }
        _ => return Pick::Back,
    };

    actions::do_report(ReportParams {
        scope, time,
        strictness: Some(strictness),
        all_items: true,  // always include all-items sheet
        extra_keys,
        data_json, csv_out,
        out: "./reports".into(),
        patrol_config: None,
        fmt: crate::Format::Table,
        quiet: false,
        console_only,
        raw: false,
    }).await;

    // After report: loop back to scope selection (not main menu)
    let next_title = t("Next", "接下来").to_string();
    let next_items: Vec<String> = if is_zh() {
        vec!["再生成一份（重选范围）".into(), "返回主菜单".into()]
    } else {
        vec!["Generate another (change scope)".into(), "Back to main menu".into()]
    };
    let next_refs: Vec<&str> = next_items.iter().map(|s| s.as_str()).collect();
    match menu(ed, &path, &next_title, &next_refs, false) {
        Pick::Sel(0) => Box::pin(report_flow(sess, ed)).await, // back to scope
        _ => Pick::Back,
    }
}

// ==================== Scope / Time pickers ====================

async fn pick_scope(sess: &mut Session, ed: &mut Editor<CandHelper, rustyline::history::DefaultHistory>, path: &str) -> Option<Scope> {
    let title = t("Scope", "巡检范围").to_string();
    let items: Vec<String> = if is_zh() {
        vec!["全部主机".into(), "按群组（多选）".into(), "按主机（多选）".into()]
    } else {
        vec!["All hosts".into(), "By group (multi)".into(), "By host (multi)".into()]
    };
    let refs: Vec<&str> = items.iter().map(|s| s.as_str()).collect();
    match menu(ed, path, &title, &refs, false) {
        Pick::Sel(0) => Some(Scope::All),
        Pick::Sel(1) => {
            let groups = sess.groups().await?;
            let names: Vec<String> = groups.iter().map(|g| g.name.clone()).collect();
            hint_line(t("Group filter (empty=all, Tab)", "群组过滤（留空=全部，Tab）"));
            let f = ask(ed, "filter > ", names.clone()).unwrap_or_default().to_lowercase();
            let idxs: Vec<usize> = (0..names.len()).filter(|&i| f.is_empty() || names[i].to_lowercase().contains(&f)).collect();
            let shown: Vec<String> = idxs.iter().map(|&i| names[i].clone()).collect();
            let mt = if is_zh() { format!("选择群组（{} 条）", shown.len()) } else { format!("Select groups ({} matches)", shown.len()) };
            let picked = multi_menu(ed, path, &mt, &shown)?;
            Some(Scope::Groups(picked.into_iter().map(|p| names[idxs[p]].clone()).collect()))
        }
        Pick::Sel(_) => {
            let hosts = sess.hosts(None).await?;
            let names: Vec<String> = hosts.iter().map(host_label).collect();
            hint_line(t("Host/IP/OS filter (empty=all, Tab)", "主机/IP/系统过滤（留空=全部，Tab）"));
            let f = ask(ed, "filter > ", names.clone()).unwrap_or_default().to_lowercase();
            let idxs: Vec<usize> = (0..names.len()).filter(|&i| f.is_empty() || names[i].to_lowercase().contains(&f)).collect();
            let shown: Vec<String> = idxs.iter().map(|&i| names[i].clone()).collect();
            let mt = if is_zh() { format!("选择主机（{} 台）", shown.len()) } else { format!("Select hosts ({} matches)", shown.len()) };
            let picked = multi_menu(ed, path, &mt, &shown)?;
            Some(Scope::Hosts(picked.into_iter().map(|i| hosts[idxs[i]].host.clone()).collect()))
        }
        _ => None,
    }
}

async fn pick_time(ed: &mut Editor<CandHelper, rustyline::history::DefaultHistory>, path: &str) -> Option<TimeSpec> {
    loop {
        let title = t("Time Range", "时间范围").to_string();
        let items: Vec<String> = if is_zh() {
            vec!["日报（24小时）".into(), "周报（7天）".into(), "月报（30天）".into(), "年度（365天）".into(), "自定义区间".into()]
        } else {
            vec!["Daily (24h)".into(), "Weekly (7d)".into(), "Monthly (30d)".into(), "Yearly (365d)".into(), "Custom range".into()]
        };
        let refs: Vec<&str> = items.iter().map(|s| s.as_str()).collect();
        let p = match menu(ed, path, &title, &refs, false) { Pick::Sel(i) => i, _ => return None };
        let spec = match p {
            0 => TimeSpec::Period(Period::Day),
            1 => TimeSpec::Period(Period::Week),
            2 => TimeSpec::Period(Period::Month),
            3 => TimeSpec::Period(Period::Year),
            _ => {
                hint_line(t("YYYY-MM-DD[ HH:MM:SS]; empty end = now", "YYYY-MM-DD[ HH:MM:SS]；结束留空=现在"));
                let from = ask(ed, "from > ", vec![])?;
                let to = ask(ed, "to > ", vec![])?;
                TimeSpec::FromTo { from, to: if to.is_empty() { None } else { Some(to) } }
            }
        };
        let tz = std::env::var("ZBX_TZ").unwrap_or_else(|_| "Asia/Shanghai".into()).parse().unwrap_or(chrono_tz::Asia::Shanghai);
        match TimeRange::resolve(&spec, tz) {
            Ok(r) => { println!("  {}", r.fmt_human()); return Some(spec); }
            Err(e) => println!("  {e}"),
        }
    }
}
