# ut-client

基于 [kuma-client](https://crates.io/crates/kuma-client) + [clap](https://crates.io/crates/clap) 的 Rust CLI 工具，用于清理 Uptime Kuma 中的拨测任务（Monitor），支持**单条、多条、全部**清理。

## 构建与安装

需要 Rust 1.70+（`rustup` 安装：https://rustup.rs）。

```bash
# 编译 release 版本
cargo build --release

# 产物位置
./target/release/ut-client

# 可选：安装到 ~/.cargo/bin
cargo install --path .
```

## 连接配置

连接参数为全局选项，可放在子命令之前或之后；三种方式可混用（优先级：命令行 > 环境变量 > 交互输入）：

| 参数 | 环境变量 | 说明 |
| --- | --- | --- |
| `--url` | `KUMA_URL` | Uptime Kuma 服务地址，缺省 `http://localhost:3001` |
| `--username` | `KUMA_USERNAME` | 登录用户名 |
| `--password` | `KUMA_PASSWORD` | 登录密码；未提供且已给用户名时交互输入（不回显） |
| `--mfa-token` | `KUMA_MFA_TOKEN` | 2FA 动态验证码（账号开启两步验证时必填） |
| `--mfa-secret` | `KUMA_MFA_SECRET` | 2FA TOTP 密钥（由工具自动生成验证码，与 `--mfa-token` 二选一） |

推荐用环境变量，避免密码进入 shell 历史：

```bash
export KUMA_URL="http://your-kuma-host:3001"
export KUMA_USERNAME="admin"
export KUMA_PASSWORD="your-password"
```

## 使用

### 列出所有拨测任务

```bash
ut-client list
```

输出 ID 与名称对照表，用于确定要清理的目标。

### 清理拨测任务

```bash
# 单条清理
ut-client delete 5

# 多条清理
ut-client delete 5 8 12

# 全部清理
ut-client delete --all
```

说明：

- 默认会先列出待删除清单并要求交互确认（`y/N`），加 `-y` / `--yes` 跳过确认。
- 指定的 ID 不存在时会直接报错并提示先执行 `list`，不会部分执行。
- 删除按 ID 降序执行（子任务先删，避免分组引用导致失败）；`--all` 清理时失败项会自动重试一轮。
- 结束时汇总成功/失败明细；存在失败时退出码为 `1`，可直接用于脚本判断。
- **删除不可恢复**，`--all` 使用前建议先 `list` 核对。

### 帮助

```bash
ut-client --help
ut-client delete --help
```

## 故障排查

工具默认输出 `info` 级别日志（可用 `RUST_LOG` 调整，如 `RUST_LOG=debug`），
kuma-client 内部被吞掉的登录错误会以 `WARN` 行打印在报错信息上方。

报错 `It looks like the server is expecting a username/password, but none was provided`
表示 **socket 已连通但登录未完成**，看上方的 `WARN` 行确定真实原因：

| WARN 日志 | 原因 | 处理 |
| --- | --- | --- |
| `The server rejected the login: authIncorrectCreds` | 用户名或密码错误 | 核对凭据；注意不要照抄 README 里的占位符示例值 |
| `MFA enabled, but no token/secret was provided` | 账号开启了 2FA | 提供 `--mfa-token` 或 `--mfa-secret` |
| 无 WARN，直接超时 | 环境变量未生效 / 服务不可达 | 确认 `echo $KUMA_URL` 等有值；`curl http://<host>:3001/socket.io/?EIO=4&transport=polling` 验证连通 |

其他常见错误：

- `Invalid URL: ...`：`KUMA_URL` 格式不对，需带 `http://`。
- `Timeout while trying to connect`：服务不可达或端口不通。

## 退出码

| 退出码 | 含义 |
| --- | --- |
| `0` | 全部成功 |
| `1` | 连接失败 / 参数错误 / 存在删除失败项 |
| `2` | 命令行参数解析失败 |

## 项目结构

```
ut-client-rs/
├── Cargo.toml          # 依赖：kuma-client、clap、tokio、anyhow、rpassword
├── src/
│   └── main.rs         # CLI 定义 + 连接 / 列表 / 删除逻辑
└── target/release/
    └── ut-client       # 单文件可执行产物（约 4.8M，strip + lto）
```

## 实现要点

- 通过 `kuma-client` 的 Socket.IO 客户端连接 Uptime Kuma（`Client::connect`）。
- 任务列表来自 `get_monitors()`（`HashMap<String, Monitor>`），经 `monitor.common().id() / .name()` 提取。
- 删除走 `delete_monitor(id)`，全部成功后主动 `disconnect()`。

## 版本兼容补丁（vendor/kuma-client）

`kuma-client 2.0.0`（截至 2.1.0-rc.1 仍未修复）在反序列化新版 Uptime Kuma 返回的
`HttpBodyEncoding` 值（`JSON` / `FORM` / `XML`，大写）时会报
`unknown variant 'JSON'` 并导致 MonitorList 解析 panic。

本仓库通过 `[patch.crates-io]` 使用 `vendor/kuma-client` 本地副本修复（针对 Uptime Kuma 2.0.2 实测）：

1. **`HttpBodyEncoding` 大小写**：`src/models/monitor.rs` 中给枚举添加大写 serde 别名（`JSON` / `FORM` / `XML`），兼容新旧服务端，序列化输出仍为小写。
2. **新增 `smtp` 拨测类型**：Uptime Kuma 2.0.x 新增 `smtp` 类型，原版 kuma-client 不认识，解析即 panic。补丁新增 `MonitorSmtp` 结构体及 `MonitorType::Smtp` / `Monitor::Smtp` 变体。
3. **小数数值兜底**：`src/deserialize.rs` 的 `DeserializeNumberLenient` 原版遇小数值（如 `3.5`）直接报 `Unable to represent 3.5 as i32`；补丁改为四舍五入转整数，数值与字符串路径均兼容。

回归测试见 `vendor/kuma-client/tests/encoding_alias_test.rs`（4 个用例，含 smtp 反序列化）。

若后续官方版本修复，可直接移除 `[patch.crates-io]` 段落与 `vendor/` 目录。

## 跨平台构建（GitHub Actions）

仓库内置 `.github/workflows/release.yml`，覆盖 5 个目标：

| 目标 | 构建方式 | 产物 |
| --- | --- | --- |
| `x86_64-unknown-linux-gnu` | ubuntu-22.04 原生 | `ut-client-<tag>-x86_64-unknown-linux-gnu.tar.gz` |
| `aarch64-unknown-linux-gnu` | `cross` 容器交叉编译 | `ut-client-<tag>-aarch64-unknown-linux-gnu.tar.gz` |
| `x86_64-apple-darwin` | macos-13 原生 | `.tar.gz` |
| `aarch64-apple-darwin` | macos-14 原生（Apple Silicon） | `.tar.gz` |
| `x86_64-pc-windows-msvc` | windows-latest 原生 | `.zip` |

触发方式：

- **发版**：打 tag 推送即构建并自动挂到 GitHub Release：
  ```bash
  git tag v0.1.0 && git push origin v0.1.0
  ```
- **手动**：Actions 页面选择 Release 工作流 → Run workflow（产物在 workflow artifacts 里）。

注意事项：

- `vendor/`（kuma-client 本地补丁）**必须提交到仓库**，CI 依赖它，`.gitignore` 已放行。
- Linux 产物依赖 glibc（在 ubuntu-22.04 上构建，兼容性较好）；因 kuma-client 使用
  `native-tls`（需 OpenSSL），暂不提供 musl 静态链接版本。

## License

MIT
