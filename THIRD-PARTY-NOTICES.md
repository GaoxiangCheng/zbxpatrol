# 第三方声明 / Third-party notices

## Zabbix（外部系统，本项目不包含其代码）

- Zabbix 是 Zabbix SIA 的产品，采用 **GNU Affero General Public License v3 (AGPL-3.0)** 授权，
  版权所有 © Zabbix SIA。官方源码：<https://github.com/zabbix/zabbix>
- 本项目（zbxpatrol）是**独立程序**，仅通过 Zabbix 公开的 **JSON-RPC API**（`api_jsonrpc.php`）
  与 Zabbix 服务端通信；本项目**不包含、不修改、不分发** Zabbix 的任何源码或二进制组件。
- 使用本工具不代表与 Zabbix SIA 有任何关联；"Zabbix" 是 Zabbix SIA 的注册商标。
- 需要 Zabbix 源码时，请从上述官方仓库获取对应版本（本项目面向 Zabbix 6.x–7.4，已在 7.4.14 验证）。

## 本项目 Rust 依赖

全部第三方 Rust crates 及其许可证清单见仓库根目录的 SBOM 文件
`SBOM/zbxpatrol.cdx.json`（CycloneDX 1.5 格式，由 cargo-cyclonedx 从 Cargo.lock 生成）。
主要依赖及许可证（MIT / Apache-2.0 双许可为主）：

| 组件 | 用途 | 许可证 |
|---|---|---|
| tokio / axum / hyper | 异步运行时与 HTTP 服务 | MIT |
| reqwest + rustls + ring | TLS 与 Zabbix API 通信 | MIT/Apache-2.0, ISC |
| rust_xlsxwriter | Excel 报表生成 | MIT/Apache-2.0 |
| clap | 命令行解析 | MIT/Apache-2.0 |
| serde / serde_json | 序列化 | MIT/Apache-2.0 |
| comfy-table | 控制台表格 | MIT |
| chrono / chrono-tz | 时间与时区 | MIT/Apache-2.0 |
| hickory-resolver | 纯 Rust DNS 解析 | MIT/Apache-2.0 |
| rustyline | 交互式向导行编辑 | MIT |

> 上述清单为摘要，权威清单以 SBOM 文件为准。

## 本项目对 Zabbix 的修改声明

本项目**未修改 Zabbix 服务端、Agent 或前端的任何源码**；所有交互均通过 Zabbix 官方公开 API 完成。
