# lilyco-bot

**Multi-platform IM Bot adapter for [Lyco-team/lilyco](https://github.com/Lyco-team/lilyco)** — define your command once with `lilyco-core` (`Registry` + `CommandSchema`), and run it on **Telegram**, **QQ (OneBot v11)**, or **WeChat (iLink)** without duplicating business logic or bloating the core SDK workspace.

## 为什么独立成仓？（Hub-and-Spoke 低心智负担）

* `lilyco` 主仓保持精简的 10 个核心 SDK crate（CLI / TUI / Web GUI / MCP），增量编译极快；
* `lilyco-bot` 作为独立主题仓，只通过 `lilyco-core` 接入同一套 `Registry` 与 `CommandSchema`，并用 Cargo `features` 隔离各 IM 平台依赖——**用哪个平台就只编译哪个 crate**。

## 已接入的热门 crates.io 生态

| Feature | 平台 | 底层 Crate | License | 说明 |
| :--- | :--- | :--- | :--- | :--- |
| `tg` *(默认)* | Telegram | [`teloxide`](https://crates.io/crates/teloxide) (`0.17`) | MIT | 自动 `setMyCommands`、Long Polling、原生支持 `TELOXIDE_PROXY` / `HTTPS_PROXY` |
| `qq` | QQ / 群聊 | [`onebot_v11`](https://crates.io/crates/onebot_v11) (`0.1`) | MIT/Apache-2.0 | 适配 NapCat / LLOneBot OneBot v11 协议 |
| `wx` | 微信 | [`weixin-agent`](https://crates.io/crates/weixin-agent) (`0.3`) | MIT | 纯协议层 WeChat iLink Bot SDK |

## 快速开始（Telegram Bot）

```bash
# 1. 运行内置单元测试（验证命令解析、类型转换、Schema 校验与中英文 /help）
cargo test

# 2. 启动 Telegram Bot 示例（支持自动读取系统 HTTPS_PROXY / TELOXIDE_PROXY）
TELOXIDE_TOKEN="你的BotToken" cargo run --example demo_tg
```

在 Telegram 中发送：
* `/help` — 自动按用户语言列出 `Registry` 中全部可见命令；
* `/echo text="hello lilyco" repeat=2` — 自动按 `CommandSchema` 做类型转换、边界校验并执行 `RegisteredCommand` handler。

## License

Dual-licensed under MIT OR Apache-2.0.
