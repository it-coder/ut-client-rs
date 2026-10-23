use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use kuma_client::{Client, Config, Url};
use std::io::{self, Write};

/// Uptime Kuma 拨测任务清理工具
///
/// 支持单条、多条、全部清理 Uptime Kuma 中的拨测任务（Monitor）。
#[derive(Parser, Debug)]
#[command(name = "ut-client", version, about, disable_help_subcommand = true)]
struct Cli {
    #[command(flatten)]
    conn: ConnArgs,

    #[command(subcommand)]
    command: Command,
}

/// 连接参数（也可通过环境变量 KUMA_URL / KUMA_USERNAME / KUMA_PASSWORD 提供）
#[derive(Args, Debug)]
struct ConnArgs {
    /// Uptime Kuma 服务地址，例如 http://localhost:3001
    #[arg(long, env = "KUMA_URL", global = true)]
    url: Option<String>,

    /// 登录用户名
    #[arg(long, env = "KUMA_USERNAME", global = true)]
    username: Option<String>,

    /// 登录密码（建议用环境变量 KUMA_PASSWORD 传入，避免出现在 shell 历史中）
    #[arg(long, env = "KUMA_PASSWORD", hide_env_values = true, global = true)]
    password: Option<String>,

    /// 2FA 动态验证码（账号开启两步验证时必填；或用 KUMA_MFA_SECRET 提供 TOTP 密钥）
    #[arg(long, env = "KUMA_MFA_TOKEN", global = true)]
    mfa_token: Option<String>,

    /// 2FA TOTP 密钥（由工具自动生成动态验证码，二选一）
    #[arg(long, env = "KUMA_MFA_SECRET", global = true)]
    mfa_secret: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// 列出所有拨测任务
    List,
    /// 清理拨测任务：传一个 ID 为单条，多个 ID 为多条，--all 为全部
    Delete {
        /// 要清理的任务 ID（可传多个，如: ut-client delete 5 8 12）
        ids: Vec<i32>,

        /// 清理全部任务（含分组任务）
        #[arg(long)]
        all: bool,

        /// 跳过确认，直接执行
        #[arg(short, long)]
        yes: bool,
    },
}

/// 本地缓存的拨测任务摘要
struct MonitorBrief {
    id: i32,
    name: String,
}

async fn connect(conn: &ConnArgs) -> Result<Client> {
    let url_str = conn
        .url
        .clone()
        .unwrap_or_else(|| "http://localhost:3001".to_string());

    let username = conn.username.clone();
    let password = match (&conn.password, &username) {
        (Some(p), _) => Some(p.clone()),
        (None, Some(_)) => {
            // 用户名已给但密码未给，交互式输入，避免回显
            eprint!("请输入密码: ");
            io::stderr().flush()?;
            Some(rpassword::read_password()?)
        }
        _ => None,
    };

    let client = Client::connect(Config {
        url: Url::parse(&url_str).with_context(|| format!("无效的 URL: {url_str}"))?,
        username,
        password,
        mfa_token: conn.mfa_token.clone(),
        mfa_secret: conn.mfa_secret.clone(),
        ..Default::default()
    })
    .await
    .with_context(|| {
        format!(
            "连接 Uptime Kuma 失败: {url_str}\
             \n提示: 若报 \"server is expecting a username/password\"，表示 socket 已连通但登录被服务端拒绝\
             （多为密码错误或账号开启了 2FA）。真实原因已由 kuma-client 以 warn 日志输出，\
             可用 RUST_LOG=info 重跑查看；开启 2FA 时需提供 --mfa-token 或 --mfa-secret。"
        )
    })?;

    Ok(client)
}

async fn fetch_monitors(client: &Client) -> Result<Vec<MonitorBrief>> {
    let monitors = client
        .get_monitors()
        .await
        .context("获取拨测任务列表失败")?;

    let mut list: Vec<MonitorBrief> = monitors
        .values()
        .filter_map(|m| {
            let common = m.common();
            let id = common.id().clone()?;
            let name = common.name().clone()?;
            Some(MonitorBrief { id, name })
        })
        .collect();
    list.sort_by_key(|m| m.id);
    Ok(list)
}

fn print_table(monitors: &[MonitorBrief]) {
    if monitors.is_empty() {
        println!("（当前没有任何拨测任务）");
        return;
    }
    println!("{:>6}  {}", "ID", "名称");
    println!("{:>6}  {}", "--", "----");
    for m in monitors {
        println!("{:>6}  {}", m.id, m.name);
    }
    println!("\n共 {} 条任务", monitors.len());
}

/// 交互式确认；`yes=true` 或输入为空时返回 true
fn confirm(prompt: &str, yes: bool) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    eprint!("{prompt} [y/N]: ");
    io::stderr().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let answer = line.trim().to_ascii_lowercase();
    Ok(answer.is_empty() && false || answer == "y" || answer == "yes")
}

async fn cmd_list(conn: &ConnArgs) -> Result<()> {
    let client = connect(conn).await?;
    let monitors = fetch_monitors(&client).await?;
    print_table(&monitors);
    client.disconnect().await.ok();
    Ok(())
}

async fn cmd_delete(conn: &ConnArgs, ids: Vec<i32>, all: bool, yes: bool) -> Result<()> {
    if !all && ids.is_empty() {
        bail!("未指定要清理的任务 ID。用法: ut-client delete <ID...> 或 ut-client delete --all");
    }
    if all && !ids.is_empty() {
        bail!("--all 不能与指定 ID 同时使用");
    }

    let client = connect(conn).await?;
    let monitors = fetch_monitors(&client).await?;

    // 解析待清理目标：单条 / 多条 / 全部
    let targets: Vec<MonitorBrief> = if all {
        monitors
    } else {
        let mut found = Vec::new();
        for &id in &ids {
            match monitors.iter().find(|m| m.id == id) {
                Some(m) => found.push(MonitorBrief {
                    id: m.id,
                    name: m.name.clone(),
                }),
                None => bail!("ID {id} 不存在，请先执行 `ut-client list` 查看"),
            }
        }
        found
    };

    if targets.is_empty() {
        println!("当前没有任何拨测任务，无需清理。");
        return Ok(());
    }

    println!("将要清理以下 {} 条拨测任务:", targets.len());
    print_table(&targets);
    println!();
    if !confirm("确认清理？此操作不可恢复", yes)? {
        println!("已取消。");
        return Ok(());
    }

    // 按 ID 降序删除：子任务（通常 ID 更大）先删，避免分组引用导致失败
    let mut to_delete: Vec<i32> = targets.iter().map(|m| m.id).collect();
    to_delete.sort_unstable_by(|a, b| b.cmp(a));

    let total = to_delete.len();
    let mut deleted: Vec<i32> = Vec::new();
    let mut failed: Vec<(i32, String)> = Vec::new();

    for (i, id) in to_delete.iter().enumerate() {
        let name = targets
            .iter()
            .find(|m| m.id == *id)
            .map(|m| m.name.clone())
            .unwrap_or_default();
        match client.delete_monitor(*id).await {
            Ok(_) => {
                deleted.push(*id);
                println!("[{}/{}] 已清理 ID {} ({})", i + 1, total, id, name);
            }
            Err(e) => {
                failed.push((*id, e.to_string()));
                eprintln!("[{}/{}] 清理失败 ID {} ({}): {}", i + 1, total, id, name, e);
            }
        }
    }

    // 全量清理时，失败的（多半是分组先于子任务被删导致）重试一轮
    if !failed.is_empty() {
        eprintln!("\n{} 条失败，重试一轮…", failed.len());
        let retry = std::mem::take(&mut failed);
        for (i, (id, _)) in retry.iter().enumerate() {
            let name = targets
                .iter()
                .find(|m| m.id == *id)
                .map(|m| m.name.clone())
                .unwrap_or_default();
            match client.delete_monitor(*id).await {
                Ok(_) => {
                    deleted.push(*id);
                    println!("[重试 {}/{}] 已清理 ID {} ({})", i + 1, retry.len(), id, name);
                }
                Err(e) => failed.push((*id, e.to_string())),
            }
        }
    }

    println!();
    if failed.is_empty() {
        println!("完成：成功清理 {} 条任务。", deleted.len());
    } else {
        println!(
            "完成：成功 {} 条，失败 {} 条。失败明细:",
            deleted.len(),
            failed.len()
        );
        for (id, err) in &failed {
            println!("  - ID {id}: {err}");
        }
        std::process::exit(1);
    }

    client.disconnect().await.ok();
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志：让 kuma-client 内部被吞掉的登录错误等 warn 信息可见
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cli = Cli::parse();
    match cli.command {
        Command::List => cmd_list(&cli.conn).await,
        Command::Delete { ids, all, yes } => cmd_delete(&cli.conn, ids, all, yes).await,
    }
}
