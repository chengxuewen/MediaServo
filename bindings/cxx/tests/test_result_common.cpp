// mediaservo shared Result contract tests (C++11 baseline, native tl::expected API).
// Prerequisites: bindings/cxx/include (shared headers) + the three SDK headers + C headers (see test-cxx.sh for the -I list).
// Covers: value-semantics static_asserts / alias wiring / cross-SDK type identity / runtime paths / misuse contract.
// This file is the regression anchor for the future std::expected swap (result.hpp's two-line alias is the whole migration).

#include <cassert>
#include <memory>
#include <string>
#include <type_traits>
#include <utility>

#include <mediaservo/field.hpp>
#include <mediaservo/link.hpp>
#include <mediaservo/deck.hpp>

// -- static_assert: value semantics -----------------------------------------
static_assert(std::is_copy_constructible<mediaservo::Result<int> >::value, "Result<int> copy-constructible");
static_assert(std::is_copy_assignable<mediaservo::Result<int> >::value, "Result<int> copy-assignable");
static_assert(std::is_nothrow_move_constructible<mediaservo::Result<int> >::value, "Result<int> nothrow-move");
static_assert(std::is_move_constructible<mediaservo::Result<std::unique_ptr<int> > >::value, "move-only T: move ok");
static_assert(!std::is_copy_constructible<mediaservo::Result<std::unique_ptr<int> > >::value, "move-only T: no copy");

// -- static_assert: alias wiring (Result = tl::expected<T, Error>) --
static_assert(std::is_same<mediaservo::Result<int>,
                           tl::expected<int, mediaservo::Error> >::value, "alias -> tl::expected");
static_assert(std::is_same<mediaservo::Result<void>,
                           tl::expected<void, mediaservo::Error> >::value, "alias void -> tl::expected");

// -- static_assert: cross-SDK type identity (using wiring + multiple SDKs in one TU) --
static_assert(std::is_same<mediaservo::field::Error, mediaservo::link::Error>::value, "field/link Error same");
static_assert(std::is_same<mediaservo::field::Error, mediaservo::deck::Error>::value, "field/deck Error same");
static_assert(std::is_same<mediaservo::field::Result<int>, mediaservo::link::Result<int> >::value, "field/link Result same");
static_assert(std::is_same<mediaservo::field::Result<void>, mediaservo::deck::Result<void> >::value, "field/deck Result<void> same");

// -- Runtime: success/failure/access -----------------------------------------
int main() {
    // success path
    mediaservo::Result<int> a(42);
    assert(a.has_value());
    assert(a.value() == 42);
    assert(a.value_or(-1) == 42); // value_or takes the value on success

    // failure path
    mediaservo::Result<int> b(tl::unexpect, mediaservo::Error{-1, "boom"});
    assert(!b.has_value());
    assert(b.error().code == -1);
    assert(b.error().message == "boom");
    assert(b.value_or(-1) == -1); // value_or takes the default on failure

    // move-preserves-semantics
    mediaservo::Result<int> c(std::move(b));
    assert(!c.has_value());
    assert(c.error().message == "boom");

    // Result<void> both paths
    mediaservo::Result<void> v; // default-ctor = success (proven)
    assert(v.has_value());
    mediaservo::Result<void> ve(tl::unexpect, mediaservo::Error{-2, "void err"});
    assert(!ve.has_value());
    assert(ve.error().message == "void err");

    bool threw = false; // misuse contract (error() on success is UB by the standard's semantics; not asserted)
    try {
        (void)b.value(); // calling value() on a failure state -> throw
    } catch (const tl::bad_expected_access<mediaservo::Error>&) {
        threw = true;
    }
    assert(threw);

    // catch(std::exception) compatibility (contract-change declaration: the old logic_error was also a std::exception subclass)
    threw = false;
    try {
        (void)b.value();
    } catch (const std::exception& e) {
        threw = true;
        (void)e;
    }
    assert(threw);

    return 0;
}
