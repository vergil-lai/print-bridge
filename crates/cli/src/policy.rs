/// 命令应当在线执行还是允许直接访问本地状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandPolicy {
    OnlineOnly,
    OnlinePreferred,
    OfflineAllowed,
}
