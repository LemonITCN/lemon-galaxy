//! LemonGalaxy 本机 MCP 服务：把 AI 智能体的工具开放给这台电脑上的外部智能体（Claude Code、Codex、Cursor 等）。
//!
//! 工具执行、风险判定、档位、确认卡片全部复用 galaxy-agent（外部调用只是没有模型循环）；
//! 本 crate 只负责协议（新旧两代 MCP）、本机 HTTP 端点与 stdio 桥接。契约见 docs/mcp.md。

pub mod bridge;
pub mod server;
pub mod tools;

pub use bridge::{read_endpoint, remove_endpoint, run_stdio_bridge, write_endpoint, Endpoint};
pub use server::{start, ClientIdent, Host, McpServer, ServerConfig, LEGACY_VERSIONS, MODERN_VERSION};
