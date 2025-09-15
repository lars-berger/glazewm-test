use cfg_aliases::cfg_aliases;

fn main() {
  // TODO: Rust-analyzer keeps turning this feature on even though it's not
  // a default feature. TODO: Disable win and mac when the detached
  // feature is enabled, after detached is fixed.
  //
  // Also can't use `windows` as that is a buildin cfg, shorter version
  // should be clear enough.
  cfg_aliases! {
    win: { all(target_os = "windows", feature = "win") },
    mac: { all(target_os = "macos", feature = "mac") },
  }

  #[cfg(all(
    target_os = "macos",
    feature = "mac",
    not(feature = "detached")
  ))]
  println!(
    "cargo:rustc-link-search=framework=/System/Library/PrivateFrameworks"
  );
}
