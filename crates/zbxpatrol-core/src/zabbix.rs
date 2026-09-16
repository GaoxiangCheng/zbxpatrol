//! Zabbix JSON-RPC 客户端：登录/登出、Bearer 认证（7.x）、限速、重试、itemids 分批。

use crate::env::AppConfig;
use crate::errors::{PatrolError, Result};
use crate::types::{GroupRec, HostInfo, ItemRec, ProblemRec};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};

const ITEM_CHUNK: usize = 100;

pub struct ZabbixClient {
    http: reqwest::Client,
    endpoint: String,
    user: String,
    password: String,
    token: RwLock<Option<String>>,
    id: AtomicU64,
    /// 请求间隔限速（令牌间隔）
    throttle: Mutex<std::time::Instant>,
    min_interval: Duration,
}

fn is_auth_error(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("not authorized")
        || m.contains("unauthorized")
        || m.contains("session terminated")
        || m.contains("not authorised")
        || m.contains("api access")
        || m.contains("login name or password")
}

/// 展开错误链（reqwest 顶层只报 "error sending request"，根因在 source 里）
fn err_chain(e: &(dyn std::error::Error + 'static)) -> String {
    let mut msg = e.to_string();
    let mut src = e.source();
    while let Some(s) = src {
        msg.push_str(&format!(" ← {s}"));
        src = s.source();
    }
    msg
}

impl ZabbixClient {
    pub fn new(cfg: &AppConfig) -> Result<Self> {
        let mut builder = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent(format!("zbxpatrol/{}", crate::VERSION));
        if cfg.insecure {
            builder = builder
                .danger_accept_invalid_certs(true)
                .danger_accept_invalid_hostnames(true);
        }
        let http = builder
            .build()
            .map_err(|e| PatrolError::Config(format!("HTTP 客户端构建失败：{e}")))?;
        let min_interval =
            Duration::from_nanos((1_000_000_000u64 / cfg.rate_limit_per_sec.max(1) as u64).max(1));
        Ok(ZabbixClient {
            http,
            endpoint: cfg.endpoint.clone(),
            user: cfg.user.clone(),
            password: cfg.password.clone(),
            token: RwLock::new(None),
            id: AtomicU64::new(1),
            throttle: Mutex::new(std::time::Instant::now() - Duration::from_secs(1)),
            min_interval,
        })
    }

    pub fn shared(cfg: &AppConfig) -> Result<Arc<Self>> {
        Ok(Arc::new(Self::new(cfg)?))
    }

    async fn throttle(&self) {
        let mut last = self.throttle.lock().await;
        let now = std::time::Instant::now();
        let elapsed = now.duration_since(*last);
        if elapsed < self.min_interval {
            tokio::time::sleep(self.min_interval - elapsed).await;
        }
        *last = std::time::Instant::now();
    }

    /// 单次逻辑调用：传输层重试（网络/5xx/429），不做认证重试（无递归）
    async fn raw_call(&self, method: &str, params: Value, token: Option<&str>) -> Result<Value> {
        let mut attempt = 0u32;
        loop {
            self.throttle().await;
            let body = json!({
                "jsonrpc": "2.0",
                "method": method,
                "params": params,
                "id": self.id.fetch_add(1, Ordering::Relaxed),
            });
            let mut req = self.http.post(&self.endpoint).json(&body);
            if let Some(t) = token {
                req = req.bearer_auth(t);
            }
            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp
                        .text()
                        .await
                        .map_err(|e| PatrolError::Network(e.to_string()))?;
                    if status.as_u16() >= 500 || status.as_u16() == 429 {
                        if attempt < 2 {
                            attempt += 1;
                            tokio::time::sleep(Duration::from_secs(attempt as u64)).await;
                            continue;
                        }
                        return Err(PatrolError::Network(format!("HTTP {status}：{text}")));
                    }
                    if !status.is_success() {
                        return Err(PatrolError::Network(format!("HTTP {status}：{text}")));
                    }
                    let v: Value = serde_json::from_str(&text)
                        .map_err(|e| PatrolError::Network(format!("响应不是合法 JSON：{e}")))?;
                    if let Some(err) = v.get("error") {
                        let msg = format!(
                            "{}: {}",
                            err.get("message").and_then(|m| m.as_str()).unwrap_or(""),
                            err.get("data").and_then(|d| d.as_str()).unwrap_or("")
                        );
                        return Err(PatrolError::Api(format!("{method}：{msg}")));
                    }
                    return Ok(v.get("result").cloned().unwrap_or(Value::Null));
                }
                Err(e) => {
                    if attempt < 2 {
                        attempt += 1;
                        tokio::time::sleep(Duration::from_secs(attempt as u64)).await;
                        continue;
                    }
                    let mut hint = if e.is_connect() {
                        "（无法连接：检查 ZBX_URL 与网络；若为证书问题可设 ZBX_INSECURE=true）"
                    } else {
                        ""
                    };
                    let chain = err_chain(&e);
                    if chain.contains("does not resolve") || chain.contains("dns error") {
                        hint = "（域名解析失败：若本机 curl 也解析失败请检查网络/DNS；若 curl 正常而本程序失败，可在 /etc/hosts 添加一条 \"<Zabbix服务器IP> <你的Zabbix域名>\" 固定解析后重试）";
                    }
                    return Err(PatrolError::Network(format!("{method} 请求失败：{chain}{hint}")));
                }
            }
        }
    }

    /// 带认证的调用；认证失效自动重登一次（递归点已装箱）
    async fn request(
        &self,
        method: &str,
        params: Value,
        token: Option<&str>,
        retry_on_auth: bool,
    ) -> Result<Value> {
        match self.raw_call(method, params.clone(), token).await {
            Ok(v) => Ok(v),
            Err(PatrolError::Api(msg)) if retry_on_auth && is_auth_error(&msg) => {
                tracing::warn!("会话失效，重新登录后重试");
                self.relogin().await?;
                let tok = self.token.read().await.clone();
                Box::pin(self.request(method, params, tok.as_deref(), false)).await
            }
            Err(e) => Err(e),
        }
    }

    async fn relogin(&self) -> Result<()> {
        self.login().await.map(|_| ())
    }

    // ---------- 会话 ----------

    /// 无需认证的版本探测
    pub async fn api_version(&self) -> Result<String> {
        let v = self.raw_call("apiinfo.version", json!({}), None).await?;
        v.as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| PatrolError::Api("apiinfo.version 返回格式异常".into()))
    }

    pub async fn login(&self) -> Result<String> {
        let params = json!({"username": self.user, "password": self.password});
        let v = self
            .raw_call("user.login", params, None)
            .await
            .map_err(|e| {
                PatrolError::from_login_failure(format!(
                    "登录失败：{e}（检查 ZBX_USER/ZBX_PASSWORD 与账号 API 访问权限）"
                ))
            })?;
        let tok = v
            .as_str()
            .ok_or_else(|| PatrolError::Auth("user.login 未返回 token".into()))?
            .to_string();
        *self.token.write().await = Some(tok.clone());
        tracing::debug!("zabbix 登录成功");
        Ok(tok)
    }

    pub async fn logout(&self) {
        let mut guard = self.token.write().await;
        if let Some(tok) = guard.take() {
            let _ = self.raw_call("user.logout", json!([]), Some(&tok)).await;
        }
    }

    /// 常规调用：带当前会话（未登录先登录，serve 常驻场景）
    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        if self.token.read().await.is_none() {
            self.login().await?;
        }
        let tok = self.token.read().await.clone().unwrap();
        self.request(method, params, Some(&tok), true).await
    }

    // ---------- 业务方法 ----------

    pub async fn get_groups(&self) -> Result<Vec<GroupRec>> {
        let v = self
            .call(
                "hostgroup.get",
                json!({"output": ["groupid","name"], "sortfield": "name", "limit": 500}),
            )
            .await?;
        Ok(parse_array(&v, |g| GroupRec {
            groupid: g["groupid"].as_str().unwrap_or_default().into(),
            name: g["name"].as_str().unwrap_or_default().into(),
        }))
    }

    pub async fn get_hosts(
        &self,
        groupids: Option<&[String]>,
        host_names: Option<&[String]>,
    ) -> Result<Vec<HostInfo>> {
        let mut params = json!({
            "output": ["hostid","host","name","status"],
            "selectGroups": ["name"],
            "selectInterfaces": ["ip","available"],
            "sortfield": "host",
            "limit": 1000,
        });
        if let Some(g) = groupids {
            params["groupids"] = json!(g);
        }
        if let Some(names) = host_names {
            params["filter"] = json!({ "host": names });
        }
        let v = self.call("host.get", params).await?;
        let mut hosts = parse_array(&v, |h| {
            let ip = h["interfaces"]
                .as_array()
                .and_then(|ifs| ifs.iter().find(|i| !i["ip"].as_str().unwrap_or("").is_empty()))
                .and_then(|i| i["ip"].as_str())
                .unwrap_or("")
                .to_string();
            HostInfo {
                hostid: h["hostid"].as_str().unwrap_or_default().into(),
                host: h["host"].as_str().unwrap_or_default().into(),
                name: h["name"].as_str().unwrap_or_default().into(),
                ip,
                os_family: String::new(),
                groups: h["groups"]
                    .as_array()
                    .map(|gs| {
                        gs.iter()
                            .filter_map(|g| g["name"].as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default(),
            }
        });
        // 兼容兜底：部分 Zabbix 7.4 环境 host.get selectGroups 不返回 groups（实测 extend 也不返回），
        // 此时用 hostgroup.get(selectHosts) 反查成员关系补齐
        if hosts.iter().any(|h| h.groups.is_empty()) {
            if let Ok(gv) = self.call(
                "hostgroup.get",
                json!({
                    "output": ["groupid","name"],
                    "selectHosts": ["hostid"],
                    "limit": 500,
                }),
            )
            .await
            {
                use std::collections::HashMap;
                let mut membership: HashMap<String, Vec<String>> = HashMap::new();
                for g in parse_array(&gv, |g| g.clone()) {
                    let gname = g["name"].as_str().unwrap_or_default().to_string();
                    if let Some(hs) = g["hosts"].as_array() {
                        for h in hs {
                            if let Some(hid) = h["hostid"].as_str() {
                                membership.entry(hid.to_string()).or_default().push(gname.clone());
                            }
                        }
                    }
                }
                for h in &mut hosts {
                    if h.groups.is_empty() {
                        if let Some(names) = membership.get(&h.hostid) {
                            h.groups = names.clone();
                        }
                    }
                }
            }
        }
        Ok(hosts)
    }

    pub async fn get_items(&self, hostid: &str) -> Result<Vec<ItemRec>> {
        let params = json!({
            "hostids": [hostid],
            "output": ["itemid","key_","name","value_type","units","lastvalue","lastclock","state"],
            "monitored": true,
            "sortfield": "key_",
            "limit": 5000,
        });
        let v = self.call("item.get", params).await?;
        Ok(parse_array(&v, |it| ItemRec {
            itemid: it["itemid"].as_str().unwrap_or_default().into(),
            hostid: hostid.into(),
            key: it["key_"].as_str().unwrap_or_default().into(),
            name: it["name"].as_str().unwrap_or_default().into(),
            value_type: it["value_type"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0),
            units: it["units"].as_str().unwrap_or_default().into(),
            lastvalue: it["lastvalue"].as_str().map(|s| s.to_string()),
            lastclock: it["lastclock"].as_str().and_then(|s| s.parse().ok()),
        }))
    }

    /// history.get：一次一批 itemid（≤100），同一 value_type（0 float / 3 unsigned）
    pub async fn history_get(
        &self,
        itemids: &[String],
        value_type: u8,
        from: i64,
        till: i64,
    ) -> Result<Vec<(String, i64, f64)>> {
        let mut out = Vec::new();
        for chunk in itemids.chunks(ITEM_CHUNK) {
            let params = json!({
                "output": "extend",
                "history": value_type,
                "itemids": chunk,
                "time_from": from,
                "time_till": till,
                "sortfield": "clock",
                "sortorder": "ASC",
                "limit": 100000,
            });
            let v = self.call("history.get", params).await?;
            if let Some(arr) = v.as_array() {
                for row in arr {
                    let itemid = row["itemid"].as_str().unwrap_or_default().to_string();
                    let clock: i64 = row["clock"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0);
                    let value: f64 = row["value"]
                        .as_str()
                        .and_then(|s| s.trim().parse().ok())
                        .unwrap_or(f64::NAN);
                    if value.is_finite() {
                        out.push((itemid, clock, value));
                    }
                }
            }
        }
        Ok(out)
    }

    /// trend.get：小时级 min/avg/max + 样本数
    pub async fn trend_get(&self, itemids: &[String], from: i64, till: i64) -> Result<Vec<TrendRow>> {
        let mut out = Vec::new();
        for chunk in itemids.chunks(ITEM_CHUNK) {
            let params = json!({
                "output": "extend",
                "itemids": chunk,
                "time_from": from,
                "time_till": till,
                "sortfield": "clock",
            });
            let v = self.call("trend.get", params).await?;
            if let Some(arr) = v.as_array() {
                for row in arr {
                    let g = |k: &str| -> f64 {
                        row[k].as_str().and_then(|s| s.trim().parse().ok()).unwrap_or(f64::NAN)
                    };
                    let num = g("num");
                    if num.is_finite() {
                        out.push(TrendRow {
                            itemid: row["itemid"].as_str().unwrap_or_default().to_string(),
                            clock: row["clock"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0),
                            num,
                            min: g("value_min"),
                            avg: g("value_avg"),
                            max: g("value_max"),
                        });
                    }
                }
            }
        }
        Ok(out)
    }

    /// 区间内发生过的 problem（按开始时间过滤，含已恢复）
    pub async fn problems_in_range(
        &self,
        from: i64,
        till: i64,
        hostids: Option<&[String]>,
    ) -> Result<Vec<ProblemRec>> {
        self.problems_query(Some(from), Some(till), hostids).await
    }

    /// 当前未恢复的全部 problem（不带时间过滤——开始于区间之前的活跃告警也要可见）
    pub async fn problems_open(&self, hostids: Option<&[String]>) -> Result<Vec<ProblemRec>> {
        self.problems_query(None, None, hostids).await
    }

    async fn problems_query(
        &self,
        from: Option<i64>,
        till: Option<i64>,
        hostids: Option<&[String]>,
    ) -> Result<Vec<ProblemRec>> {
        // Zabbix 7.x problem.get 无 selectHosts：先取问题，再经 trigger.get 映射主机
        let mut params = json!({
            "output": ["eventid","name","severity","clock","r_eventid","acknowledged","objectid"],
            "sortfield": ["eventid"],
            "sortorder": "DESC",
            "limit": 2000,
        });
        if let Some(f) = from {
            params["time_from"] = json!(f);
        }
        if let Some(t) = till {
            params["time_till"] = json!(t);
        }
        if let Some(h) = hostids {
            params["hostids"] = json!(h);
        }
        let v = self.call("problem.get", params).await?;
        let raw = parse_array(&v, |p| {
            let sev: u8 = p["severity"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0);
            (
                ProblemRec {
                    eventid: p["eventid"].as_str().unwrap_or_default().into(),
                    name: p["name"].as_str().unwrap_or_default().into(),
                    severity: sev,
                    severity_label: crate::problems::severity_label(sev).to_string(),
                    clock: p["clock"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0),
                    recovered: p["r_eventid"].as_str().map(|s| s != "0").unwrap_or(false),
                    acknowledged: p["acknowledged"].as_str().map(|s| s == "1").unwrap_or(false),
                    hosts: Vec::new(),
                },
                p["objectid"].as_str().unwrap_or_default().to_string(),
            )
        });
        let mut problems: Vec<ProblemRec> = raw.iter().map(|(p, _)| p.clone()).collect();
        // triggerid → 主机名
        let trig_ids: Vec<String> = {
            let mut ids: Vec<String> = raw.iter().map(|(_, t)| t.clone()).collect();
            ids.sort();
            ids.dedup();
            ids
        };
        if !trig_ids.is_empty() {
            let tv = self
                .call(
                    "trigger.get",
                    json!({
                        "triggerids": trig_ids,
                        "output": ["triggerid"],
                        "selectHosts": ["host"],
                        "limit": 2000,
                    }),
                )
                .await?;
            let mut tmap: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
            for t in parse_array(&tv, |t| t.clone()) {
                let hosts: Vec<String> = t["hosts"]
                    .as_array()
                    .map(|hs| {
                        hs.iter()
                            .filter_map(|h| h["host"].as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(tid) = t["triggerid"].as_str() {
                    tmap.insert(tid.to_string(), hosts);
                }
            }
            for (p, tid) in &raw {
                if let Some(hs) = tmap.get(tid) {
                    if let Some(target) = problems.iter_mut().find(|x| x.eventid == p.eventid) {
                        target.hosts = hs.clone();
                    }
                }
            }
        }
        Ok(problems)
    }
}

fn parse_array<T>(v: &Value, f: impl Fn(&Value) -> T) -> Vec<T> {
    v.as_array().map(|arr| arr.iter().map(f).collect()).unwrap_or_default()
}

#[derive(Debug, Clone)]
pub struct TrendRow {
    pub itemid: String,
    pub clock: i64,
    pub num: f64,
    pub min: f64,
    pub avg: f64,
    pub max: f64,
}
