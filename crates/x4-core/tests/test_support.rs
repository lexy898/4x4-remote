//! The builders of the `test-support` feature, used the way a dependent crate uses them: from
//! outside, through the public module. This test does not compile without the feature.

use x4_core::test_support::{fixture_result, torrent};
use x4_rpc::testing::fixtures::SESSION_GET;

#[test]
fn the_builders_are_public_for_dependent_crates() {
    let built = torrent("Some name");

    assert_eq!(built.name, "Some name");
    assert_eq!(built.hash, torrent("Some name").hash);
}

#[test]
fn a_fixture_result_comes_without_its_envelope() {
    let result = fixture_result(SESSION_GET);

    assert!(result.get("rpc_version_semver").is_some(), "{result}");
    assert!(result.get("jsonrpc").is_none(), "{result}");
}
