# 绑定矩阵使用指引（link / deck / field × C / C++ / Python）

> **定位**: MediaServo 三 SDK（link 设备侧 IPC、deck 媒体数据面、field 组合推流面）的
> 多语言消费层。**C ABI 为契约基座**，C++/Python 为其上的薄包装（D227）。
>
> **关联**: [20-sdk-api-contract.md](../modules/20-sdk-api-contract.md) §7、[22-field-guide.md](../modules/22-field-guide.md)、
> D227（绑定族 c/cxx/py）、D240（单动态库）、D241（soname + ABI 稳定）、D247（mediaservo_ 前缀）、D248（头文件策略）

## 布局

```
bindings/
├── c/
│   ├── include/mediaservo/common.h        # 共享 C 类型（err_t/frame_meta_t 36B/frame_t）
│   ├── mediaservo-field-c/                # cdylib → libmediaservo_field.so
│   │   └── include/mediaservo/field.h     #   头文件（手工维护，D248）
│   ├── mediaservo-link-c/                 # → libmediaservo_link.so（signal + bus）
│   │   └── include/mediaservo/link.h
│   └── mediaservo-deck-c/                 # → libmediaservo_deck.so（camera/recorder/player）
│       └── include/mediaservo/deck.h
├── cxx/
│   ├── include/mediaservo/detail/result.hpp     # 共享 Result（alias tl::expected, C++11 起步）
│   ├── include/mediaservo/3rdparty/tl/expected.hpp + NOTICE  # vendored（CC0, 不可修改）
│   └── mediaservo-{field,link,deck}-cxx/  # header-only RAII（namespace mediaservo::{field,link,deck}）
└── python/mediaservo/                     # ctypes 包（非 cargo member，D228）
    └── mediaservo/{_ffi,field,link,deck}.py
```

## 构建与测试（pixi tasks）

```bash
pixi run build-c          # 三 cdylib + dev .so.<MAJOR> symlink（D241 DT_NEEDED）
pixi run test-cxx         # C++ 三 SDK 测试（g++ 编译 + 断言）
pixi run test-py          # Python unittest（需 MEDIASERVO_LIB_DIR 或 env 继承）
pixi run parity-bindings  # 跨语言一致性（version/错误路径三端断言）
pixi run abi-drift        # ABI 漂移门禁（header 声明 ↔ .so 导出对照，D248）
```

## 各语言集成

### C（契约基座）

```c
#include <mediaservo/field.h>          /* 或 link.h / deck.h */

mediaservo_push_config_t cfg = MEDIASERVO_PUSH_CONFIG_DEFAULT;  /* struct_size 自动 */
cfg.url = "ws://host:9800/ws"; cfg.psk = "..."; cfg.room = "room";
mediaservo_field_push_t* s = NULL;
if (mediaservo_field_push_connect(&cfg, &s) != MEDIASERVO_OK) {
    char err[256]; mediaservo_field_last_error(err, sizeof(err));
}
```

编译: `gcc app.c -I bindings/c/mediaservo-field-c/include -I bindings/c/include \
       -L target/debug -lmediaservo_field`（运行 `LD_LIBRARY_PATH=target/debug`）

### C++（header-only RAII）

```cpp
#include <mediaservo/field.hpp>        /* 或 link.hpp / deck.hpp */
using mediaservo::field::PushConfig;
using mediaservo::field::PushSession;

auto s = PushSession::connect(PushConfig{"ws://host:9800/ws", "psk", "room"});
if (!s) { /* s.error().code / s.error().message */ }
s.value().start_video_frames();        /* RAII: 析构自动 close */
```

编译: `g++ -std=c++11 app.cpp -I bindings/cxx/mediaservo-field-cxx/include -I bindings/cxx/include \
       -I bindings/c/mediaservo-field-c/include -I bindings/c/include \
       -L target/debug -lmediaservo_field`

### Python（ctypes）

```python
from mediaservo.field import PushConfig, PushSession
s = PushSession.connect(PushConfig(url="ws://host:9800/ws", psk="...", room="room"))
s.publish_video()
```

运行: `export MEDIASERVO_LIB_DIR=$PWD/target/debug && python3 app.py`
（或 `LD_LIBRARY_PATH=target/debug`；三 SDK 子模块 `mediaservo.{field,link,deck}`）

## 契约要点（跨语言一致）

| 项 | 规则 |
|---|---|
| 前缀 | 符号/类型/宏 `mediaservo_*`（D247）；C++ 命名空间 `mediaservo::<sdk>`；Python 模块同名 |
| soname | `libmediaservo_<sdk>.so.<MAJOR>`（D241，MAJOR = C ABI 版本）|
| struct_size | 所有跨 FFI 配置结构体首字段 `size_t struct_size`（R3，调用方填 sizeof）|
| 生命周期 | handle 单线程属主；close 后任何 API 调用为 UB；close 幂等 |
| 回调 | 仅在泵线程触发；回调内禁止调 close；指针/事件字符串仅回调内有效（需保留请拷贝）|
| 错误 | `mediaservo_<sdk>_last_error(buf, len)` 线程安全；0 = ok，<0 = 错误码 |
| 版本 | `mediaservo_<sdk>_version(buf, len)` → MAJOR.MINOR.PATCH |
| 兼容 | within MAJOR 只加法（D241）；头文件改动 = ABI 变更，须过 `abi-drift` |

## 已验证能力（2026-08-18）

| 语言 | 验证 |
|---|---|
| C | field/link/deck 三端 live e2e（真实 server 收帧 / 事件泵 / 91 帧闭环录制）|
| C++ | 三 SDK 测试（version/错误路径/move 语义/Result 误用）+ parity |
| Python | 22 tests + field push live e2e（connected/published/frames）|
| 全矩阵 | `parity-bindings` 三端一致 + `abi-drift` 声明=导出（7/12/15）|

## 已知限制

- **PullSession 收帧挂起**（见 22-field-guide 已知限制）：消费方是 client，未在绑定层暴露
- **deck Player**：C 面仅 open/frames/close（seek/set_rate YAGNI 押后，N2 标注）
- **Python**：ctypes 首版（D227 两步走）；pyo3 加速后端待触发条件（帧路径 >10% 预算等）
- **Windows/macOS**：soname 仅 Linux（R8 门控）；macOS 用默认 dylib 命名；Python `_lib_filename` 已按平台分派

## 附录 A — SDD 追溯矩阵（函数 × 测试覆盖，2026-08-18）

> 单测 = cargo test（c crate 内）；e2e = scripts/e2e-bindings.sh（真实 server/FFmpeg）；parity = parity-bindings。
> ⚠️ = 覆盖缺口（见下）。错误路径/状态机未逐函数列出（c crate 内统一模式）。

### field（7 函数）
| 函数 | 单测 | 正向 e2e |
|---|---|---|
| push_connect | ✓（null/small-size/missing-required）| ✓ vehicle_push |
| push_publish_video | ✓（null handle）| ✓ vehicle_push（track=video）|
| push_start_video_frames | ✓（null 失败）| ✓ vehicle_push（frames running）|
| push_stop_video_frames | ✓（null noop）| ✓ vehicle_push（close 前调用）|
| push_close | ✓（null/幂等）| ✓ vehicle_push |
| last_error（+deprecated 别名）| ✓ roundtrip ×2 | ✓ parity |
| version | ✓ roundtrip | ✓ parity（三端一致）|

### link（12 函数）
| 函数 | 单测 | 正向 e2e |
|---|---|---|
| signal_connect | ✓（null/small-size/missing-required/bad-role）| ✓ vehicle_signal |
| signal_send | ✓（null/empty/未连接/closed）| ✓ vehicle_signal（encoder_status 回显）|
| signal_on_event | ✓（null noop）| ✓ vehicle_signal（event 泵）|
| signal_close | ✓（null）| ✓ vehicle_signal |
| bus_attach | ✓（null）+ 正向（双节点验签）| ⚠️ e2e 无（单测覆盖 SHM 往返）|
| bus_publish | ✓（null/未连接/closed）+ 正向 | ⚠️ e2e 无（单测覆盖）|
| bus_subscribe | ✓（null）+ 正向 | ⚠️ e2e 无（单测覆盖）|
| bus_recv | ✓（null）+ 正向（meta/payload 断言）| ⚠️ e2e 无（单测覆盖）|
| stream_close | ✓（null）| — |
| bus_close | ✓（null）| — |
| last_error | ✓ roundtrip | ✓ parity |
| version | ✓ roundtrip | ✓ parity |

### deck（15 函数）
| 函数 | 单测 | 正向 e2e |
|---|---|---|
| devices_enumerate | ✓（双调用/空/非法 kind）| ✓ record_playback |
| camera_open | ✓（null/small-size/未知设备/roundtrip）| ✓ record_playback |
| camera_start | ✓（double-start）| ✓ record_playback（帧泵）|
| camera_frames_cb | ✓（null）| ✓ record_playback（90+ 帧回调）|
| camera_stop | ✓（close roundtrip 内）| ✓ record_playback |
| camera_close | ✓ | ✓ record_playback |
| recorder_new | ✓（null/父目录缺失）| ✓ record_playback |
| recorder_record | ✓（null/未 start 相机）| ✓ record_playback（mp4 产物）|
| recorder_stop | ✓（null 失败）| ✓ record_playback |
| recorder_close | ✓（null）| ✓ record_playback |
| player_open | ✓（null/文件缺失）| ✓ record_playback |
| player_frames_cb | ✓（null 失败）| ✓ record_playback（91 帧解码）|
| player_close | ✓（null）| ✓ record_playback |
| last_error | ✓ roundtrip | ✓ parity |
| version | ✓ roundtrip | ✓ parity |

### 跨语言
- parity-bindings: version + 空配置 connect 错误路径（C/C++/Python 断言一致）✓
- abi-drift: 34 声明 == 34 导出 ✓

### 已补齐（2026-08-18，二次收尾）
1. ✅ link bus 正向链路单测（双节点 token/ACL 验签 → subscribe → publish → recv 断言 meta/payload，真实 SHM 往返）
2. ✅ field start/stop、link stream_close/bus_close、deck recorder_stop/player_frames_cb null 单测
3. ✅ C++ live e2e（e2e-bindings 第 6 步，vehicle_field.cpp 连真实 server）

### 剩余缺口（可接受）
- link bus 的 e2e 级覆盖依赖单测（无真实跨进程 server 场景）；cxx/py 其余 SDK 正向依赖 C 层已验证链路

## CLI 集成（mediaservo.sh，统一入口）

```bash
./mediaservo.sh build bindings                # 构建三 cdylib + dev .so.<MAJOR> symlink
./mediaservo.sh deploy bindings --prefix /usr/local
  # lib/    libmediaservo_{field,link,deck}.so.0.1.0 + .so.0 + .so（D241 三件套）
  # include/mediaservo/  {common,field,link,deck}.h + {field,link,deck}.hpp（D248）
# 消费方式（三选一）:
#   pkg-config:  PKG_CONFIG_PATH=<prefix>/lib/pkgconfig cc app.c $(pkg-config --cflags --libs mediaservo-field)
#   CMake:       find_package(mediaservo COMPONENTS field REQUIRED) → target_link_libraries(app mediaservo::field)
#                （单包多组件，OpenCV/Boost 模式；仅需 link: COMPONENTS link → mediaservo::link；-DCMAKE_PREFIX_PATH=<prefix>）
#   gcc 直链:    gcc app.c -I <prefix>/include -L <prefix>/lib -lmediaservo_field
# Python: pip install bindings/python/mediaservo（薄包；运行时 .so 定位: LD_LIBRARY_PATH 或 ldconfig）
```

## Node.js 绑定（napi-rs，livekit rtc-ffi-bindings 同构，2026-08-18）

- **架构**: `bindings/node/rust/mediaservo-node`（napi 3 cdylib → .node）直绑 Rust SDK async API；TS 薄包装 `lib/index.mjs`（PushSession/SignalSession/CameraSource/Recorder/Player）；事件/帧经 ThreadsafeFunction → JS 主线程（livekit async_queue 同构）
- **构建/安装**: `./mediaservo.sh build bindings`（含 mediaservo.node）+ `deploy bindings` → `<prefix>/node/mediaservo/`
- **使用**: `import { PushSession } from '<prefix>/node/mediaservo/lib/index.mjs'`（或 NODE_PATH）
- **运行前置**: LD_LIBRARY_PATH 含 pixi lib（FFmpeg 动态库 libavformat.so.63 等——conda 工具链）；测试 `npm test`（bindings/node/）
- **已验证**: 真 server 推流（connected/published/frames）+ 信令事件桥 + 录制回放闭环（node:test 5/5）
- **契约**: napi 方法 camelCase（publishVideo）；元组参数 → JS 数组（[meta, data]）；`Function<T,()>` + `build_threadsafe_function::<T>().build()`

## C++11 承诺与 Result 契约（2026-08-18）

- **C++11 起步可用**: 三 SDK header-only 绑定 + 共享 Result 全部 `-std=c++11` 编译零警告（test-cxx 全量门禁）
- **Result 定义**: `mediaservo::Result<T> = tl::expected<T, mediaservo::Error>`（alias，`detail/result.hpp`）
  - 原生 API: `has_value()/value()/error()/value_or()`；**无 operator bool**；`Result<void>` 即 `tl::expected<void, Error>`
  - **契约变更（source-breaking, 2026-08-18）**: 误用 `value()` 抛 `tl::bad_expected_access<Error>`（std::exception 子类，what() 通用文案，细节经异常 `.error().code/.message`）；旧 `std::logic_error` 断言需改 catch 类型（`catch(std::exception)` 兼容新旧）；`error()` on success 为标准 UB（旧版抛异常）
  - 绑定刚交付零真实消费者，变更无外部破坏记录
- **CC0 归属**: `tl/expected.hpp` vendored from TartanLlama/expected v1.2.0（CC0-1.0），见 `3rdparty/NOTICE`（不可修改，升级走重新 vendor）
- **未来 std::expected swap 路径**: 编译器升 C++23 后 `detail/result.hpp` 两行 alias 改为 `std::expected<T, Error>`（API 同构），契约测试 `bindings/cxx/tests/test_result_common.cpp` 为回归锚
