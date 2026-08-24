//! Task C3: host-recorder 进程测试 — FrameBus 订阅 → deck Recorder 落盘 MP4。
//!
//! - `bad_args_exit_2_with_usage`: 缺参/坏参 → exit 2 + stderr 用法提示
//! - `record_disabled_exits_0`: [record] enabled=false → exit 0 + stdout 消息
//!   （进程级门控；oxfile 仍含 recorder app，运行时 gate）
//! - `recorder_writes_mp4_from_capturer_frames`: capturer（真进程）+ recorder
//!   （真进程）→ MP4 落盘 → ffprobe 验证 duration>0 + codec=h264 →
//!   SIGTERM 双双优雅退出 0
//!
//! 前置: 测试须在 pixi run 环境运行（ffprobe/FFmpeg 动态库来自 pixi env）;
//! C25: 跑前清 `/tmp/iceoryx2` + `/dev/shm/iox2_*`。

#![cfg(target_os = "linux")]

use std::io::Read;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;

use mediaservo_link::{
    CapabilityToken, Ed25519SigningKey, Ed25519VerifyingKey, NodeAcl, NodeId, Role, TokenFile,
};

// 测试用 Ed25519 密钥对（openssl 生成，仅测试用；与 link/deck/capturer/streamer 测试同源）。
const PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIObCg8b+Le6kKOI/+pE+4+YhXUlr6X6h7q8p/MjvHmXT\n-----END PRIVATE KEY-----\n";
const PUB_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAgXprEbnahCZoZtLpiUqR0ruqtzEfRXk/Gl/6F6PEm4o=\n-----END PUBLIC KEY-----\n";

fn token(role: Role, node_id: &str) -> (CapabilityToken, Ed25519VerifyingKey) {
    let sk = Ed25519SigningKey::from_pem(PRIV_PEM.as_bytes());
    let vk = Ed25519VerifyingKey::from_pem(PUB_PEM.as_bytes());
    let acl = NodeAcl::for_role(NodeId::new(node_id), role);
    (CapabilityToken::sign(&acl, 3600, &sk).unwrap(), vk)
}

/// C25: 清 iceoryx2 0.9.3 运行时残留（/tmp/iceoryx2 + /dev/shm/iox2_*）。
fn cleanup_iceoryx() {
    let _ = std::fs::remove_dir_all("/tmp/iceoryx2");
    if let Ok(entries) = std::fs::read_dir("/dev/shm") {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with("iox2_") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

#[test]
fn bad_args_exit_2_with_usage() {
    for args in [
        vec![],                 // 全缺
        vec!["--config"],       // 缺值
        vec!["--config", "x"],  // 缺 token
        vec!["--bogus", "y"],   // 未知参数
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_host-recorder"))
            .args(&args)
            .output()
            .expect("spawn host-recorder");
        assert_eq!(out.status.code(), Some(2), "args {args:?} 应 exit 2");
        assert!(
            !out.stderr.is_empty(),
            "args {args:?} stderr 应有用法提示, got: {:?}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn record_disabled_exits_0() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg_path = dir.path().join("host.yaml");
    std::fs::write(
        &cfg_path,
        "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\nrecord:\n  enabled: false\n",
    )
    .expect("write host.yaml");
    // 门控先于令牌读取 — 禁用时凭据无效也应 exit 0（garbage token 验证顺序）
    let tok_path = dir.path().join("garbage.token");
    std::fs::write(&tok_path, b"garbage").expect("write token");
    let out = Command::new(env!("CARGO_BIN_EXE_host-recorder"))
        .args([
            "--config",
            cfg_path.to_str().expect("cfg utf8"),
            "--token",
            tok_path.to_str().expect("token utf8"),
        ])
        .output()
        .expect("spawn host-recorder");
    assert_eq!(out.status.code(), Some(0), "disabled 应 exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("未启用"), "应指明未启用, got: {stdout}");
}

/// 读取子进程日志（stdout+stderr 合并到同一文件）。
fn read_log(file: &tempfile::NamedTempFile) -> String {
    let mut out = String::new();
    file.reopen()
        .expect("reopen log")
        .read_to_string(&mut out)
        .expect("read log");
    out
}

/// 轮询日志直到出现 needle（≤ timeout_secs）。
fn wait_for_secs(log: &tempfile::NamedTempFile, needle: &str, timeout_secs: u64) {
    for _ in 0..(timeout_secs * 2) {
        if read_log(log).contains(needle) {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    panic!("{timeout_secs}s 内未见 {needle:?}, log:\n{}", read_log(log));
}

/// 轮询日志直到出现 needle（≤10s）。
fn wait_for(log: &tempfile::NamedTempFile, needle: &str) {
    wait_for_secs(log, needle, 10);
}

/// ffprobe 单值查询（pixi run 环境 PATH 提供 ffprobe）。
fn ffprobe(args: &[&str]) -> String {
    let out = Command::new("ffprobe")
        .args(args)
        .output()
        .expect("spawn ffprobe（需 pixi run 环境）");
    assert!(
        out.status.success(),
        "ffprobe 失败: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// E2E: capturer（真进程发布 camera/cam0）→ recorder（真进程订阅 + 落盘）
/// → ffprobe 验证 MP4（duration>0 + h264）→ SIGTERM 双双优雅退出 0。
#[tokio::test]
async fn recorder_writes_mp4_from_capturer_frames() {
    cleanup_iceoryx();
    let dir = tempfile::tempdir().expect("tempdir");
    let pid = std::process::id();
    let out_dir = dir.path().join("recordings");

    // host.yaml: cam0 generator 30fps + [record] enabled + 测试专用 out_dir
    let cfg_path = dir.path().join("host.yaml");
    std::fs::write(
        &cfg_path,
        format!(
            "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 30\n\
             record:\n  enabled: true\n  out_dir: \"{}\"\n",
            out_dir.display()
        ),
    )
    .expect("write host.yaml");

    // 令牌: capturer=Capture（发布 camera/*），recorder=Recorder（订阅 camera/*）
    let (cap_tok, cap_vk) = token(Role::Capture, &format!("capture-{pid}"));
    let cap_path = dir.path().join("cam0.token");
    std::fs::write(&cap_path, TokenFile::encode(&cap_tok, &cap_vk)).expect("write cap token");
    let (rec_tok, rec_vk) = token(Role::Recorder, &format!("recorder-{pid}"));
    let rec_path = dir.path().join("recorder.token");
    std::fs::write(&rec_path, TokenFile::encode(&rec_tok, &rec_vk)).expect("write rec token");

    // capturer 进程（先起：录制首帧 gate 依赖发布端）
    let cap_log = tempfile::NamedTempFile::new().expect("cap log");
    let mut capturer = Command::new(env!("CARGO_BIN_EXE_host-capturer"))
        .args([
            "--camera",
            "cam0",
            "--config",
            cfg_path.to_str().expect("cfg utf8"),
            "--token",
            cap_path.to_str().expect("cap token utf8"),
        ])
        .stdout(Stdio::from(cap_log.reopen().expect("reopen cap log")))
        .stderr(Stdio::from(cap_log.reopen().expect("reopen cap log")))
        .spawn()
        .expect("spawn host-capturer");
    wait_for(&cap_log, "capturer ready");

    // recorder 进程
    let rec_log = tempfile::NamedTempFile::new().expect("rec log");
    let mut recorder = Command::new(env!("CARGO_BIN_EXE_host-recorder"))
        .args([
            "--config",
            cfg_path.to_str().expect("cfg utf8"),
            "--token",
            rec_path.to_str().expect("rec token utf8"),
        ])
        .stdout(Stdio::from(rec_log.reopen().expect("reopen rec log")))
        .stderr(Stdio::from(rec_log.reopen().expect("reopen rec log")))
        .spawn()
        .expect("spawn host-recorder");
    wait_for(&rec_log, "recorder ready");

    // 录制约 2.5s → SIGTERM → flush + trailer 收尾
    tokio::time::sleep(Duration::from_millis(2500)).await;
    unsafe { libc::kill(recorder.id() as i32, libc::SIGTERM) };
    let st = recorder.wait().expect("wait recorder");
    assert_eq!(st.code(), Some(0), "recorder 应优雅退出 0, got {st:?}");

    // ffprobe 验证: duration > 0.5s + codec h264（moov 完整 = trailer 已写）
    let mp4 = out_dir.join("cam0.mp4");
    let duration = ffprobe(&[
        "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0",
        mp4.to_str().expect("mp4 utf8"),
    ]);
    let codec = ffprobe(&[
        "-v", "error", "-select_streams", "v:0", "-show_entries", "stream=codec_name", "-of",
        "csv=p=0", mp4.to_str().expect("mp4 utf8"),
    ]);
    eprintln!("[recorder_e2e] duration={duration}s codec={codec}");
    assert!(
        duration.parse::<f64>().unwrap_or(0.0) > 0.5,
        "duration 应 > 0.5s, got: {duration}"
    );
    assert_eq!(codec.trim(), "h264", "codec 应为 h264, got: {codec}");

    // capturer 同样优雅退出
    unsafe { libc::kill(capturer.id() as i32, libc::SIGTERM) };
    let ct = capturer.wait().expect("wait capturer");
    assert_eq!(ct.code(), Some(0), "capturer 应优雅退出 0, got {ct:?}");
}

/// 断言失败时 SIGKILL 子进程（防孤儿；正常路径已 wait → ESRCH 忽略）。
struct KillOnDrop(u32);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        unsafe { libc::kill(self.0 as i32, libc::SIGKILL) };
    }
}

/// C5 崩溃恢复契约（审查 #1 回归）: recorder 在 capturer 缺席时存活并告警
/// （stall warning 必须 ≤20s 出现 — 修复前内层 10s timeout 被 deck pump 的
/// 50ms 外层轮询取消 → 永不触发），capturer 后启动后录制恢复（帧流动 → MP4 有效）。
#[tokio::test]
async fn recorder_survives_missing_capturer_and_resumes() {
    cleanup_iceoryx();
    let dir = tempfile::tempdir().expect("tempdir");
    let pid = std::process::id();
    let out_dir = dir.path().join("recordings");

    let cfg_path = dir.path().join("host.yaml");
    std::fs::write(
        &cfg_path,
        format!(
            "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 30\n\
             record:\n  enabled: true\n  out_dir: \"{}\"\n",
            out_dir.display()
        ),
    )
    .expect("write host.yaml");

    // recorder 先起（capturer 缺席）
    let (rec_tok, rec_vk) = token(Role::Recorder, &format!("recorder-{pid}"));
    let rec_path = dir.path().join("recorder.token");
    std::fs::write(&rec_path, TokenFile::encode(&rec_tok, &rec_vk)).expect("write rec token");
    let rec_log = tempfile::NamedTempFile::new().expect("rec log");
    let mut recorder = Command::new(env!("CARGO_BIN_EXE_host-recorder"))
        .args([
            "--config",
            cfg_path.to_str().expect("cfg utf8"),
            "--token",
            rec_path.to_str().expect("rec token utf8"),
        ])
        .stdout(Stdio::from(rec_log.reopen().expect("reopen rec log")))
        .stderr(Stdio::from(rec_log.reopen().expect("reopen rec log")))
        .spawn()
        .expect("spawn host-recorder");
    let _kill_on_drop = KillOnDrop(recorder.id()); // panic 时不留孤儿进程
    wait_for(&rec_log, "recorder ready");

    // 无帧 stall 警告必须出现（10s 无帧 + 节流 → 约 10-11s; 给 20s 窗口）
    wait_for_secs(&rec_log, "无帧", 20);

    // capturer 后启动 → 录制恢复（帧流动 → MP4 duration>0）
    let (cap_tok, cap_vk) = token(Role::Capture, &format!("capture-{pid}"));
    let cap_path = dir.path().join("cam0.token");
    std::fs::write(&cap_path, TokenFile::encode(&cap_tok, &cap_vk)).expect("write cap token");
    let cap_log = tempfile::NamedTempFile::new().expect("cap log");
    let mut capturer = Command::new(env!("CARGO_BIN_EXE_host-capturer"))
        .args([
            "--camera",
            "cam0",
            "--config",
            cfg_path.to_str().expect("cfg utf8"),
            "--token",
            cap_path.to_str().expect("cap token utf8"),
        ])
        .stdout(Stdio::from(cap_log.reopen().expect("reopen cap log")))
        .stderr(Stdio::from(cap_log.reopen().expect("reopen cap log")))
        .spawn()
        .expect("spawn host-capturer");
    wait_for(&cap_log, "capturer ready");

    tokio::time::sleep(Duration::from_millis(2500)).await;
    unsafe { libc::kill(recorder.id() as i32, libc::SIGTERM) };
    let st = recorder.wait().expect("wait recorder");
    assert_eq!(st.code(), Some(0), "recorder 应优雅退出 0, got {st:?}");

    let mp4 = out_dir.join("cam0.mp4");
    let duration = ffprobe(&[
        "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0",
        mp4.to_str().expect("mp4 utf8"),
    ]);
    let codec = ffprobe(&[
        "-v", "error", "-select_streams", "v:0", "-show_entries", "stream=codec_name", "-of",
        "csv=p=0", mp4.to_str().expect("mp4 utf8"),
    ]);
    eprintln!("[recorder_e2e] stall-recovery duration={duration}s codec={codec}");
    assert!(
        duration.parse::<f64>().unwrap_or(0.0) > 0.5,
        "capturer 后启动后应恢复录制, duration got {duration}"
    );
    assert_eq!(codec.trim(), "h264", "codec 应为 h264, got: {codec}");

    unsafe { libc::kill(capturer.id() as i32, libc::SIGTERM) };
    let ct = capturer.wait().expect("wait capturer");
    assert_eq!(ct.code(), Some(0), "capturer 应优雅退出 0, got {ct:?}");
}
