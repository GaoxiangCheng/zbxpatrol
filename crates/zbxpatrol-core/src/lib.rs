//! zbxpatrol-core：Zabbix 巡检核心库。
//!
//! 分层：env(配置) / zabbix(API 客户端) / rules(指标规则) / discovery(发现)
//!       / metrics(聚合) / scoring(评分) / problems(告警) / pipeline(编排)。
//! 全部业务逻辑在此，界面壳（CLI/HTTP）只做参数解析与渲染。

pub mod env;
pub mod errors;
pub mod timerange;
pub mod types;
pub mod zabbix;
pub mod rules;
pub mod discovery;
pub mod metrics;
pub mod scoring;
pub mod patrol_config;
pub mod problems;
pub mod pipeline;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
