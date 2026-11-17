fn main() {
    println!("cargo:rerun-if-changed=web/dist");
    assert!(
        std::path::Path::new("web/dist/index.html").exists(),
        "Build the browser assets first: cd web && npm ci && npm run build"
    );
}
