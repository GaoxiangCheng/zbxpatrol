# zbxpatrol 项目开发与交接文档

> 本文档面向后续开发者或 AI 维护者，涵盖架构、构建、测试、部署、关键设计决策与已知限制。

---

## 1. 项目概述

| 项 | 值 |
|---|---|
| 名称 | zbxpatrol |
| 版本 | （随 Release，见 Tags，文档不固定版本号） |
| 语言 | Rust (edition 2021) |
| 代码仓库 | https://github.com/GaoxiangCheng/zbxpatrol (GitHub)；内网 Gitea 作镜像 |
| 运行环境 | Linux x86_64/aarch64 (musl 静态)、macOS arm64 |
| 依赖 | 零运行时依赖（单一二进制） |

**功能**：对 Zabbix 监控的全部服务器自动巡检，产出风险评分报表（Excel/JSON/CSV），提供 HTTP API 供第三方调用，内置交互式向导。已停用的主机/监控项/触发器标记「停用」且不参与评分。

---

## 2. 架构

```
crates/zbxpatrol-core/          # 核心库（全部业务逻辑，可独立复用）
├── env.rs        环境变量、~/.zbxpatrol/config.env 路径
├── errors.rs     PatrolError（Config/MissingEnv/Auth/Network/Api）→ 退出码
├── timerange.rs  时间解析（period/last/from-to）、时区、文件名格式化
├── types.rs      全部数据模型（serde）—— ReportData/HostInspection/MetricStats 等
├── zabbix.rs     JSON-RPC 客户端：Bearer 认证、重试、限速、分批
├── rules.rs      指标识别（key 正则→类别），内置规则已校准真实环境
├── discovery.rs  范围解析、逐主机 item 归类、OS 类型推导
├── metrics.rs    纯函数算法：聚合/分桶/差分/回归/通配（全部有单测）
├── scoring.rs    三档严格度评分 + TOML 声明式自定义规则
├── patrol_config.rs  patrol.toml 解析
├── problems.rs   严重级别映射
└── pipeline.rs   编排：run_inspection / run_query / host_metric_series

crates/zbxpatrol-cli/           # 壳
├── main.rs       clap 定义、配置加载、退出码
├── actions.rs    子命令实现（CLI/向导/serve 共用）+ 补全脚本
├── lang.rs       语言（en 默认 / zh）
├── wizard.rs     交互向导（rustyline 补全、L 键切语言）
├── serve.rs      axum HTTP API
└── render/       渲染器
    ├── mod.rs       table/json/csv/火花线/ASCII 图
    ├── xlsx.rs      Excel 报表（rust_xlsxwriter）
    └── bilingual.rs 中英文映射表
```

---

## 3. 关键设计决策

### 3.1 核心与壳分离
- `zbxpatrol-core` 无 UI 依赖，未来运维平台可同进程直接依赖
- 跨进程则走 HTTP API（serve）

### 3.2 数据选择策略
- ≤1 天：`history.get`（原始数据，最准）
- >1 天：`trend.get`（小时聚合，加权平均 `Σ(avgᵢ·numᵢ)/Σnumᵢ`）
- 当前值：`item.get` 的 `lastvalue`（零额外开销）

### 3.3 DNS 解析
- 使用 hickory 纯 Rust 解析器（非 musl 内置）
- 原因：musl 静态二进制与部分内网 DNS 不兼容（已在 Rocky Linux 9.8 实测验证）
- 若极端环境仍报 `Name does not resolve`，兜底：`echo "<IP> <域名>" >> /etc/hosts`

### 3.4 中英文双语
- `lang.rs` 全局状态（AtomicU8），`t(en, zh)` 函数切换
- `--lang en`（默认）/ `--lang zh`，向导内按 `L` 键即时切换
- 核心层产出中文值，CLI 渲染层 `bilingual.rs` 翻译为英文
- 风险点描述（长句子）用关键词替换做部分翻译，非逐句翻译

### 3.5 构建优化
- `[profile.release]` 使用 `lto = "thin"` + `codegen-units = 16`
- Release 构建约 40 秒（全量 LTO + 1 unit 时约 90 秒）

---

## 4. 构建与部署

### 4.1 开发环境

```bash
cargo build                   # debug
cargo build --release         # release (~40s)
cargo test -p zbxpatrol-core # 27 个单元测试
cargo clippy --all-targets   # 零警告
```

### 4.2 三平台交叉编译

```bash
# macOS arm64（本机直接编译）
cargo build --release

# Linux x86_64 musl 静态
cargo zigbuild --release --target x86_64-unknown-linux-musl

# Linux aarch64 musl 静态
cargo zigbuild --release --target aarch64-unknown-linux-musl

# 依赖：cargo install cargo-zigbuild + rustup target add <target>
```

### 4.3 部署到服务器

```bash
# 原子替换（不中断运行中的进程）
cat target/x86_64-unknown-linux-musl/release/zbxpatrol | ssh root@<server> \
  'cat > /opt/src/zbxpatrol.new && chmod +x /opt/src/zbxpatrol.new && mv /opt/src/zbxpatrol.new /opt/src/zbxpatrol'

# 安装补全
./zbxpatrol completions bash > /etc/bash_completion.d/zbxpatrol
./zbxpatrol completions zsh > ~/.zfunc/_zbxpatrol
```

### 4.4 部署物（deploy/ 目录）

| 文件 | 用途 |
|---|---|
| `zbxpatrol-daily.service` | systemd 定时巡检 |
| `zbxpatrol-serve.service` | 常驻 HTTP API |
| `Dockerfile` | 容器构建 |
| `crontab.example` | cron 定时任务示例 |
| `package-upload.sh` | 构建+上传 Gitea Packages（内网分发可选；公开渠道用 GitHub Releases） |
| `push.sh` | 一键 commit+push |

### 4.5 配置文件

三层优先级（高→低）：
1. 进程环境变量（`ZBX_URL`/`ZBX_USER`/`ZBX_PASSWORD` 等）
2. `./.env`（当前目录）
3. `~/.zbxpatrol/config.env`（交互初始化自动生成，权限 600）

---

## 5. 测试

### 5.1 单元测试（24 个，离线）

```bash
cargo test -p zbxpatrol_core
```

覆盖：时间解析、时区换算、history 聚合、trend 加权、分桶与取反、计数器差分与回绕、重启检测、回归与满盘预测、三档阈值逐项对照、规则正则（真实环境 key 样本）、OS 推导、TOML 解析。

### 5.2 API 自测（23 项断言）

```bash
./zbxpatrol serve --listen 127.0.0.1:8787 --token <TOKEN> &
./deploy/api-selftest.sh http://127.0.0.1:8787 <TOKEN>
```

覆盖：health/groups/hosts/items/query/report 结构字段/risk/xlsx 魔数/save/400/401。

### 5.3 CLI 回归

主要命令 × 4 台测试主机（2 Windows + 2 Linux）× 各功能选项，约 60 项断言。

---

## 6. 发布（GitHub Releases）

```bash
# 构建三平台静态二进制（工具链：musl 交叉 / cargo-zigbuild + macOS SDK）
cargo zigbuild --release --target x86_64-unknown-linux-musl
cargo zigbuild --release --target aarch64-unknown-linux-musl
cargo zigbuild --release --target aarch64-apple-darwin   # 需 SDKROOT 指向 macOS SDK

# 在 GitHub Releases 页附到对应 tag（或用 gh CLI）
gh release create v<version> dist/* --title "v<version>" --notes "..."
```

内网 Gitea 分发（可选保留）：`GITEA_USER=<user> GITEA_PASS=<pass> ./deploy/package-upload.sh <version>`。

---
## 7. 已知限制与待改进

| 项目 | 状态 | 说明 |
|---|---|---|
| 风险点翻译 | 部分 | 长句子用关键词替换，非完整翻译 |
| JSON 字段名 | 英文 | 接口兼容性考虑，不支持语言切换 |
| 评分自定义 | 已实现 | patrol.toml [[scoring]]，见 docs/SCORING
| `--keys` | 已删除 | 功能不稳定，用 `--all-items` + `--raw` 替代 |
| 多 Zabbix 实例 | 未实现 | 核心层预留 DataSource trait 抽象 |
| Web UI | 未实现 | v2 路线图 |

---

## 8. 文件索引

```
├── Cargo.toml                    # workspace
├── patrol.example.toml           # 业务配置样例
├── README.md                     # 英文主页（默认）
├── README.zh.md                  # 中文主页
├── docs/
│   ├── README.md                 # 文档索引
│   ├── API.zh.md / API.en.md     # HTTP API 文档
│   ├── CLI.zh.md / CLI.en.md     # CLI 参考
│   ├── CODE.zh.md / CODE.en.md   # 架构与代码说明
│   ├── INTERACTIVE.zh.md / .en.md # 交互向导
│   ├── SCORING.zh.md / SCORING.en.md # 自定义评分指南
│   ├── HANDOFF.md                # 本文档
│   └── openapi.yaml              # OpenAPI 3.0 规范
├── deploy/                       # 部署物
├── dist/                         # 三平台二进制（不入 git）
└── crates/                       # 源码
```

---

## 9. 交接清单

- [ ] 读本文档 + README.md 了解架构
- [ ] `cargo test` 确认 28 个单测通过
- [ ] `./deploy/api-selftest.sh` 确认 API 正常
- [ ] 部署最新版到服务器并 `check` 通过
- [ ] 阅读 docs/SCORING.zh.md 了解评分自定义
- [ ] 阅读 docs/openapi.yaml 了解 API 契约
- [ ] 确认 GitHub Releases 发布权限（GaoxiangCheng/zbxpatrol）
- [ ] （可选，内网）确认 Gitea 镜像/Packages 上传权限
