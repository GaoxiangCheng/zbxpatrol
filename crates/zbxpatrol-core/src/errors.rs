//! 统一错误与退出码：0 成功 / 2 配置或凭据 / 3 网络或 API / 4 部分数据缺失。

#[derive(Debug, thiserror::Error)]
pub enum PatrolError {
    /// 环境变量缺失、格式错误、密码错误等 —— 退出码 2
    #[error("配置错误：{0}")]
    Config(String),

    /// 必填环境变量缺失（可触发 TTY 交互式初始化）—— 退出码 2
    #[error("缺少环境变量 {0}（终端环境可直接运行 zbxpatrol 交互初始化，配置保存到 ~/.zbxpatrol/config.env）")]
    MissingEnv(String),

    /// 登录失败、会话失效且无法重登 —— 退出码 2
    #[error("认证失败：{0}")]
    Auth(String),

    /// 连不上、超时、HTTP 错误、Zabbix API 返回 error —— 退出码 3
    #[error("网络或 API 错误：{0}")]
    Network(String),

    /// Zabbix API 业务错误（参数/权限等）—— 退出码 3
    #[error("Zabbix API 错误：{0}")]
    Api(String),
}

impl PatrolError {
    pub fn exit_code(&self) -> i32 {
        match self {
            PatrolError::Config(_) | PatrolError::MissingEnv(_) | PatrolError::Auth(_) => 2,
            PatrolError::Network(_) | PatrolError::Api(_) => 3,
        }
    }

    /// 首次登录失败时区分「密码错」与「网络问题」
    pub fn from_login_failure(msg: String) -> Self {
        let m = msg.to_lowercase();
        if m.contains("name or password") || m.contains("login name or password") || m.contains("权限")
            || m.contains("permission") || m.contains("api access")
        {
            PatrolError::Auth(msg)
        } else {
            PatrolError::Network(msg)
        }
    }
}

pub type Result<T> = std::result::Result<T, PatrolError>;
