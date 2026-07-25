use std::fs;

fn main() {
    for (name, path) in [
        ("INDEX", "web/index.html"),
        ("APP", "web/app.js"),
        ("DASHBOARD", "web/dashboard.js"),
        ("OPS", "web/ops.js"),
        ("USAGE", "web/usage.js"),
        ("USERS", "web/users.js"),
        ("BATCH_IMAGES", "web/batch-images.js"),
        ("CONTENT", "web/content.js"),
        ("ENGAGEMENT", "web/engagement.js"),
        ("ACCOUNTS_TOOLS", "web/accounts-tools.js"),
        ("ACCOUNT_SCHEDULES", "web/account-schedules.js"),
        ("SUBSCRIPTIONS", "web/subscriptions.js"),
        ("CHANNELS", "web/channels.js"),
        ("MONITOR_ADMIN", "web/monitor-admin.js"),
        ("TURNSTILE", "web/turnstile.js"),
        ("STYLES", "web/styles.css"),
        ("LOGO", "web/logo.svg"),
        ("SETUP", "web/setup.js"),
    ] {
        let bytes = fs::read(path).unwrap_or_else(|error| panic!("cannot read {path}: {error}"));
        println!("cargo:rerun-if-changed={path}");
        println!(
            "cargo:rustc-env=SUB2API_MINI_{name}_ETAG={:016x}",
            fnv1a(&bytes)
        );
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
