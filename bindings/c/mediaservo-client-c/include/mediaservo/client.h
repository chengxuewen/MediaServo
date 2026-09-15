/* MediaServo client C ABI (舱端/消费侧 SDK) — 登录 + SFU 视频消费 + 控制 DataChannel。
 *
 * MAJOR = C ABI 版本 (D241)：within MAJOR 只加法，二进制兼容。
 * 本头文件手工维护（稳定导出面，等价 cbindgen 输出纪律）。
 * 共享 C 类型（mediaservo_err_t）见 common.h。
 *
 * 用法（控制回路，镜像 crates/mediaservo-client/examples/basic.rs）:
 *   ms_client_login_config_t lc = MS_CLIENT_LOGIN_CONFIG_DEFAULT;
 *   lc.http_base_url = "http://10.0.0.2:9800"; lc.username = "operator"; lc.password = "...";
 *   char token[4096];
 *   if (ms_client_login(&lc, token, sizeof(token)) != MEDIASERVO_OK) { 读 ms_client_last_error }
 *   ms_client_config_t cfg = MEDIASERVO_CLIENT_CONFIG_DEFAULT;
 *   cfg.signaling_url = "ws://10.0.0.2:9800/ws"; cfg.room = "vehicle_1"; cfg.jwt = token;
 *   ms_client_session_t* s = NULL;
 *   ms_client_session_create(&cfg, &s);
 *   const char* labels[] = {"chassis"};
 *   ms_client_control_t* c = NULL;
 *   ms_client_open_control(s, labels, 1, &c);
 *   ms_client_control_on_ack(c, on_ack, NULL);
 *   ms_client_control_send(c, "chassis", 1, "steer", "{\"deg\":10}");
 *   ...
 *   ms_client_control_close(c);
 *   ms_client_session_close(s);
 *
 * 生命周期契约（承 link-c R2 纪律）：
 * - handle 单线程属主；close 后任何 API 调用为 UB（close(NULL) 幂等返回 OK）。
 * - session_close = 置 closed 标志 → 关会话（唤醒视频泵）→ join 视频泵 → 释放内存。
 *   控制 handle 独立关闭（control_close join 自己的 ack 泵）；先关 control 后关 session
 *   为正序，反序时 ack 泵随会话信令面消亡自然收敛。
 * - 帧/ack 回调仅在各自内部泵线程触发；回调调用期间不持任何锁；回调必须快速返回；
 *   回调内禁止调用任何 ms_client_* API（含 close）——未定义行为。
 * - 回调内 JSON 字符串与 frame.data 指针仅在回调内有效（需保留请拷贝）。
 * - 全部配置结构首字段 struct_size：填 sizeof(结构)，库校验 >= 已知尺寸、
 *   超长忽略——结构演进不破坏二进制兼容（R3）。
 * - 所有阻塞调用在进程级共享 multi_thread tokio runtime 上 block_on
 *   （ack 泵/WS 读循环为 tokio::spawn 后台任务，任务存活要求 runtime 长于任何单次调用）。
 */
#ifndef MEDIASERVO_CLIENT_H
#define MEDIASERVO_CLIENT_H

#include <stddef.h>
#include <stdint.h>

#include <mediaservo/common.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ── 错误码（0 = ok, <0 = error；ClientError 全变体映射，穷尽 match 单测钉）── */
#define MEDIASERVO_CLIENT_ERR_INVALID_ARG   (-1)
#define MEDIASERVO_CLIENT_ERR_LOGIN         (-2)   /* 登录请求/解析失败 */
#define MEDIASERVO_CLIENT_ERR_UNAUTHORIZED  (-3)   /* InvalidCredentials / AuthRejected(4003/4010/4011) */
#define MEDIASERVO_CLIENT_ERR_DENIED        (-4)   /* ControlDenied（server 4012） */
#define MEDIASERVO_CLIENT_ERR_TIMEOUT       (-5)   /* Timeout{what} */
#define MEDIASERVO_CLIENT_ERR_SIGNAL        (-6)   /* Signal/LinkError（WS 建连/断连/收发） */
#define MEDIASERVO_CLIENT_ERR_PROTOCOL      (-7)   /* ProtocolTooLow / ProtocolUnsupported / 4101 */
#define MEDIASERVO_CLIENT_ERR_MALFORMED     (-8)   /* MalformedResponse / UnsupportedScheme */
#define MEDIASERVO_CLIENT_ERR_STATE         (-9)   /* InvalidState（未开通道/会话或泵已关） */
#define MEDIASERVO_CLIENT_ERR_INTERNAL     (-10)   /* Server(非分类码) / WebRtc / Io / panic 兜底 */

/* ── 登录配置 ── */
typedef struct ms_client_login_config_t {
    size_t struct_size;      /* sizeof(ms_client_login_config_t) */
    const char* http_base_url; /* "http://host:port"（v1 仅明文 http；POST {base}/api/auth/login） */
    const char* username;
    const char* password;
} ms_client_login_config_t;

#define MS_CLIENT_LOGIN_CONFIG_DEFAULT { sizeof(ms_client_login_config_t), NULL, NULL, NULL }

/* ── 会话配置 ── */
typedef struct ms_client_config_t {
    size_t struct_size;      /* sizeof(ms_client_config_t) */
    const char* signaling_url; /* "ws://host:9800/ws" */
    const char* room;          /* 房间 ID */
    const char* jwt;           /* 账号 JWT（ms_client_login 输出）；与 psk 二选一（恰一非空） */
    const char* psk;           /* PSK 直传（专网无账号场景）；与 jwt 二选一 */
    const char* role;          /* "Client"/"Viewer"→消费角色, "Remote"→舱对端角色; NULL=Client。
                                *   注: Client/Viewer 本地同形（PeerRole::Consumer）——
                                *   控制权限差异由 server 账号 can_control 门裁决，非本地角色。 */
} ms_client_config_t;

#define MEDIASERVO_CLIENT_CONFIG_DEFAULT { sizeof(ms_client_config_t), NULL, NULL, NULL, NULL, NULL }

/* ── opaque handle ── */
typedef struct ms_client_session_t ms_client_session_t;
typedef struct ms_client_control_t ms_client_control_t;

/* ── 视频帧描述（I420；data 仅在回调内有效）── */
typedef struct mediaservo_client_frame_t {
    uint32_t width;
    uint32_t height;
    int64_t  ts_us;         /* 上游无时钟源时恒 0（按到达序消费） */
    const uint8_t* data;    /* I420 平面连续（len 字节）；仅回调内有效 */
    size_t   len;
} mediaservo_client_frame_t;

/* ── 回调（各自内部泵线程触发；串/帧仅回调内有效）── */
typedef void (*ms_client_frame_cb)(ms_client_session_t* s, const mediaservo_client_frame_t* frame, void* user);
typedef void (*ms_client_ack_cb)(ms_client_control_t* c, const char* ack_json, void* user);

/* ── API ── */

/* 账号登录（POST {http_base}/api/auth/login，阻塞）。成功 out_token = NUL 结尾 JWT
 * （cap 不足 → ERR_INVALID_ARG，不截断写入）。 */
mediaservo_err_t ms_client_login(const ms_client_login_config_t* cfg, char* out_token, size_t cap);

/* 信令连接 + 入房（阻塞）。jwt/psk 恰一非空；成功后 *out 指向新 handle（调用方 close）。 */
mediaservo_err_t ms_client_session_create(const ms_client_config_t* cfg, ms_client_session_t** out);

/* 谈成的方言版本（S0；旧 server = 1）。 */
mediaservo_err_t ms_client_session_negotiated(const ms_client_session_t* s, uint32_t* out_protocol);

/* 消费房间第一路视频：等 producer(≤30s) → consume → 启动帧泵线程（阻塞，首调用生效；
 * 重复调用 → ERR_STATE）。帧回调仅在该泵线程触发。 */
mediaservo_err_t ms_client_session_consume_video(ms_client_session_t* s, ms_client_frame_cb cb, void* user);

/* 开出程控制通道集（每会话一次性——ack 泵每会话一条；二次调用 → ERR_STATE）。
 * labels 为通道名数组（如 "chassis"/"gimbal"）。方言 <2 本地预拒（ERR_PROTOCOL）。 */
mediaservo_err_t ms_client_open_control(ms_client_session_t* s, const char* const* labels, size_t labels_len, ms_client_control_t** out);

/* 注册 ack 回调（首次注册启动 ack 泵；重复注册替换，旧回调留至 close 释放）。
 * cb=NULL 取消注册。ack_json = ControlAck 序列化（{"ack":..,"result":..}）。 */
mediaservo_err_t ms_client_control_on_ack(ms_client_control_t* c, ms_client_ack_cb cb, void* user);

/* 发送一条命令信封 {seq, cmd, payload}（serde 构造，payload_json NULL/"" = null；
 * 非法 JSON → ERR_INVALID_ARG）。seq 由调用方自增维护（D-H3 重发幂等安全）。 */
mediaservo_err_t ms_client_control_send(ms_client_control_t* c, const char* label, uint64_t seq, const char* cmd, const char* payload_json);

/* server 分配的 data producer id 的 JSON 数组（观测面；cap 不足 → ERR_INVALID_ARG）。 */
mediaservo_err_t ms_client_control_producer_ids(const ms_client_control_t* c, char* out_json, size_t cap);

/* 关闭控制通道并释放 handle（幂等；join ack 泵后才释放内存）。 */
mediaservo_err_t ms_client_control_close(ms_client_control_t* c);

/* 关闭会话并释放 handle（幂等；join 视频泵后才释放内存；不隐式关 control）。 */
mediaservo_err_t ms_client_session_close(ms_client_session_t* s);

/* ── 通用 ── */

/* 最近一次错误详情（线程安全）。 */
mediaservo_err_t ms_client_last_error(char* buf, size_t len);

/* SDK 版本 (MAJOR.MINOR.PATCH)。 */
mediaservo_err_t ms_client_version(char* buf, size_t len);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* MEDIASERVO_CLIENT_H */
