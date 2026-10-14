/* MediaServo shared C++ Result type — the common foundation for the three SDK headers (the D248 single source of truth).
 *
 * mediaservo::Result<T> is an alias for tl::expected<T, Error> (fully migrated 2026-08-18,
 * C++11 baseline) — native API: has_value()/value()/error()/value_or(); no operator bool;
 * misusing value()/error() throws tl::bad_expected_access<Error> (a std::exception subclass;
 * what() is generic text; details come from .error().code/.message on the exception).
 *
 * Contract change declaration (source-breaking): before 2026-08-18 misuse threw std::logic_error (the hand-written
 * variant implementation); D241 pins only the C ABI, and the C++ header API carries no ABI promise. catch(std::exception)
 * handles both the old and new generations.
 *
 * Future std::expected swap path: once the compiler reaches C++23, change this file's two-line alias to
 * `template<typename T> using Result = std::expected<T, Error>;` (same API shape),
 * with the contract tests (bindings/cxx/tests/test_result_common.cpp) as the regression anchor.
 */
#ifndef MEDIASERVO_RESULT_HPP
#define MEDIASERVO_RESULT_HPP

#include <mediaservo/3rdparty/tl/expected.hpp>

#include <cstdint>
#include <string>

namespace mediaservo {

/// Error details (code is the corresponding SDK C header's MEDIASERVO_<SDK>_ERR_* value; message read from last_error).
///
/// Machine-readable bits added in S6 batch 1b (filled by the client family; always 0/false for link/deck/field):
/// wire_code = server wire-code readback (0 = local / no code domain); retryable = the retryable family
/// (the C++ mirror of the D273 classification). The constructor keeps every existing two-argument `Error{code, msg}`
/// brace-init source compatible and does not trigger -Wmissing-field-initializers (the C++11-baseline compile surface);
/// header-only, no ABI promise (D241 pins only the C ABI).
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

/// Native tl::expected API (has_value()/value()/error()/value_or(), no operator bool).
/// Result<void> is just tl::expected<void, Error> (the alias instantiates directly for T=void; no specialization needed).
template <typename T>
using Result = tl::expected<T, Error>;

} // namespace mediaservo

#endif /* MEDIASERVO_RESULT_HPP */
