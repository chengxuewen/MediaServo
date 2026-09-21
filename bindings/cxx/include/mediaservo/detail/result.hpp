/* MediaServo 共享 C++ Result 类型 — 三 SDK 头文件的公共基座（D248 单一事实源）。
 *
 * mediaservo::Result<T> 为 tl::expected<T, Error> 的 alias（完全迁移，2026-08-18，
 * C++11 起步）——原生 API: has_value()/value()/error()/value_or()；无 operator bool；
 * 误用 value()/error() 抛 tl::bad_expected_access<Error>（std::exception 子类，
 * what() 为通用文案，细节经异常 .error().code/.message 获取）。
 *
 * 契约变更声明（source-breaking）: 2026-08-18 前误用抛 std::logic_error（手写
 * variant 实现）；D241 仅锁 C ABI，C++ header API 无 ABI 承诺。catch(std::exception)
 * 兼容新旧两代。
 *
 * 未来 std::expected swap 路径: 编译器升 C++23 后，本文件两行 alias 改为
 * `template<typename T> using Result = std::expected<T, Error>;`（API 同构），
 * 契约测试（bindings/cxx/tests/test_result_common.cpp）为回归锚。
 */
#ifndef MEDIASERVO_RESULT_HPP
#define MEDIASERVO_RESULT_HPP

#include <mediaservo/3rdparty/tl/expected.hpp>

#include <cstdint>
#include <string>

namespace mediaservo {

/// 错误详情（code 为对应 SDK C 头 MEDIASERVO_<SDK>_ERR_* 值；message 读自 last_error）。
///
/// S6 批1b 追加机读位（client 家族填充；link/deck/field 恒 0/false）：
/// wire_code = server 线码回读（0=本地/无码域）；retryable = 可重试族
/// （D273 分类的 C++ 镜像）。构造器形保持全部既有 `Error{code, msg}` 两参
/// brace-init 源兼容且不触发 -Wmissing-field-initializers（C++11 起步编译面）；
/// header-only 无 ABI 承诺（D241 仅锁 C ABI）。
struct Error {
    int code;
    std::string message;
    uint16_t wire_code;
    bool retryable;

    Error() : code(0), message(), wire_code(0), retryable(false) {}
    Error(int c, std::string m) : code(c), message(std::move(m)), wire_code(0), retryable(false) {}
    Error(int c, std::string m, uint16_t w, bool r)
        : code(c), message(std::move(m)), wire_code(w), retryable(r) {}
};

/// 原生 tl::expected API（has_value()/value()/error()/value_or()，无 operator bool）。
/// Result<void> 即 tl::expected<void, Error>（alias 对 T=void 直接实例化，无需特化）。
template <typename T>
using Result = tl::expected<T, Error>;

} // namespace mediaservo

#endif /* MEDIASERVO_RESULT_HPP */
