pub mod dingtalk;
pub mod email;
pub mod feishu;
pub mod startup_guard;

// Re-export adapter types
pub use dingtalk::DingTalkAdapter;
pub use email::EmailAdapter;
pub use feishu::FeishuAdapter;