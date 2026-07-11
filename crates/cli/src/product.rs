use async_trait::async_trait;

use crate::{CommandError, CommandErrorKind, ProductKind};

/// 由具体产品实现的桌面集成能力。
#[async_trait]
pub trait ProductCommandAdapter: Send + Sync {
    /// 返回当前可执行文件的产品类型。
    fn product_kind(&self) -> ProductKind {
        ProductKind::Desktop
    }
    /// 返回当前是否启用开机自启动。
    async fn autostart_status(&self) -> Result<bool, CommandError>;

    /// 启用或禁用开机自启动。
    async fn set_autostart(&self, enabled: bool) -> Result<(), CommandError>;

    /// 设置桌面应用语言。
    async fn set_language(&self, language: &str) -> Result<(), CommandError>;
}

/// 对不支持桌面集成能力的产品返回稳定错误。
pub struct UnsupportedProductCommandAdapter {
    reason: &'static str,
    kind: ProductKind,
}

impl UnsupportedProductCommandAdapter {
    /// 创建不支持产品能力的适配器。
    pub fn new(reason: &'static str) -> Self {
        Self {
            reason,
            kind: ProductKind::Desktop,
        }
    }

    /// 创建 Headless 产品的不支持适配器。
    pub fn headless(reason: &'static str) -> Self {
        Self {
            reason,
            kind: ProductKind::Headless,
        }
    }

    fn error(&self) -> CommandError {
        CommandError::new(CommandErrorKind::Unsupported, self.reason)
    }
}

#[async_trait]
impl ProductCommandAdapter for UnsupportedProductCommandAdapter {
    fn product_kind(&self) -> ProductKind {
        self.kind
    }
    async fn autostart_status(&self) -> Result<bool, CommandError> {
        Err(self.error())
    }

    async fn set_autostart(&self, _enabled: bool) -> Result<(), CommandError> {
        Err(self.error())
    }

    async fn set_language(&self, _language: &str) -> Result<(), CommandError> {
        Err(self.error())
    }
}
