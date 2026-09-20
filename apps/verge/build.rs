fn main() {
    println!("cargo:rerun-if-env-changed=VERGE_BUILD_CHANNEL");
    let channel = std::env::var("VERGE_BUILD_CHANNEL").unwrap_or_else(|_| "stable".into());
    assert!(
        matches!(channel.as_str(), "stable" | "dev"),
        "VERGE_BUILD_CHANNEL must be stable or dev"
    );
    println!("cargo:rustc-env=VERGE_BUILD_CHANNEL={channel}");
}
