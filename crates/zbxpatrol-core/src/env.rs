//! 连接配置：只从环境变量读取（CLI 壳负责 .env 加载后再取 env）。

use crate::errors::{PatrolError, Result};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub url: String,
    pub user: String,
    pub password: String,
    pub timeout: Duration,
    pub insecure: bool,
    pub tz_name: String,
    pub concurrency: usize,
    pub rate_limit_per_sec: u32,
    /// 完整 JSON-RPC 端点（url + /api_jsonrpc.php）
    pub endpoint: String,
}

fn env_str(key: &str) -> Result<String> {
    std::env::var(key).map_err(|_| PatrolError::MissingEnv(key.to_string()))
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        let url = env_str("ZBX_URL")?;
        let user = env_str("ZBX_USER")?;
        let password = env_str("ZBX_PASSWORD")?;
        let timeout = std::env::var("ZBX_TIMEOUT")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(30);
        let insecure = std::env::var("ZBX_INSECURE")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);
        let tz_name = std::env::var("ZBX_TZ").unwrap_or_else(|_| "Asia/Shanghai".to_string());
        let concurrency = std::env::var("PATROL_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v >= 1 && *v <= 64)
            .unwrap_or(8);
        let rate_limit_per_sec = std::env::var("PATROL_RATE_LIMIT")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|v| *v >= 1)
            .unwrap_or(10);

        let base = url.trim_end_matches('/');
        if !base.starts_with("http") {
            return Err(PatrolError::Config(format!("ZBX_URL 必须以 http(s):// 开头：{base}")));
        }
        Ok(AppConfig {
            endpoint: format!("{base}/api_jsonrpc.php"),
            url: base.to_string(),
            user,
            password,
            timeout: Duration::from_secs(timeout),
            insecure,
            tz_name,
            concurrency,
            rate_limit_per_sec,
        })
    }

    pub fn tz(&self) -> chrono_tz::Tz {
        self.tz_name.parse().unwrap_or(chrono_tz::Asia::Shanghai)
    }
}

/// 家目录配置文件：~/.zbxpatrol/config.env（交互式初始化写入）
pub fn home_config_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| {
        let mut p = std::path::PathBuf::from(h);
        p.push(".zbxpatrol");
        p.push("config.env");
        p
    })
}
