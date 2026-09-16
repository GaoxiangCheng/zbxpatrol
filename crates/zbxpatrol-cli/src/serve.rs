//! 本地 HTTP API（axum）：供第三方程序调用。
//! GET /health /groups /hosts /items   POST /query /report（?format=xlsx 返回文件流）

use crate::render;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::StreamExt;
use serde::Deserialize;
use std::sync::Arc;
use zbxpatrol_core::env::AppConfig;
use zbxpatrol_core::errors::PatrolError;
use zbxpatrol_core::patrol_config::PatrolToml;
use zbxpatrol_core::pipeline::{self, InspectOptions};
use zbxpatrol_core::timerange::{Period, TimeRange, TimeSpec};
use zbxpatrol_core::types::Scope;
use zbxpatrol_core::zabbix::ZabbixClient;

pub struct AppState {
    pub cfg: AppConfig,
    pub client: Arc<ZabbixClient>,
    pub token: Option<String>,
    pub patrol: PatrolToml,
}

fn ok_json<T: serde::Serialize>(t: &T) -> Response {
    (StatusCode::OK, Json(serde_json::json!({ "ok": true, "data": t }))).into_response()
}

fn err_resp(status: u16, code: u16, msg: String) -> Response {
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        Json(serde_json::json!({ "ok": false, "error": { "code": code, "message": msg } })),
    )
        .into_response()
}

fn map_err(e: PatrolError) -> Response {
    match &e {
        PatrolError::Config(_) | PatrolError::MissingEnv(_) | PatrolError::Auth(_) => {
            err_resp(400, 2, e.to_string())
        }
        PatrolError::Network(_) | PatrolError::Api(_) => err_resp(502, 3, e.to_string()),
    }
}

// ---------- 请求体 ----------

/// 嵌套时间对象（与 README/openapi 文档一致）；与平铺 period/last/from/to 不可同时提供
#[derive(Deserialize, Default, Clone)]
struct TimeBody {
    period: Option<String>,
    last: Option<String>,
    from: Option<String>,
    to: Option<String>,
}

impl TimeBody {
    fn into_spec(self) -> Result<TimeSpec, PatrolError> {
        if self.from.is_some() {
            return Ok(TimeSpec::FromTo { from: self.from.clone().unwrap(), to: self.to.clone() });
        }
        if let Some(l) = &self.last {
            return Ok(TimeSpec::Last(l.clone()));
        }
        match self.period.as_deref() {
            None => Err(PatrolError::Config(
                "time 对象为空（需 period/last/from+to 至少一项）".into(),
            )),
            Some(p) => Ok(TimeSpec::Period(match p {
                "day" => Period::Day,
                "week" => Period::Week,
                "month" => Period::Month,
                "year" => Period::Year,
                other => {
                    return Err(PatrolError::Config(format!(
                        "未知 period {other:?}（支持 day|week|month|year）"
                    )))
                }
            })),
        }
    }
}

#[derive(Deserialize, Default)]
struct ApiReq {
    /// 群组名
    group: Option<String>,
    /// 主机名列表
    hosts: Option<Vec<String>>,
    /// 嵌套时间对象：{"time":{"last":"7d"}}（与 openapi 一致）
    time: Option<TimeBody>,
    /// 时间（平铺写法）：period=day|week|month|year / last="48h" / from+to
    period: Option<String>,
    last: Option<String>,
    from: Option<String>,
    to: Option<String>,
    /// /query：key 列表（支持通配）
    keys: Option<Vec<String>>,
    /// /report：附加全量指标
    all_items: Option<bool>,
    /// /report：评分严格度 loose|standard|strict
    strictness: Option<String>,
    /// /report: export raw history samples
    raw: Option<bool>,
}

impl ApiReq {
    /// hosts（更精确）优先于 group，与 CLI 的 --host/--hosts > --group 一致
    fn scope(&self) -> Scope {
        if let Some(h) = &self.hosts {
            Scope::Hosts(h.clone())
        } else if let Some(g) = &self.group {
            Scope::Groups(vec![g.clone()])
        } else {
            Scope::All
        }
    }
    fn spec(&self) -> Result<TimeSpec, PatrolError> {
        if let Some(t) = &self.time {
            if self.period.is_some() || self.last.is_some() || self.from.is_some() || self.to.is_some() {
                return Err(PatrolError::Config(
                    "time 对象与平铺 period/last/from/to 不可同时提供".into(),
                ));
            }
            return t.clone().into_spec();
        }
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
            Some(o) => return Err(PatrolError::Config(format!("未知 period {o:?}"))),
        };
        Ok(TimeSpec::Period(p))
    }
}

#[derive(Deserialize)]
struct HostsParams {
    group: Option<String>,
}

#[derive(Deserialize)]
struct ItemsParams {
    host: Option<String>,
    group: Option<String>,
    search: Option<String>,
}

// ---------- handlers ----------

async fn not_found() -> Response {
    err_resp(404, 0, "未知端点（可用：/health /groups /hosts /items /query /report）".into())
}

/// 统一 JSON 解析失败响应（替代 axum 默认纯文本）
fn bad_json(e: axum::extract::rejection::JsonRejection) -> Response {
    err_resp(400, 2, format!("请求体 JSON 非法：{e}"))
}

async fn health(State(st): State<Arc<AppState>>) -> Response {
    match st.client.api_version().await {
        Ok(v) => ok_json(&serde_json::json!({
            "status": "up",
            "zabbix": st.cfg.url,
            "zabbix_version": v,
            "version": zbxpatrol_core::VERSION,
        })),
        Err(_) => {
            // 可能未登录或会话过期：重登后再试
            match st.client.login().await {
                Ok(_) => match st.client.api_version().await {
                    Ok(v) => ok_json(&serde_json::json!({
                        "status": "up",
                        "zabbix": st.cfg.url,
                        "zabbix_version": v,
                        "version": zbxpatrol_core::VERSION,
                    })),
                    Err(e) => err_resp(502, 3, format!("zabbix 不可达：{e}")),
                },
                Err(e) => err_resp(502, 3, format!("登录失败：{e}")),
            }
        }
    }
}

async fn groups(State(st): State<Arc<AppState>>) -> Response {
    match st.client.get_groups().await {
        Ok(gs) => ok_json(&gs),
        Err(e) => map_err(e),
    }
}

async fn hosts(State(st): State<Arc<AppState>>, Query(p): Query<HostsParams>) -> Response {
    // 复用 CLI 逻辑：填充 OS 类型（Linux/Windows/…）
    match crate::actions::hosts_with_os(&st.client, p.group.as_deref(), st.cfg.concurrency).await {
        Ok(hs) => ok_json(&hs),
        Err(e) => map_err(e),
    }
}

async fn items(State(st): State<Arc<AppState>>, Query(p): Query<ItemsParams>) -> Response {
    let scope = if let Some(h) = &p.host {
        Scope::Hosts(vec![h.clone()])
    } else if let Some(g) = &p.group {
        Scope::Groups(vec![g.clone()])
    } else {
        Scope::All
    };
    let hosts = match zbxpatrol_core::discovery::resolve_hosts(&st.client, &scope).await {
        Ok(h) => h,
        Err(e) => return map_err(e),
    };
    let sem = Arc::new(tokio::sync::Semaphore::new(st.cfg.concurrency));
    let mut jobs = Vec::new();
    for h in hosts {
        let client = st.client.clone();
        let sem = sem.clone();
        jobs.push(async move {
            let _p = sem.acquire_owned().await.unwrap();
            let items = client.get_items(&h.hostid).await?;
            Ok::<_, PatrolError>(items)
        });
    }
    let mut all: Vec<zbxpatrol_core::types::ItemRec> = Vec::new();
    let mut stream = futures::stream::iter(jobs).buffer_unordered(st.cfg.concurrency);
    while let Some(r) = stream.next().await {
        match r {
            Ok(items) => all.extend(items),
            Err(e) => return map_err(e),
        }
    }
    // 按 key 聚合
    use std::collections::BTreeMap;
    let mut agg: BTreeMap<String, (String, String, u8, usize)> = BTreeMap::new();
    for it in &all {
        let e = agg
            .entry(it.key.clone())
            .or_insert((it.name.clone(), it.units.clone(), it.value_type, 0));
        e.3 += 1;
    }
    let search = p.search.as_deref().map(|s| s.to_lowercase());
    let rows: Vec<serde_json::Value> = agg
        .into_iter()
        .filter(|(k, (name, _, _, _))| match &search {
            Some(s) => k.to_lowercase().contains(s) || name.to_lowercase().contains(s),
            None => true,
        })
        .map(|(k, (name, unit, vt, cnt))| {
            serde_json::json!({"key": k, "name": name, "unit": unit, "value_type": vt, "hosts": cnt})
        })
        .collect();
    ok_json(&rows)
}

async fn query(
    State(st): State<Arc<AppState>>,
    body: Result<Json<ApiReq>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Json(req) = match body {
        Ok(b) => b,
        Err(e) => return bad_json(e),
    };
    let keys = match req.keys.clone() {
        Some(k) if !k.is_empty() => k,
        _ => return err_resp(400, 2, "keys 不能为空（示例 {\"keys\":[\"system.cpu.util\"]}）".into()),
    };
    let spec = match req.spec() {
        Ok(s) => s,
        Err(e) => return err_resp(400, 2, e.to_string()),
    };
    let range = match TimeRange::resolve(&spec, st.cfg.tz()) {
        Ok(r) => r,
        Err(e) => return err_resp(400, 2, e.to_string()),
    };
    let scope = req.scope();
    match pipeline::run_query(st.client.clone(), &scope, &keys, &range, st.cfg.concurrency).await {
        Ok(rows) => ok_json(&rows),
        Err(e) => map_err(e),
    }
}

async fn report(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<std::collections::HashMap<String, String>>,
    body: Result<Json<ApiReq>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Json(req) = match body {
        Ok(b) => b,
        Err(e) => return bad_json(e),
    };
    let spec = match req.spec() {
        Ok(s) => s,
        Err(e) => return err_resp(400, 2, e.to_string()),
    };
    let range = match TimeRange::resolve(&spec, st.cfg.tz()) {
        Ok(r) => r,
        Err(e) => return err_resp(400, 2, e.to_string()),
    };
    let scope = req.scope();
    let mode = match req.strictness.as_deref().map(zbxpatrol_core::scoring::Strictness::parse) {
        Some(None) => return err_resp(400, 2, "strictness 仅支持 loose|standard|strict".into()),
        Some(Some(m)) => m,
        None => zbxpatrol_core::scoring::Strictness::Standard,
    };
    let opts = InspectOptions {
        all_items: req.all_items.unwrap_or(false),
        patrol: st.patrol.clone(),
        strictness: mode,
        charts: true,
        extra_keys: req.keys.clone().unwrap_or_default(),
        raw: req.raw.unwrap_or(false),
    };
    match pipeline::run_inspection(st.client.clone(), &scope, &range, &opts, st.cfg.concurrency, None).await {
        Ok(outcome) => {
            let fmt = params
                .get("format")
                .map(|v| v.to_lowercase())
                .or_else(|| headers.get("x-format").and_then(|v| v.to_str().ok()).map(|v| v.to_lowercase()))
                .unwrap_or_else(|| "json".into());
            let want_save = params.get("save").map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false);
            // 服务器端落盘：save=1 时在 serve 工作目录 reports/ 生成文件并返回路径
            if want_save {
                let data = &outcome.data;
                if let Err(e) = std::fs::create_dir_all("reports") {
                    return err_resp(500, 3, format!("创建 reports/ 失败：{e}"));
                }
                let safe_scope = data
                    .scope_names
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "all".into())
                    .chars()
                    .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' })
                    .collect::<String>();
                let base = format!("巡检报告_{}_{}", data.range.from, data.range.till);
                let saved: std::io::Result<String> = (|| async {
                    match fmt.as_str() {
                        "xlsx" => {
                            let path = format!("reports/{base}_{safe_scope}.xlsx");
                            render::xlsx::render_report(data, std::path::Path::new(&path))
                                .map_err(|e| std::io::Error::other(e.to_string()))?;
                            Ok(path)
                        }
                        "csv" => {
                            let path = format!("reports/{base}_{safe_scope}.csv");
                            render::report_csv_file(data, std::path::Path::new(&path))?;
                            Ok(path)
                        }
                        _ => {
                            let path = format!("reports/{base}_{safe_scope}.json");
                            std::fs::write(&path, serde_json::to_string_pretty(data).unwrap_or_default())?;
                            Ok(path)
                        }
                    }
                })()
                .await;
                return match saved {
                    Ok(path) => ok_json(&serde_json::json!({
                        "saved": true, "file": path, "format": fmt,
                        "summary": data.summary,
                    })),
                    Err(e) => err_resp(500, 3, format!("保存失败：{e}")),
                };
            }
            if fmt == "xlsx" {
                match render::xlsx::render_report_buffer(&outcome.data) {
                    Ok(bytes) => (
                        StatusCode::OK,
                        [
                            (header::CONTENT_TYPE, "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
                            (header::CONTENT_DISPOSITION, "attachment; filename=\"patrol-report.xlsx\""),
                        ],
                        bytes,
                    )
                        .into_response(),
                    Err(e) => err_resp(500, 3, format!("xlsx 生成失败：{e}")),
                }
            } else if fmt == "csv" {
                use axum::body::Body;
                let mut body = render::report_csv_string(&outcome.data);
                body.insert(0, '\u{FEFF}');
                (
                    StatusCode::OK,
                    [
                        (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
                        (header::CONTENT_DISPOSITION, "attachment; filename=\"patrol-report.csv\""),
                    ],
                    Body::from(body),
                )
                    .into_response()
            } else {
                ok_json(&outcome.data)
            }
        }
        Err(e) => map_err(e),
    }
}

// ---------- 鉴权中间件 ----------

async fn auth_mw(
    State(st): State<Arc<AppState>>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let path = req.uri().path();
    if path == "/health" || st.token.is_none() {
        return next.run(req).await;
    }
    let expect = format!("Bearer {}", st.token.clone().unwrap_or_default());
    let ok = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|v| v == expect)
        .unwrap_or(false);
    if ok {
        next.run(req).await
    } else {
        err_resp(401, 401, "未授权：需要 Authorization: Bearer <token>".into())
    }
}

// ---------- 入口 ----------

pub async fn run(listen: &str, token: Option<String>) -> i32 {
    let cfg = match AppConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("zbxpatrol: {e}");
            return 2;
        }
    };
    let client = match ZabbixClient::shared(&cfg) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("zbxpatrol: {e}");
            return 2;
        }
    };
    if let Err(e) = client.login().await {
        eprintln!("zbxpatrol: {e}");
        return 2;
    }
    // 业务配置：自动加载当前目录 patrol.toml（可选）
    let patrol = std::fs::read_to_string("patrol.toml")
        .ok()
        .and_then(|s| PatrolToml::load_str(&s).ok())
        .unwrap_or_default();
    // 安全约束：未设 token 时拒绝绑定非回环地址（所有端点免鉴权，不能暴露到外部）
    if token.is_none() {
        let host = listen.rsplit_once(':').map(|(h, _)| h).unwrap_or(listen);
        let external = match host.parse::<std::net::IpAddr>() {
            Ok(ip) => !ip.is_loopback(),
            Err(_) => host != "localhost", // 主机名非 localhost 视为对外
        };
        if external {
            eprintln!(
                "zbxpatrol: 拒绝启动：未设置 --token 时仅允许绑定回环地址（当前 {listen}）。对外暴露必须配置 --token。"
            );
            return 2;
        }
    }
    let state = Arc::new(AppState { cfg: cfg.clone(), client, token, patrol });
    let app = Router::new()
        .route("/health", get(health))
        .route("/groups", get(groups))
        .route("/hosts", get(hosts))
        .route("/items", get(items))
        .route("/query", post(query))
        .route("/report", post(report))
        .fallback(not_found)
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_mw))
        .with_state(state);
    let listener = match tokio::net::TcpListener::bind(listen).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("zbxpatrol: 监听 {listen} 失败：{e}");
            return 2;
        }
    };
    tracing::info!("HTTP API 已启动：http://{listen}（/health /groups /hosts /items /query /report）");
    if let Err(e) = axum::serve(listener, app).await {
        eprintln!("zbxpatrol: {e}");
        return 3;
    }
    0
}
