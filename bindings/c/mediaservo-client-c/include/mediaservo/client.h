/* MediaServo client C ABI (舱端/消费侧 SDK) — 登录 + SFU 视频消费 + 控制 DataChannel。
 *
 * MAJOR = C ABI 版本 (D241)：within MAJOR 只加法，二进制兼容。
 * 本头文件手工维护（稳定导出面，等价 cbindgen 输出纪律）。
 * 共享 C 类型（mediaservo_err_t）见 common.h。
 *
 * S6 批1b（⚠ breaking）：全量前缀 ms_client_* → mediaservo_client_*（零别名）；
 * login 转扁平参数形；⊘ 旧形保留一周期（last_error 全局 / consume_video 首路 /
 * video_stats 会话 union）；新面 = 多路 Consumer + 句柄错误槽（error_t 机读
 * code/wire_code/retryable）+ 会话状态观测 + ack token 累积注册。
 *
 * 用法（多路消费 + 控制回路，镜像 crates/mediaservo-client/examples/basic.rs）:
 *   char token[4096]; size_t need = 0;
 *   if (mediaservo_client_login("http://10.0.0.2:9800", "operator", "...",
 *                               token, sizeof(token), &need) != MEDIASERVO_OK) { 读 last_error }
 *   mediaservo_client_config_t cfg = MEDIASERVO_CLIENT_CONFIG_DEFAULT;
 *   cfg.signaling_url = "ws://10.0.0.2:9800/ws"; cfg.room = "vehicle_1"; cfg.jwt = token;
 *   mediaservo_client_session_t* s = NULL;
 *   mediaservo_client_session_create(&cfg, &s);
 *   char pid[256];
 *   mediaservo_client_session_wait_video(s, 30000, pid, sizeof(pid), NULL);
 *   mediaservo_client_consumer_t* cons = NULL;
 *   mediaservo_client_session_consume(s, pid, on_frame, user, on_free, &cons);
 *   const char* labels[] = {"chassis"};
 *   mediaservo_client_control_t* c = NULL;
 *   mediaservo_client_open_control(s, labels, 1, &c);
 *   uint64_t tok = 0;
 *   mediaservo_client_control_on_ack(c, on_ack, user, on_free, &tok);
 *   mediaservo_client_control_send(c, "chassis", 1, "steer", "{\"deg\":10}");
 *   ...
 *   mediaservo_client_control_off_ack(c, tok);            // 或留待 close 统一回收
 *   mediaservo_client_control_close(c);
 *   mediaservo_client_consumer_close(cons);
 *   mediaservo_client_session_close(s);
 *
 * 生命周期契约（承 link-c R2 纪律）：
 * - handle 单线程属主；close 后任何 API 调用为 UB（close(NULL) 幂等返回 OK）。
 * - session_close = 置 closed 标志 → 关会话（唤醒 ⊘ 视频泵；watch sender 消亡唤醒
 *   状态泵）→ join 泵 → 逐条 user_free → 释放内存。不隐式关 consumer/control。
 * - consumer_close = 撤本路接收端（K1 重放判死该路）→ join 泵（≤250ms）→
 *   user_free 恰好一次 → 释放。
 * - 控制 handle 独立关闭（control_close join 自己的 ack 泵）；先关 control 后关
 *   session 为正序，反序时 ack 泵随会话信令面消亡自然收敛。
 * - 帧/ack/状态回调仅在各自内部泵线程触发；回调调用期间不持任何锁；回调必须快速
 *   返回；回调内禁止调用任何 mediaservo_client_* API（含 close）——未定义行为。
 * - 回调内 JSON 字符串与 frame.data 指针仅在回调内有效（需保留请拷贝）。
 * - user_free 契约（每条注册恰好调用一次）：on_state → session_close；
 *   consume → consumer_close；on_ack → off_ack 后的泵回收轮（≤1s）或
 *   control_close（未 off 的随 close 释放）。
 * - 全部配置结构首字段 struct_size：填 sizeof(结构)，库校验 >= 已知尺寸、
 *   超长忽略——结构演进不破坏二进制兼容（R3）。
 * - needed 溢出合同（login/list_rooms/wait_video/consumer_id/consumer_stats/
 *   producer_ids/video_stats）：cap 不足 → *needed 写入必需字节数（含 NUL）+
 *   返回 ERR_INVALID_ARG（不写半截）；成功 → *needed = 实际长度。needed 可 NULL。
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
#define MEDIASERVO_CLIENT_ERR_UNAUTHORIZED  (-3)   /* InvalidCredentials / AuthRejected(4003/4010/4011) /
                                                       RestRejected(REST 发现面非 2xx) */
#define MEDIASERVO_CLIENT_ERR_DENIED        (-4)   /* ControlDenied（server 4012） */
#define MEDIASERVO_CLIENT_ERR_TIMEOUT       (-5)   /* Timeout{what} */
#define MEDIASERVO_CLIENT_ERR_SIGNAL        (-6)   /* Signal/LinkError（WS 建连/断连/收发） */
#define MEDIASERVO_CLIENT_ERR_PROTOCOL      (-7)   /* ProtocolTooLow / ProtocolUnsupported / 4101 */
#define MEDIASERVO_CLIENT_ERR_MALFORMED     (-8)   /* MalformedResponse / UnsupportedScheme */
#define MEDIASERVO_CLIENT_ERR_STATE         (-9)   /* InvalidState（未开通道/会话或泵已关） */
#define MEDIASERVO_CLIENT_ERR_INTERNAL     (-10)   /* Server(非分类码) / WebRtc / Io / panic 兜底 */

/* 会话连接态线值（session_state/out_state + state 回调第二参数）。 */
#define MEDIASERVO_CLIENT_STATE_DISCONNECTED 0u /* auto_reconnect 关且断链 */
#define MEDIASERVO_CLIENT_STATE_CONNECTED    1u /* 信令已连接（正常服务态） */
#define MEDIASERVO_CLIENT_STATE_RECONNECTING 2u /* 断链后重连环内 */
#define MEDIASERVO_CLIENT_STATE_FAILED       3u /* 不可重试拒（auth 族/4101，终态） */

/* DC 就绪态线值（control_ready_state；= RTCDataChannelState 稳定映射）。 */
#define MEDIASERVO_CLIENT_DC_CONNECTING 0u
#define MEDIASERVO_CLIENT_DC_OPEN       1u
#define MEDIASERVO_CLIENT_DC_CLOSING    2u
#define MEDIASERVO_CLIENT_DC_CLOSED     3u

/* ── 机读错误结构（K3；session_error 出参）── */
typedef struct mediaservo_client_error_t {
    size_t   struct_size;  /* sizeof(mediaservo_client_error_t) */
    int      code;         /* MEDIASERVO_CLIENT_ERR_*（0 = 本句柄无错） */
    uint32_t wire_code;    /* server wire 码回读（0 = 本地/无码域） */
    uint8_t  retryable;    /* 1 = 可重试族（D273 分类的 C 镜像） */
} mediaservo_client_error_t;

/* ── 登录配置（⊘ 保留一周期：批1b 起 login 转扁平参数形，本结构不再被消费；
 *    批2 随 ⊘ 清单一并移除）── */
typedef struct mediaservo_client_login_config_t {
    size_t struct_size;      /* sizeof(mediaservo_client_login_config_t) */
    const char* http_base_url; /* "http://host:port"（v1 仅明文 http；POST {base}/api/auth/login） */
    const char* username;
    const char* password;
} mediaservo_client_login_config_t;

#define MEDIASERVO_CLIENT_LOGIN_CONFIG_DEFAULT { sizeof(mediaservo_client_login_config_t), NULL, NULL, NULL }

/* ── 会话配置 ── */
typedef struct mediaservo_client_config_t {
    size_t struct_size;      /* sizeof(mediaservo_client_config_t) */
    const char* signaling_url; /* "ws://host:9800/ws" */
    const char* room;          /* 房间 ID */
    const char* jwt;           /* 账号 JWT（mediaservo_client_login 输出）；与 psk 二选一（恰一非空） */
    const char* psk;           /* PSK 直传（专网无账号场景）；与 jwt 二选一 */
    const char* role;          /* "Client"/"Viewer"→消费角色, "Remote"→舱对端角色; NULL=Client。
                                *   注: Client/Viewer 本地同形（PeerRole::Consumer）——
                                *   控制权限差异由 server 账号 can_control 门裁决，非本地角色。 */
    const char* hmac_key_file; /* 急停 HMAC 密钥文件（W4b；须 0600，非空，≤4KiB 尾换行自动剥离）。
                                *   NULL = estop 不签名（车端未配 key = 迁移放行形；
                                *   车端已配 key = 车端拒签=正确裁决非静默）。路径坏不拦
                                *   建会话——estop 调用点报 INVALID_ARG。G13: 密钥不走 argv/env。 */
} mediaservo_client_config_t;

#define MEDIASERVO_CLIENT_CONFIG_DEFAULT \
    { sizeof(mediaservo_client_config_t), NULL, NULL, NULL, NULL, NULL, NULL }

/* ── opaque handle ── */
typedef struct mediaservo_client_session_t mediaservo_client_session_t;
typedef struct mediaservo_client_control_t mediaservo_client_control_t;
typedef struct mediaservo_client_consumer_t mediaservo_client_consumer_t;

/* ── 视频帧描述（I420；data 仅在回调内有效）── */
typedef struct mediaservo_client_frame_t {
    uint32_t width;
    uint32_t height;
    int64_t  ts_us;         /* 上游无时钟源时恒 0（按到达序消费） */
    const uint8_t* data;    /* I420 平面连续（len 字节）；仅回调内有效 */
    size_t   len;
} mediaservo_client_frame_t;

/* ── 回调（各自内部泵线程触发；串/帧仅回调内有效）── */
typedef void (*mediaservo_client_frame_cb)(mediaservo_client_session_t* s, const mediaservo_client_frame_t* frame, void* user);
typedef void (*mediaservo_client_ack_cb)(mediaservo_client_control_t* c, const char* ack_json, void* user);
typedef void (*mediaservo_client_state_cb)(mediaservo_client_session_t* s, uint8_t state, void* user);
/* user 指针释放器：NULL = 不释放；每条注册恰好调用一次（时机见文件头 user_free 契约）。 */
typedef void (*mediaservo_client_user_free)(void* user);

/* ── API ── */

/* 账号登录（POST {http_base}/api/auth/login，阻塞；批1b 扁平签名升形）。
 * 成功 out_jwt = NUL 结尾 JWT；needed 溢出合同见文件头（jwt 建议 cap ≥ 4096）。 */
mediaservo_err_t mediaservo_client_login(const char* http_base, const char* username,
                                         const char* password, char* out_jwt, size_t cap,
                                         size_t* needed);

/* 房间发现（GET {http_base}/api/rooms，阻塞；会话前自由函数——不依赖任何 handle）。
 * out_json = JSON 数组 `[{"room_id":..,"kind":"video"|"audio"},..]`（server wire 透传，
 * 本层不解析）。needed 溢出合同见文件头。
 * token 失效/角色不符（非 2xx）→ ERR_UNAUTHORIZED，详情 mediaservo_client_last_error。 */
mediaservo_err_t mediaservo_client_list_rooms(const char* http_base, const char* jwt, char* out_json, size_t cap, size_t* needed);

/* 错误码 → 静态文案（表源 = ClientError Display 模板；未知码 "unknown error code"）。
 * 拷贝语义 = 截断不报错（与 last_error 同形）。 */
mediaservo_err_t mediaservo_client_strerror(int code, char* buf, size_t cap);

/* 信令连接 + 入房（阻塞）。jwt/psk 恰一非空；成功后 *out 指向新 handle（调用方 close）。 */
mediaservo_err_t mediaservo_client_session_create(const mediaservo_client_config_t* cfg, mediaservo_client_session_t** out);

/* 谈成的方言版本（S0；旧 server = 1）。 */
mediaservo_err_t mediaservo_client_session_negotiated(const mediaservo_client_session_t* s, uint32_t* out_protocol);

/* 会话连接态快照（out_state = MEDIASERVO_CLIENT_STATE_*）。会话已关 → ERR_STATE。 */
mediaservo_err_t mediaservo_client_session_state(const mediaservo_client_session_t* s, uint8_t* out_state);

/* 注册会话连接态回调（累积形：多次注册 = 每次变化逐个回调全部注册项；不可注销，
 * 随 session_close 统一释放并逐条 user_free）。泵在首次注册且会话存活时启动；
 * 只报变化——注册后的初始态用 session_state 自取。cb=NULL → ERR_INVALID_ARG。 */
mediaservo_err_t mediaservo_client_session_on_state(mediaservo_client_session_t* s,
                                                    mediaservo_client_state_cb cb, void* user,
                                                    mediaservo_client_user_free user_free);

/* 纯等待房间内视频 producer（无副作用；多路编排定向入口）。成功 out_pid = producer
 * id；超时 → ERR_TIMEOUT；断链 → ERR_STATE。needed 溢出合同见文件头。 */
mediaservo_err_t mediaservo_client_session_wait_video(mediaservo_client_session_t* s,
                                                      uint64_t timeout_ms, char* out_pid,
                                                      size_t cap, size_t* needed);

/* 订阅一路视频 producer（多路新形，K4）：每路独立帧泵 + consumer 句柄，互不连坐。
 * producer_id 来自 wait_video 或 list/观测面。成功后 **out_consumer = 新 handle
 * （调用方 consumer_close；session_close 不隐式关）。user_free 在 consumer_close
 * 恰好一次。cb=NULL → ERR_INVALID_ARG（本形无取消语义）。 */
mediaservo_err_t mediaservo_client_session_consume(mediaservo_client_session_t* s,
                                                   const char* producer_id,
                                                   mediaservo_client_frame_cb cb, void* user,
                                                   mediaservo_client_user_free user_free,
                                                   mediaservo_client_consumer_t** out_consumer);

/* ⊘ 消费房间第一路视频（首路形，保留一周期；新代码用 wait_video+consume）：
 * 内部 = 等 producer(≤30s) → consume → into_receiver，行为与旧形逐字节一致。
 * 首调用启动帧泵线程；重复调用 → ERR_STATE（一次性闸）。 */
mediaservo_err_t mediaservo_client_session_consume_video(mediaservo_client_session_t* s, mediaservo_client_frame_cb cb, void* user);

/* ⊘ 视频统计汇总 JSON（会话级 union，保留一周期；多路精确读数用 consumer_stats）：
 * {"bytes_received","packets_received","packets_lost","frames_decoded",
 *  "frame_width","frame_height","frames_per_second"}，本会话 inbound-rtp 折叠，
 * 无消费者=全零。needed 溢出合同见文件头。 */
mediaservo_err_t mediaservo_client_session_video_stats(const mediaservo_client_session_t* s, char* out_json, size_t cap, size_t* needed);

/* K3 句柄级机读错误（本句柄最近一次失败调用；无错 = code 0）。覆盖状态/传输/映射
 * 类失败；缓冲溢出（needed 合同）只走返回码+全局 last_error。out.struct_size 必填。 */
mediaservo_err_t mediaservo_client_session_error(const mediaservo_client_session_t* s, mediaservo_client_error_t* out);

/* K3 句柄级错误文本（本句柄最近一次失败详情；无错 = 空串）。截断不报错。 */
mediaservo_err_t mediaservo_client_session_last_error(const mediaservo_client_session_t* s, char* buf, size_t len);

/* 急停双路投递（W4b）：DC 快路径（hmac_key_file 配置时带 HMAC 签名）+ 信令审计副本。
 * 前置：mediaservo_client_open_control 已成功。payload_json NULL/空 = null。
 * OK = 投递成功（非"已执行"——车端裁决看 seq 回执 ack）。 */
mediaservo_err_t mediaservo_client_session_emergency_stop(mediaservo_client_session_t* s, mediaservo_client_control_t* c,
                                                          const char* label, uint64_t seq,
                                                          const char* payload_json);

/* 开出程控制通道集（每会话一次性——ack 泵每会话一条；二次调用 → ERR_STATE）。
 * labels 为通道名数组（如 "chassis"/"gimbal"）。方言 <2 本地预拒（ERR_PROTOCOL）。 */
mediaservo_err_t mediaservo_client_open_control(mediaservo_client_session_t* s, const char* const* labels, size_t labels_len, mediaservo_client_control_t** out);

/* 发送一条命令信封 {seq, cmd, payload}（serde 构造，payload_json NULL/"" = null；
 * 非法 JSON → ERR_INVALID_ARG）。seq 由调用方自增维护（D-H3 重发幂等安全）。 */
mediaservo_err_t mediaservo_client_control_send(mediaservo_client_control_t* c, const char* label, uint64_t seq, const char* cmd, const char* payload_json);

/* 注册 ack 回调（累积形，批1b 签名升形：多次注册 = 每条 ack 逐个回调全部注册项；
 * 首次注册启动泵）。out_token 回传注销凭据（可 NULL；0=无效值）。cb=NULL → 拒
 * （注销走 control_off_ack）。user_free 在 off_ack 后的泵回收轮（≤1s）或
 * control_close 恰好一次。ack_json = ControlAck 序列化（{"ack":..,"result":..}）。 */
mediaservo_err_t mediaservo_client_control_on_ack(mediaservo_client_control_t* c, mediaservo_client_ack_cb cb,
                                                  void* user, mediaservo_client_user_free user_free,
                                                  uint64_t* out_token);

/* 按 token 注销 ack 回调（标记 retired，泵下轮回收并调用其 user_free；对下一轮之后
 * 的 ack 不再生效——在途轮次可能仍触发一次，有界宽限）。未知 token → ERR_INVALID_ARG。 */
mediaservo_err_t mediaservo_client_control_off_ack(mediaservo_client_control_t* c, uint64_t token);

/* DC 就绪态（out_state = MEDIASERVO_CLIENT_DC_*；发送/急停前门禁用）。 */
mediaservo_err_t mediaservo_client_control_ready_state(const mediaservo_client_control_t* c, uint8_t* out_state);

/* SCTP 背压水位（字节数；0 = 可安全追加。与 ack 泵争锁，单线程属主契约内可接受）。 */
mediaservo_err_t mediaservo_client_control_buffered_amount(mediaservo_client_control_t* c, uint64_t* out_bytes);

/* server 分配的 data producer id 的 JSON 数组（观测面）。needed 溢出合同见文件头
 * （批1b 升形：旧固定 buf 盲点修正）。 */
mediaservo_err_t mediaservo_client_control_producer_ids(const mediaservo_client_control_t* c, char* out_json, size_t cap, size_t* needed);

/* 关闭控制通道并释放 handle（幂等；join ack 泵后全量回收注册表逐条 user_free）。 */
mediaservo_err_t mediaservo_client_control_close(mediaservo_client_control_t* c);

/* ── 单路消费者（session_consume 产出；K4 多路互不连坐）── */

/* 本路 producer id（needed 溢出合同；id 短，cap 256 足够）。 */
mediaservo_err_t mediaservo_client_consumer_id(const mediaservo_client_consumer_t* c, char* out, size_t cap, size_t* needed);

/* 本路视频统计 JSON（单路读数，键表 = session_video_stats 同形）。needed 合同同上。 */
mediaservo_err_t mediaservo_client_consumer_stats(mediaservo_client_consumer_t* c, char* out_json, size_t cap, size_t* needed);

/* 关闭本路并释放 handle（幂等；撤接收端 → join 泵（≤250ms）→ user_free 恰好一次）。 */
mediaservo_err_t mediaservo_client_consumer_close(mediaservo_client_consumer_t* c);

/* 关闭会话并释放 handle（幂等；join 泵 + 状态回调逐条 user_free 后才释放内存；
 * 不隐式关 consumer/control）。 */
mediaservo_err_t mediaservo_client_session_close(mediaservo_client_session_t* s);

/* ── 通用 ── */

/* ⊘ 最近一次错误详情（进程全局，保留一周期——K3 双写兜底；新代码用句柄级
 * session_last_error + session_error）。线程安全。 */
mediaservo_err_t mediaservo_client_last_error(char* buf, size_t len);

/* SDK 版本 (MAJOR.MINOR.PATCH)。 */
mediaservo_err_t mediaservo_client_version(char* buf, size_t len);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* MEDIASERVO_CLIENT_H */
