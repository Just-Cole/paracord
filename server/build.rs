// The site is embedded from ../web/dist. Make sure the folder exists so the
// server still compiles before the site is built; it then shows a "not built" page.
fn main() {
    let dist = std::path::Path::new("../web/dist");
    if !dist.exists() {
        std::fs::create_dir_all(dist).expect("create web/dist");
    }
    println!("cargo:rerun-if-changed=../web/dist");
}
