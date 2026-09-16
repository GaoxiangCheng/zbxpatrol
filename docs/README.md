# zbxpatrol Documentation

**English (default)** · [🇨🇳 简体中文快速入口 / Chinese quick entry](#中文文档) | [README (EN)](../README.md) · [README（中文）](../README.zh.md)

> Zabbix Server Inspection & Reporting · Single binary · Wizard + CLI + HTTP API

<a id="index"></a>
## English Documentation

| Topic | Link |
|---|---|
| HTTP API reference (+ machine-readable spec & self-test) | [API.en.md](API.en.md) |
| Interactive wizard guide | [INTERACTIVE.en.md](INTERACTIVE.en.md) |
| CLI reference | [CLI.en.md](CLI.en.md) |
| Custom scoring rules (patrol.toml) | [SCORING.en.md](SCORING.en.md) |
| Architecture & key code notes | [CODE.en.md](CODE.en.md) |
| OpenAPI 3.0 specification (machine-readable) | [openapi.yaml](openapi.yaml) |
| **Developer Handoff Guide** | [HANDOFF.md](HANDOFF.md) |

Quick start: repo root [README.md](../README.md).

<a id="中文文档"></a>
## 中文文档

| 主题 | 链接 |
|---|---|
| HTTP API 接口（含机器可读规范与自测） | [API.zh.md](API.zh.md) |
| 交互式向导使用说明 | [INTERACTIVE.zh.md](INTERACTIVE.zh.md) |
| 命令行（CLI）参考 | [CLI.zh.md](CLI.zh.md) |
| 自定义评分规则（patrol.toml） | [SCORING.zh.md](SCORING.zh.md) |
| 关键代码与架构说明 | [CODE.zh.md](CODE.zh.md) |
| **开发者交接文档** | [HANDOFF.md](HANDOFF.md) |

快速上手：仓库根目录 [README（中文）](../README.zh.md)。

## Conventions / 约定

- Angle-bracket tokens in examples (e.g. `<host>`, `<group>`) are **variables** — replace with real values (Tab completion helps). 示例中尖括号内容为**变量**，替换为实际值。
- All docs use multi-level anchors — TOC entries jump on Gitea. 文档使用多级锚点，可点击跳转。
- API consumers: import [openapi.yaml](openapi.yaml) into Swagger UI / Postman / code generators, and run `deploy/api-selftest.sh` against a live instance. API 使用方可导入 openapi.yaml，并运行自测脚本验证部署。
