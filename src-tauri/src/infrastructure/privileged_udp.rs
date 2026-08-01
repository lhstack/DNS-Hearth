//! Persistent privileged UDP forwarding for macOS DNS port 53.
//!
//! A per-user LaunchDaemon owns the protected socket. The desktop app keeps a
//! Unix-domain control connection open while forwarding is active; disconnect
//! closes UDP port 53, including after a crash. Administrator authorization is
//! only needed to install or update the versioned helper.

use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, SocketAddr};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader};
use tokio::net::{UdpSocket, UnixListener, UnixStream};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

const DAEMON_ARGUMENT: &str = "--privileged-udp-daemon";
const HELPER_VERSION: &str = env!("CARGO_PKG_VERSION");
const HELPER_ID: &str = "com.lhstack.dns-hearth.udp-forwarder";
const MAX_CONCURRENT_QUERIES: usize = 1024;
const MAX_DNS_MESSAGE_SIZE: usize = 65_535;
const FORWARD_TIMEOUT: Duration = Duration::from_secs(10);
const SERVICE_START_TIMEOUT: Duration = Duration::from_secs(30);
const SERVICE_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const FORWARD_CONTROL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub struct PrivilegedUdpDaemonConfig {
    socket_path: PathBuf,
    allowed_uid: u32,
}

#[derive(Debug)]
pub struct PrivilegedUdpForwarderLease {
    control_stream: Option<StdUnixStream>,
}

impl PrivilegedUdpDaemonConfig {
    pub fn from_process_arguments() -> Result<Option<Self>> {
        let mut arguments = std::env::args().skip(1);
        if arguments.next().as_deref() != Some(DAEMON_ARGUMENT) {
            return Ok(None);
        }
        let socket_path = PathBuf::from(parse_value(&mut arguments, "--socket")?);
        let allowed_uid = parse_value(&mut arguments, "--allowed-uid")?.parse()?;
        if arguments.next().is_some() {
            bail!("Unexpected privileged UDP daemon argument");
        }
        if !socket_path.is_absolute() || allowed_uid == 0 {
            bail!("Invalid privileged UDP daemon configuration");
        }
        Ok(Some(Self {
            socket_path,
            allowed_uid,
        }))
    }

    pub fn run(self) -> Result<()> {
        #[cfg(not(target_os = "macos"))]
        bail!("Privileged DNS forwarding is only supported on macOS");

        #[cfg(target_os = "macos")]
        {
            if unsafe { libc::geteuid() } != 0 {
                bail!("Privileged DNS daemon must run as root");
            }
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(self.run_async())
        }
    }

    async fn run_async(self) -> Result<()> {
        if let Some(parent) = self.socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let _ = std::fs::remove_file(&self.socket_path);
        let listener = UnixListener::bind(&self.socket_path).with_context(|| {
            format!(
                "Failed to bind helper socket: {}",
                self.socket_path.display()
            )
        })?;
        set_socket_owner_and_mode(&self.socket_path, self.allowed_uid)?;

        loop {
            let (stream, _) = listener.accept().await?;
            if peer_uid(&stream)? != self.allowed_uid {
                continue;
            }
            tokio::spawn(async move {
                if let Err(error) = handle_control_connection(stream).await {
                    eprintln!("Privileged DNS control connection failed: {error:#}");
                }
            });
        }
    }
}

impl PrivilegedUdpForwarderLease {
    pub async fn start(
        listen: SocketAddr,
        target: SocketAddr,
        _control_directory: &Path,
    ) -> Result<Self> {
        validate_forwarding_addresses(listen, target)?;
        let service = ServiceInstallation::for_current_user()?;
        ensure_service(service.clone()).await?;
        let control_stream =
            tokio::task::spawn_blocking(move || start_forwarding(service, listen, target))
                .await
                .context("Privileged DNS control task failed")??;
        Ok(Self {
            control_stream: Some(control_stream),
        })
    }

    pub async fn stop_and_wait(&mut self) {
        let Some(mut stream) = self.control_stream.take() else {
            return;
        };
        let _ = tokio::task::spawn_blocking(move || -> Result<()> {
            stream.write_all(b"STOP\n")?;
            stream.flush()?;
            let response = read_response(&stream)?;
            if response != "OK" {
                bail!("Privileged DNS daemon rejected stop request: {}", response);
            }
            Ok(())
        })
        .await;
    }
}

impl Drop for PrivilegedUdpForwarderLease {
    fn drop(&mut self) {
        // Closing the control stream is the daemon's crash-safe stop signal.
        self.control_stream.take();
    }
}

#[derive(Clone)]
struct ServiceInstallation {
    uid: u32,
    label: String,
    helper_path: PathBuf,
    plist_path: PathBuf,
    socket_path: PathBuf,
}

impl ServiceInstallation {
    fn for_current_user() -> Result<Self> {
        let uid = unsafe { libc::getuid() };
        if uid == 0 {
            bail!("The desktop application must not run as root");
        }
        let label = format!("{}.{}", HELPER_ID, uid);
        Ok(Self {
            uid,
            helper_path: PathBuf::from(format!(
                "/Library/PrivilegedHelperTools/{}-{}",
                HELPER_ID, uid
            )),
            plist_path: PathBuf::from(format!("/Library/LaunchDaemons/{}.plist", label)),
            socket_path: PathBuf::from(format!("/var/run/{}-{}.sock", HELPER_ID, uid)),
            label,
        })
    }
}

async fn ensure_service(service: ServiceInstallation) -> Result<()> {
    let probe = service.clone();
    if tokio::task::spawn_blocking(move || probe_service(&probe))
        .await
        .context("Privileged DNS probe task failed")?
        .is_ok()
    {
        return Ok(());
    }

    install_service_helper(&service).await?;

    let deadline = tokio::time::Instant::now() + SERVICE_START_TIMEOUT;
    loop {
        let probe = service.clone();
        match tokio::task::spawn_blocking(move || probe_service(&probe)).await {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(error)) if tokio::time::Instant::now() >= deadline => return Err(error),
            Err(error) => return Err(error.into()),
            _ => tokio::time::sleep(Duration::from_millis(150)).await,
        }
    }
}

fn probe_service(service: &ServiceInstallation) -> Result<()> {
    let mut stream = StdUnixStream::connect(&service.socket_path).with_context(|| {
        format!(
            "Cannot connect to privileged DNS service at {}",
            service.socket_path.display()
        )
    })?;
    stream.set_read_timeout(Some(SERVICE_PROBE_TIMEOUT))?;
    stream.set_write_timeout(Some(SERVICE_PROBE_TIMEOUT))?;
    writeln!(stream, "VERSION")?;
    stream.flush()?;
    let response = read_response(&stream)?;
    if response != format!("VERSION {}", HELPER_VERSION) {
        bail!("Privileged DNS helper version mismatch: {}", response);
    }
    Ok(())
}

fn start_forwarding(
    service: ServiceInstallation,
    listen: SocketAddr,
    target: SocketAddr,
) -> Result<StdUnixStream> {
    let mut stream = StdUnixStream::connect(&service.socket_path)?;
    stream.set_read_timeout(Some(FORWARD_CONTROL_TIMEOUT))?;
    stream.set_write_timeout(Some(FORWARD_CONTROL_TIMEOUT))?;
    writeln!(stream, "START {} {}", listen, target)?;
    stream.flush()?;
    let response = read_response(&stream)?;
    if response != "OK" {
        bail!(
            "Privileged DNS daemon failed to start forwarding: {}",
            response
        );
    }
    stream.set_read_timeout(None)?;
    stream.set_write_timeout(None)?;
    Ok(stream)
}

async fn install_service_helper(service: &ServiceInstallation) -> Result<()> {
    let executable = std::env::current_exe().context("Failed to locate application executable")?;
    let staging_directory = stage_helper_directory(service.uid)?;
    let staged_helper = staging_directory.join("dns-hearth-helper");
    let staged_plist = staging_directory.join(format!("{}.plist", service.label));
    stage_installation_files(&executable, &staged_helper, &staged_plist, service)?;

    // The elevated AppleScript process can be denied access to a development
    // binary located on a TCC-protected external volume. The unprivileged app
    // therefore stages immutable inputs in a private /private/tmp directory
    // before requesting authorization.
    let install_files = format!(
        "set -e; \
         test -x {source}; \
         test -f {temp_plist}; \
         mkdir -p /Library/PrivilegedHelperTools; \
         /usr/bin/install -o root -g wheel -m 755 {source} {helper}; \
         /usr/bin/install -o root -g wheel -m 644 {temp_plist} {plist}; \
         rm -f {socket}; \
         test -x {helper}; \
         test -f {plist}",
        source = shell_quote(&staged_helper.to_string_lossy()),
        helper = shell_quote(&service.helper_path.to_string_lossy()),
        temp_plist = shell_quote(&staged_plist.to_string_lossy()),
        plist = shell_quote(&service.plist_path.to_string_lossy()),
        socket = shell_quote(&service.socket_path.to_string_lossy()),
    );
    let result = async {
        authorize("安装特权 DNS 服务", install_files).await?;
        verify_installed_files(service)?;

        let reload = format!(
            "set -e; \
             launchctl bootout system/{label} >/dev/null 2>&1 || true; \
             launchctl bootstrap system {plist} 2>/dev/null || \
             launchctl kickstart system/{label}",
            label = shell_quote(&service.label),
            plist = shell_quote(&service.plist_path.to_string_lossy()),
        );
        authorize("重新加载特权 DNS 服务", reload).await
    }
    .await;
    let _ = std::fs::remove_dir_all(staging_directory);
    result
}

fn stage_helper_directory(uid: u32) -> Result<PathBuf> {
    let path = PathBuf::from(format!(
        "/private/tmp/{}-{}-{}",
        HELPER_ID,
        uid,
        std::process::id()
    ));
    if path.exists() {
        std::fs::remove_dir_all(&path)
            .with_context(|| format!("无法清理旧的 helper 暂存目录: {}", path.display()))?;
    }
    std::fs::create_dir(&path)
        .with_context(|| format!("无法创建 helper 暂存目录: {}", path.display()))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    Ok(path)
}

fn stage_installation_files(
    executable: &Path,
    staged_helper: &Path,
    staged_plist: &Path,
    service: &ServiceInstallation,
) -> Result<()> {
    std::fs::copy(executable, staged_helper).with_context(|| {
        format!(
            "无法将 helper 从 {} 暂存到 {}",
            executable.display(),
            staged_helper.display()
        )
    })?;
    std::fs::set_permissions(staged_helper, std::fs::Permissions::from_mode(0o700))?;
    std::fs::write(staged_plist, launch_daemon_plist(service))?;
    std::fs::set_permissions(staged_plist, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn verify_installed_files(service: &ServiceInstallation) -> Result<()> {
    verify_root_owned_file(&service.helper_path, 0o755, "特权 DNS helper")?;
    verify_root_owned_file(&service.plist_path, 0o644, "LaunchDaemon 配置")?;
    Ok(())
}

fn verify_root_owned_file(path: &Path, expected_mode: u32, label: &str) -> Result<()> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("{} 安装后无法读取: {}", label, path.display()))?;
    if !metadata.is_file() {
        bail!("{} 不是普通文件: {}", label, path.display());
    }
    if metadata.uid() != 0 {
        bail!("{} 必须归 root 所有，实际 uid={}", label, metadata.uid());
    }
    let mode = metadata.mode() & 0o777;
    if mode != expected_mode {
        bail!(
            "{} 权限错误，预期 {:o}，实际 {:o}",
            label,
            expected_mode,
            mode
        );
    }
    Ok(())
}

fn launch_daemon_plist(service: &ServiceInstallation) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{label}</string>
<key>ProgramArguments</key><array>
<string>{helper}</string><string>{argument}</string>
<string>--socket</string><string>{socket}</string>
<string>--allowed-uid</string><string>{uid}</string>
</array>
<key>KeepAlive</key><true/>
<key>RunAtLoad</key><true/>
<key>ProcessType</key><string>Interactive</string>
</dict></plist>
"#,
        label = xml_escape(&service.label),
        helper = xml_escape(&service.helper_path.to_string_lossy()),
        argument = DAEMON_ARGUMENT,
        socket = xml_escape(&service.socket_path.to_string_lossy()),
        uid = service.uid,
    )
}

async fn handle_control_connection(stream: UnixStream) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = TokioBufReader::new(reader).lines();
    let Some(command) = lines.next_line().await? else {
        return Ok(());
    };
    if command == "VERSION" {
        writer
            .write_all(format!("VERSION {}\n", HELPER_VERSION).as_bytes())
            .await?;
        return Ok(());
    }

    let (listen, target) = match parse_start_command(&command) {
        Ok(addresses) => addresses,
        Err(error) => {
            writer
                .write_all(format!("ERR {error:#}\n").as_bytes())
                .await?;
            return Ok(());
        }
    };
    let socket = match UdpSocket::bind(listen).await {
        Ok(socket) => Arc::new(socket),
        Err(error) => {
            writer
                .write_all(
                    format!("ERR Failed to bind privileged UDP socket to {listen}: {error}\n")
                        .as_bytes(),
                )
                .await?;
            return Ok(());
        }
    };
    writer.write_all(b"OK\n").await?;

    let cancellation = CancellationToken::new();
    let forwarding = tokio::spawn(run_forwarder(socket, target, cancellation.clone()));
    loop {
        match lines.next_line().await? {
            Some(value) if value == "STOP" => {
                stop_forwarding(cancellation, forwarding).await?;
                writer.write_all(b"OK\n").await?;
                return Ok(());
            }
            Some(value) => {
                writer
                    .write_all(format!("ERR unsupported command: {value}\n").as_bytes())
                    .await?;
            }
            None => {
                stop_forwarding(cancellation, forwarding).await?;
                return Ok(());
            }
        }
    }
}

async fn stop_forwarding(
    cancellation: CancellationToken,
    forwarding: tokio::task::JoinHandle<Result<()>>,
) -> Result<()> {
    cancellation.cancel();
    forwarding
        .await
        .context("Privileged DNS forwarding task failed")??;
    Ok(())
}

async fn run_forwarder(
    socket: Arc<UdpSocket>,
    target: SocketAddr,
    cancellation: CancellationToken,
) -> Result<()> {
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_QUERIES));
    let mut query_tasks = JoinSet::new();
    let mut buffer = vec![0_u8; MAX_DNS_MESSAGE_SIZE];
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => break,
            received = socket.recv_from(&mut buffer) => {
                let (length, client) = received?;
                let Ok(permit) = semaphore.clone().try_acquire_owned() else {
                    continue;
                };
                let request = buffer[..length].to_vec();
                let listening_socket = socket.clone();
                query_tasks.spawn(async move {
                    let _permit = permit;
                    if let Err(error) = forward_query(listening_socket, client, target, request).await {
                        eprintln!("DNS port forward failed: {error:#}");
                    }
                });
                while query_tasks.try_join_next().is_some() {}
            }
        }
    }
    query_tasks.abort_all();
    while query_tasks.join_next().await.is_some() {}
    drop(socket);
    Ok(())
}

async fn forward_query(
    listening_socket: Arc<UdpSocket>,
    client: SocketAddr,
    target: SocketAddr,
    request: Vec<u8>,
) -> Result<()> {
    let bind_address = match target.ip() {
        IpAddr::V4(_) => "127.0.0.1:0",
        IpAddr::V6(_) => "[::1]:0",
    };
    let upstream = UdpSocket::bind(bind_address).await?;
    upstream.send_to(&request, target).await?;
    let mut response = vec![0_u8; MAX_DNS_MESSAGE_SIZE];
    let (length, source) = tokio::time::timeout(FORWARD_TIMEOUT, upstream.recv_from(&mut response))
        .await
        .context("DNS forwarding response timed out")??;
    if source != target {
        bail!("DNS forwarding response came from unexpected source {source}");
    }
    listening_socket
        .send_to(&response[..length], client)
        .await?;
    Ok(())
}

fn parse_start_command(command: &str) -> Result<(SocketAddr, SocketAddr)> {
    let mut values = command.split_whitespace();
    if values.next() != Some("START") {
        bail!("Unsupported privileged DNS command");
    }
    let listen: SocketAddr = values.next().context("Missing listen address")?.parse()?;
    let target: SocketAddr = values.next().context("Missing target address")?.parse()?;
    if values.next().is_some() {
        bail!("Unexpected START command value");
    }
    validate_forwarding_addresses(listen, target)?;
    Ok((listen, target))
}

fn validate_forwarding_addresses(listen: SocketAddr, target: SocketAddr) -> Result<()> {
    if listen.port() != 53 {
        bail!("Privileged UDP forwarding is restricted to DNS port 53");
    }
    if !target.ip().is_loopback() || target.port() < 1024 {
        bail!("Privileged UDP forwarding target must be an unprivileged loopback socket");
    }
    Ok(())
}

fn read_response(stream: &StdUnixStream) -> Result<String> {
    let mut response = String::new();
    BufReader::new(stream.try_clone()?).read_line(&mut response)?;
    let response = response.trim().to_string();
    if response.is_empty() {
        bail!("Privileged DNS daemon closed the control connection");
    }
    Ok(response)
}

fn parse_value(
    arguments: &mut impl Iterator<Item = String>,
    expected_name: &str,
) -> Result<String> {
    let actual_name = arguments
        .next()
        .with_context(|| format!("Missing {expected_name}"))?;
    if actual_name != expected_name {
        bail!("Expected {expected_name}, received {actual_name}");
    }
    arguments
        .next()
        .with_context(|| format!("Missing value for {expected_name}"))
}

async fn authorize(operation: &str, shell_command: String) -> Result<()> {
    let script =
        "on run argv\ndo shell script (item 1 of argv) with administrator privileges\nend run";
    let child = tokio::process::Command::new("/usr/bin/osascript")
        .args(["-e", script, "--", &shell_command])
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("Failed to request macOS administrator authorization")?;
    let output = match tokio::time::timeout(SERVICE_START_TIMEOUT, child.wait_with_output()).await {
        Ok(result) => result?,
        Err(_) => {
            bail!(
                "{}超过 {} 秒，已中止；请检查旧 LaunchDaemon 状态",
                operation,
                SERVICE_START_TIMEOUT.as_secs()
            );
        }
    };
    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let details = [stdout, stderr]
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(" | ");
        if details.contains("User canceled") || details.contains("-128") {
            bail!("已取消 macOS 管理员授权，无法安装 DNS 端口服务");
        }
        bail!(
            "{}失败（退出码 {:?}）: {}",
            operation,
            output.status.code(),
            details
        );
    }
    Ok(())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(target_os = "macos")]
fn set_socket_owner_and_mode(path: &Path, uid: u32) -> Result<()> {
    let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())?;
    if unsafe { libc::chown(c_path.as_ptr(), uid, u32::MAX) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn set_socket_owner_and_mode(_path: &Path, _uid: u32) -> Result<()> {
    bail!("Privileged DNS forwarding is only supported on macOS")
}

#[cfg(target_os = "macos")]
fn peer_uid(stream: &UnixStream) -> Result<u32> {
    use std::os::fd::AsRawFd;
    let mut uid = 0;
    let mut gid = 0;
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(uid)
}

#[cfg(not(target_os = "macos"))]
fn peer_uid(_stream: &UnixStream) -> Result<u32> {
    bail!("Privileged DNS forwarding is only supported on macOS")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quote_handles_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\"'\"'b'");
    }

    #[test]
    fn each_user_has_an_isolated_service_name() {
        let service = ServiceInstallation::for_current_user().unwrap();
        assert!(service.label.ends_with(&service.uid.to_string()));
        assert!(service
            .socket_path
            .to_string_lossy()
            .contains(&service.uid.to_string()));
    }

    #[test]
    fn forwarding_rejects_non_dns_privileged_port() {
        assert!(validate_forwarding_addresses(
            "127.0.0.1:54".parse().unwrap(),
            "127.0.0.1:10053".parse().unwrap()
        )
        .is_err());
    }
}
